use crate::{
    Claim, Configuration, Desired, Error, ErrorCode, Id, Impact, Mutation, ObservedWindow, Plan,
    Presentation, Result, Runtime, ShowState, Snapshot, new_id,
};
use std::collections::BTreeMap;

/// A native undo unit holds only its explicitly changed slots. Never persisted as authored state.
#[derive(Clone, Debug)]
pub struct UndoRecord {
    plan: Plan,
    prior_presentations: BTreeMap<Id, Presentation>,
    prior_claims: BTreeMap<Id, Claim>,
    prior_geometry: BTreeMap<Id, Desired>,
    after: BTreeMap<Id, ObservedWindow>,
    generations: BTreeMap<Id, u64>,
}

impl UndoRecord {
    #[must_use]
    pub fn capture(plan: &Plan, prior: &Runtime, settled: &Snapshot, current: &Runtime) -> Self {
        Self {
            prior_geometry: prior
                .geometry
                .iter()
                .filter(|(_, desired)| plan.scope.contains(&desired.slot))
                .map(|(id, desired)| (id.clone(), desired.clone()))
                .collect(),
            plan: plan.clone(),
            prior_presentations: prior
                .presentations
                .iter()
                .filter(|(slot, _)| plan.scope.contains(*slot))
                .map(|(id, value)| (id.clone(), value.clone()))
                .collect(),
            prior_claims: prior
                .claims
                .iter()
                .filter(|(_, claim)| plan.scope.contains(&claim.slot))
                .map(|(id, value)| (id.clone(), value.clone()))
                .collect(),
            after: plan
                .expected
                .keys()
                .filter(|id| {
                    plan.desired.contains_key(*id)
                        || plan.mutations.iter().any(|mutation| &mutation.window == *id)
                        || prior
                            .claims
                            .get(*id)
                            .is_some_and(|claim| plan.scope.contains(&claim.slot))
                })
                .filter_map(|id| {
                    settled.windows.get(id).map(|observed| (id.clone(), observed.clone()))
                })
                .collect(),
            generations: plan
                .scope
                .iter()
                .map(|id| (id.clone(), current.generations.get(id).copied().unwrap_or_default()))
                .collect(),
        }
    }

    /// Called only after our own successful undo. External events never rebase authority.
    pub fn rebase_after_undo(&mut self, runtime: &Runtime, snapshot: &Snapshot) {
        if self.after.iter().all(|(id, observed)| snapshot.windows.get(id) == Some(observed))
            && self
                .plan
                .scope
                .iter()
                .all(|slot| runtime.presentations.get(slot) == self.plan.presentations.get(slot))
        {
            for (slot, generation) in &mut self.generations {
                *generation = runtime.generations.get(slot).copied().unwrap_or_default();
            }
        }
    }

    #[allow(clippy::too_many_lines)] // One transaction derives a reversible diff and validates its authority.
    pub fn reverse(
        &self,
        config: &Configuration,
        runtime: &Runtime,
        snapshot: &Snapshot,
    ) -> Result<Plan> {
        let mut result = self.plan.clone();
        result.id = new_id("undo");
        result.mode = None;
        result.config_revision = config.revision;
        result.generations = self.generations.clone();
        result.expected = self.after.clone();
        result.presentations = self.prior_presentations.clone();
        result.desired.clear();
        result.mutations.clear();
        result.impact = Impact::default();
        result.diagnostics = vec!["Scoped undo: focus is not restored".into()];
        result.idempotent = false;
        // Revalidate lifetime, observed state, topology and newer ownership before deriving mutations.
        result.revalidate(config, runtime, snapshot)?;
        for (window, claim) in &self.prior_claims {
            let prior = self.plan.expected.get(window).ok_or_else(|| {
                Error::new(ErrorCode::StaleBinding, "Undo lacks prior observation", window)
            })?;
            let mut desired = self.prior_geometry.get(window).cloned().unwrap_or_else(|| Desired {
                window: window.clone(),
                placement: claim.placement.clone(),
                slot: claim.slot.clone(),
                frame: prior.frame,
                context: String::new(),
                allocated: prior.frame,
                strict_size: false,
                carried: true,
                dpi: prior.dpi,
                client_target: prior.client.map(Some),
                minimum_client_target: None,
            });
            desired.frame = prior.frame;
            desired.strict_size = false;
            desired.minimum_client_target = None;
            desired.dpi = prior.dpi;
            desired.client_target = prior.client.map(Some);
            result.desired.insert(window.clone(), desired);
        }
        for original in &self.plan.mutations {
            let window = &original.window;
            let prior = &self.plan.expected[window];
            let current = &self.after[window];
            let reference = config.windows.get(window).ok_or_else(|| {
                Error::new(ErrorCode::TargetMissing, "Undo reference was removed", window)
            })?;
            let geometry = original
                .geometry
                .and_then(|_| (prior.frame != current.frame).then_some(prior.frame));
            let visible = original
                .visible
                .and_then(|_| (prior.visible != current.visible).then_some(prior.visible));
            let move_only = prior.frame.width == current.frame.width
                && prior.frame.height == current.frame.height;
            let protection = crate::effective_protection(config, runtime, window, None, None);
            let show_state = original
                .show_state
                .and_then(|_| (prior.show_state != current.show_state).then_some(prior.show_state));
            if show_state.is_some()
                && (!reference.capabilities.allow_show_state
                    || protection.geometry_lock
                    || protection.maintain_visible && show_state == Some(ShowState::Minimized)
                    || protection.prohibit_focus && show_state == Some(ShowState::Maximized))
            {
                return Err(Error::new(
                    ErrorCode::UnsupportedOperation,
                    "Show-state undo is outside current profile/protection",
                    window,
                ));
            }
            if geometry.is_some()
                && (protection.geometry_lock
                    || !current.can_move
                    || (!move_only && !current.can_resize)
                    || current.show_state != ShowState::Normal)
                || visible == Some(false)
                    && (protection.maintain_visible
                        || !reference.allow_hide
                        || !current.can_hide
                        || current.has_owned_dialog)
                || geometry.is_some() && protection.keep_monitor && prior.display != current.display
            {
                return Err(Error::new(
                    ErrorCode::UnsatisfiableConstraints,
                    "Current protection/capability prevents undo",
                    window,
                ));
            }
            if let Some(frame) = geometry {
                for (other, observed) in &snapshot.windows {
                    if other != window
                        && observed.visible
                        && crate::effective_protection(config, runtime, other, None, None)
                            .maintain_visible
                        && frame.overlaps(observed.frame)
                    {
                        return Err(Error::new(
                            ErrorCode::UnsatisfiableConstraints,
                            "Undo would cover protected content",
                            other,
                        ));
                    }
                }
                if move_only {
                    result.impact.moved += 1;
                } else {
                    result.impact.resized += 1;
                }
            }
            if let Some(show) = visible {
                if show {
                    result.impact.shown += 1;
                } else {
                    result.impact.hidden += 1;
                }
            }
            if geometry.is_some() || visible.is_some() || show_state.is_some() {
                result.mutations.push(Mutation {
                    window: window.clone(),
                    binding: current.binding.clone(),
                    geometry,
                    move_only,
                    visible,
                    focus: false,
                    show_state,
                });
            }
        }
        result.revalidate(config, runtime, snapshot)?;
        Ok(result)
    }
}
