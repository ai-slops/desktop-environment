use crate::{
    Configuration, Error, ErrorCode, Id, Node, OutputProtection, OutputState, Placement,
    ReflowPolicy, Request, Result, Runtime, Snapshot, Target, TransitionMode, effective_protection,
};
use std::collections::BTreeMap;

#[must_use]
pub fn has_continuous_rule(node: &Node) -> bool {
    if let Node::Group(group) = node {
        group.reflow == ReflowPolicy::ContinuousRule
            || group.children.iter().any(has_continuous_rule)
    } else {
        false
    }
}
#[must_use]
pub fn existing_position_members(node: &Node) -> Vec<&Placement> {
    match node {
        Node::Placement(_) => Vec::new(),
        Node::Group(group) => group
            .children
            .iter()
            .flat_map(|child| match child {
                Node::Placement(placement)
                    if group.reflow == ReflowPolicy::ExistingPositionFirst =>
                {
                    vec![placement]
                }
                Node::Group(_) => existing_position_members(child),
                Node::Placement(_) => Vec::new(),
            })
            .collect(),
    }
}
/// Ordinary window APIs cannot prove application-internal typing/IME state. A
/// foreground managed window conservatively defers all automatic geometry work.
pub fn continuous_permission(
    config: &Configuration,
    runtime: &Runtime,
    snapshot: &Snapshot,
    slot: &str,
    gesture_active: bool,
) -> Result<()> {
    let deferred = |message| Error::new(ErrorCode::PermissionDenied, message, slot);
    let active =
        runtime.presentations.get(slot).ok_or_else(|| deferred("No active presentation"))?;
    let root = config
        .views
        .get(&active.view)
        .and_then(|view| view.roots.get(&active.root))
        .ok_or_else(|| deferred("Active authored root disappeared"))?;
    if !has_continuous_rule(root) || runtime.paused || runtime.suspended.contains(slot) {
        return Err(deferred("Continuous rule is disabled or suspended"));
    }
    if gesture_active || snapshot.focused.is_some() {
        return Err(deferred(
            "Detected gesture or foreground managed interaction; reflow deferred",
        ));
    }
    if !active.approved_resize.is_empty()
        || active.expansion.is_some()
        || config.slots.get(slot).is_none_or(|slot| slot.designated_public)
    {
        return Err(deferred(
            "Expanded or public-designated presentation requires explicit reflow",
        ));
    }
    let mut leaves = Vec::new();
    root.placements(&mut leaves);
    for placement in leaves {
        let protection = effective_protection(
            config,
            runtime,
            &placement.window,
            Some(&placement.id),
            Some(slot),
        );
        if protection != crate::Protection::default()
            || active.overrides.contains_key(&placement.window)
            || snapshot.windows.get(&placement.window).is_some_and(|window| {
                window.has_owned_dialog || window.show_state != crate::ShowState::Normal
            })
        {
            return Err(deferred(
                "Protection, Visit exception or modal/show-state interaction defers reflow",
            ));
        }
        if config.windows[&placement.window].output_protection != OutputProtection::None
            && runtime.providers.output(&placement.window, snapshot.now_ms)
                != OutputState::VerifiedPrivate
        {
            return Err(Error::new(
                ErrorCode::OutputStateUnknown,
                "Fresh output evidence required before continuous reflow",
                &placement.window,
            ));
        }
    }
    Ok(())
}
pub fn continuous_request(
    config: &Configuration,
    runtime: &Runtime,
    snapshot: &Snapshot,
    slot: &str,
    gesture_active: bool,
) -> Result<Request> {
    continuous_permission(config, runtime, snapshot, slot, gesture_active)?;
    let active = &runtime.presentations[slot];
    let mut request = Request::open(
        config,
        Target {
            view: active.view.clone(),
            roots: BTreeMap::from([(active.root.clone(), Id::from(slot))]),
        },
    );
    request.mode = TransitionMode::Reflow;
    request.selected_tabs = active.selected_tabs.clone();
    request.filter.clone_from(&active.filter);
    Ok(request)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VariantHistory {
    pub recent: Vec<(Id, u64)>,
    pub frozen: bool,
    pub revision: u64,
}
impl VariantHistory {
    /// Four alternating choices within two seconds freeze the last committed choice.
    pub fn observe(&mut self, choice: &str, now_ms: u64, revision: u64) -> bool {
        if revision != self.revision || self.recent.last().is_some_and(|(_, prior)| *prior > now_ms)
        {
            *self = Self { revision, ..Self::default() };
        }
        if self.frozen {
            return true;
        }
        self.recent.retain(|(_, time)| now_ms.saturating_sub(*time) <= 2000);
        if self.recent.last().is_none_or(|(prior, _)| prior != choice) {
            self.recent.push((choice.into(), now_ms));
        }
        if self.recent.len() > 8 {
            self.recent.remove(0);
        }
        if self.recent.len() >= 4 {
            let recent = &self.recent[self.recent.len() - 4..];
            self.frozen = recent[0].0 == recent[2].0
                && recent[1].0 == recent[3].0
                && recent[0].0 != recent[1].0;
        }
        self.frozen
    }
}
