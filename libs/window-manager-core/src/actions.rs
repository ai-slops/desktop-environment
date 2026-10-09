use crate::{
    Configuration, Desired, Error, ErrorCode, Id, Impact, Mutation, OutputProtection, OutputState,
    Plan, Result, Runtime, ShowState, Snapshot, effective_protection, new_id, slot_bounds,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum WindowActionKind {
    ShowState(ShowState),
    Rescue,
    Reveal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowAction {
    pub id: Id,
    pub expected_revision: u64,
    pub window: Id,
    pub slot: Id,
    pub action: WindowActionKind,
}

/// Explicit user actions share the transition revalidation, journal, settlement and undo paths.
#[allow(clippy::too_many_lines)]
pub fn plan_window_action(
    config: &Configuration,
    runtime: &Runtime,
    snapshot: &Snapshot,
    action: &WindowAction,
) -> Result<Plan> {
    config.validate()?;
    let reference = config.windows.get(&action.window).ok_or_else(|| {
        Error::new(ErrorCode::TargetMissing, "Window reference missing", &action.window)
    })?;
    let slot = config.slots.get(&action.slot).ok_or_else(|| {
        Error::new(ErrorCode::TargetMissing, "Destination Slot missing", &action.slot)
    })?;
    let slot = crate::resolve_slot(config, slot, snapshot)?;
    let observed = snapshot.windows.get(&action.window).ok_or_else(|| {
        Error::new(ErrorCode::StaleBinding, "Window binding missing", &action.window)
    })?;
    if config.revision != action.expected_revision {
        return Err(Error::new(
            ErrorCode::StaleRevision,
            "Action configuration changed",
            &action.id,
        ));
    }
    if runtime.claims.get(&action.window).is_some_and(|claim| claim.slot != action.slot) {
        return Err(Error::new(
            ErrorCode::ClaimConflict,
            "Action cannot steal another slot's resource",
            &action.window,
        ));
    }
    let protection =
        effective_protection(config, runtime, &action.window, None, Some(&action.slot));
    let mut mutation = Mutation {
        window: action.window.clone(),
        binding: observed.binding.clone(),
        geometry: None,
        visible: None,
        move_only: true,
        focus: false,
        show_state: None,
    };
    let mut impact = Impact::default();
    match action.action {
        WindowActionKind::ShowState(state) => {
            if !reference.capabilities.allow_show_state
                || protection.geometry_lock
                || protection.maintain_visible && state == ShowState::Minimized
                || protection.prohibit_focus && state == ShowState::Maximized
                || observed.has_owned_dialog
            {
                return Err(Error::new(
                    ErrorCode::UnsupportedOperation,
                    "Explicit show-state action is outside profile/protection/modal constraints",
                    &action.window,
                ));
            }
            if state != observed.show_state {
                mutation.show_state = Some(state);
            }
        }
        WindowActionKind::Reveal => {
            if !observed.visible && observed.show_state != ShowState::Minimized {
                mutation.visible = Some(true);
                impact.shown = 1;
            }
        }
        WindowActionKind::Rescue => {
            let bounds = slot_bounds(&slot, snapshot)?;
            if protection.geometry_lock
                || !reference.capabilities.allow_move
                || !observed.can_move
                || protection.keep_monitor && slot.display != observed.display
                || snapshot.displays[&slot.display].dpi != observed.dpi
            {
                return Err(Error::new(
                    ErrorCode::UnsupportedOperation,
                    "Rescue requires movable normal state and compatible DPI/monitor constraints",
                    &action.window,
                ));
            }
            if observed.frame.width > bounds.width || observed.frame.height > bounds.height {
                return Err(Error::new(
                    ErrorCode::UnsatisfiableConstraints,
                    "Rescue cannot fit without shrinking; resize requires a separate action",
                    &action.window,
                ));
            }
            let frame = crate::Rect {
                x: observed.frame.x.clamp(bounds.x, bounds.x + bounds.width - observed.frame.width),
                y: observed
                    .frame
                    .y
                    .clamp(bounds.y, bounds.y + bounds.height - observed.frame.height),
                ..observed.frame
            };
            for (window, other) in &snapshot.windows {
                if window != &action.window
                    && other.visible
                    && effective_protection(config, runtime, window, None, None).maintain_visible
                    && frame.overlaps(other.frame)
                {
                    return Err(Error::new(
                        ErrorCode::UnsatisfiableConstraints,
                        "Rescue overlaps protected content",
                        window,
                    ));
                }
            }
            if frame != observed.frame {
                mutation.geometry = Some(frame);
                impact.moved = 1;
            }
        }
    }
    if reference.output_protection != OutputProtection::None
        && runtime.providers.output(&action.window, snapshot.now_ms) != OutputState::VerifiedPrivate
    {
        return Err(Error::new(
            ErrorCode::OutputStateUnknown,
            "Output protection prevents unverified explicit action",
            &action.window,
        ));
    }
    let mut result = Plan {
        id: if action.id.is_empty() { new_id("action") } else { action.id.clone() },
        config_revision: config.revision,
        topology_revision: snapshot.topology_revision,
        scope: BTreeSet::from([action.slot.clone()]),
        generations: BTreeMap::from([(
            action.slot.clone(),
            runtime.generations.get(&action.slot).copied().unwrap_or_default(),
        )]),
        expected: BTreeMap::from([(action.window.clone(), observed.clone())]),
        presentations: runtime
            .presentations
            .iter()
            .filter(|(id, _)| *id == &action.slot)
            .map(|(id, presentation)| (id.clone(), presentation.clone()))
            .collect(),
        desired: BTreeMap::new(),
        mutations: Vec::new(),
        impact,
        diagnostics: vec!["Explicit state/rescue action; saved geometry is unchanged".into()],
        idempotent: false,
        mutation_slots: BTreeMap::from([(action.window.clone(), action.slot.clone())]),
        domains: vec![BTreeSet::from([action.slot.clone()])],
        blocked: BTreeMap::new(),
    };
    for (window, claim) in &runtime.claims {
        if claim.slot == action.slot
            && let Some(current) = snapshot.windows.get(window)
        {
            result.expected.insert(window.clone(), current.clone());
            let mut desired = runtime.geometry.get(window).cloned().unwrap_or_else(|| Desired {
                window: window.clone(),
                placement: claim.placement.clone(),
                slot: claim.slot.clone(),
                frame: current.frame,
                context: String::new(),
                allocated: current.frame,
                strict_size: false,
                carried: true,
                dpi: current.dpi,
                client_target: current.client.map(Some),
                minimum_client_target: None,
            });
            desired.frame = if window == &action.window {
                mutation.geometry.unwrap_or(current.frame)
            } else {
                current.frame
            };
            desired.strict_size = window == &action.window && mutation.geometry.is_some();
            desired.dpi = current.dpi;
            desired.client_target = current.client.map(Some);
            result.desired.insert(window.clone(), desired);
        }
    }
    if mutation.geometry.is_some() || mutation.visible.is_some() || mutation.show_state.is_some() {
        result.mutations.push(mutation);
    } else {
        result.idempotent = true;
        result.impact.unchanged = 1;
    }
    result.revalidate(config, runtime, snapshot)?;
    if runtime.completed.contains(&action.id) {
        result.idempotent = true;
        result.mutations.clear();
        result.impact = Impact { unchanged: 1, ..Impact::default() };
    }
    Ok(result)
}
