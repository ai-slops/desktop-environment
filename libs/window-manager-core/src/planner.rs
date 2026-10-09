// Counts are bounded by configuration validation (256); pixel conversion is rounded
// explicitly and all final rectangles are validated before producing any mutation.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
use crate::{
    Binding, Configuration, DisplaySlot, Error, ErrorCode, Group, Id, Node, ObservedWindow,
    Placement, Rect, Result, ShowState, Snapshot, Strategy, Target, context_key, evaluate, new_id,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionMode {
    #[default]
    Open,
    KeepSize,
    KeepHere,
    Bring,
    Restore,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: Id,
    pub expected_revision: u64,
    pub scope: BTreeSet<Id>,
    pub targets: Vec<Target>,
    pub mode: TransitionMode,
    pub retain: BTreeSet<Id>,
    /// Explicit stable child IDs, never a tab's array index.
    pub selected_tabs: BTreeMap<Id, Id>,
    pub focus: Option<Id>,
}

impl Request {
    #[must_use]
    pub fn open(config: &Configuration, target: Target) -> Self {
        Self {
            id: new_id("request"),
            expected_revision: config.revision,
            scope: target.roots.values().cloned().collect(),
            targets: vec![target],
            mode: TransitionMode::Open,
            retain: BTreeSet::new(),
            selected_tabs: BTreeMap::new(),
            focus: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    pub slot: Id,
    pub placement: Id,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisitOverride {
    pub mode: TransitionMode,
    pub frame: Rect,
    pub client: [i32; 2],
    pub dpi: u32,
    pub display: Id,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presentation {
    pub view: Id,
    pub root: String,
    pub visit: Id,
    pub selected_tabs: BTreeMap<Id, Id>,
    pub variants: BTreeMap<Id, Id>,
    pub overrides: BTreeMap<Id, VisitOverride>,
}

#[derive(Clone, Debug, Default)]
pub struct Runtime {
    pub presentations: BTreeMap<Id, Presentation>,
    pub claims: BTreeMap<Id, Claim>,
    pub generations: BTreeMap<Id, u64>,
    pub completed: BTreeSet<Id>,
    pub paused: bool,
    /// Failed components require explicit resumption; other slots remain available.
    pub suspended: BTreeSet<Id>,
}

#[derive(Clone, Debug)]
pub struct Desired {
    pub window: Id,
    pub placement: Id,
    pub slot: Id,
    pub frame: Rect,
    pub context: String,
    pub allocated: Rect,
    pub strict_size: bool,
    pub carried: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mutation {
    pub window: Id,
    pub binding: Binding,
    pub geometry: Option<Rect>,
    pub move_only: bool,
    pub visible: Option<bool>,
    pub focus: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Impact {
    pub unchanged: usize,
    pub moved: usize,
    pub resized: usize,
    pub shown: usize,
    pub hidden: usize,
    pub focus_requests: usize,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub id: Id,
    pub config_revision: u64,
    pub topology_revision: u64,
    pub scope: BTreeSet<Id>,
    pub generations: BTreeMap<Id, u64>,
    pub expected: BTreeMap<Id, ObservedWindow>,
    pub presentations: BTreeMap<Id, Presentation>,
    pub desired: BTreeMap<Id, Desired>,
    pub mutations: Vec<Mutation>,
    pub impact: Impact,
    pub diagnostics: Vec<String>,
    pub idempotent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Planned,
    Applying,
    Settled,
    PartiallyApplied,
    Blocked,
    Superseded,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WindowResult {
    pub window: Id,
    pub submitted: bool,
    pub settled: bool,
    pub error: Option<Error>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransitionResult {
    pub request: Id,
    pub status: Status,
    pub windows: Vec<WindowResult>,
    /// Submission or geometry observation cannot establish application render readiness.
    pub rendering_readiness: String,
}

fn conflict(id: &str, message: impl Into<String>) -> Error {
    Error::new(ErrorCode::UnsatisfiableConstraints, message, id)
}

fn tab_targets(node: &Node, output: &mut BTreeMap<Id, BTreeSet<Id>>) {
    if let Node::Group(group) = node {
        output.insert(
            group.id.clone(),
            group.children.iter().map(|child| child.id().into()).collect(),
        );
        for child in &group.children {
            tab_targets(child, output);
        }
    }
}

fn validate_binding_uniqueness(snapshot: &Snapshot) -> Result<()> {
    let mut identities = BTreeMap::new();
    for (id, observed) in &snapshot.windows {
        let binding = &observed.binding;
        let identity = (binding.handle, binding.process, binding.process_started, binding.session);
        if let Some(other) = identities.insert(identity, id) {
            return Err(Error::new(
                ErrorCode::AmbiguousBinding,
                format!(
                    "Window references {other} and {id} resolve to one real window; reuse a single reference"
                ),
                id,
            ));
        }
    }
    Ok(())
}

pub fn slot_bounds(slot: &DisplaySlot, snapshot: &Snapshot) -> Result<Rect> {
    let monitor = snapshot.displays.get(&slot.display).ok_or_else(|| {
        Error::new(
            ErrorCode::TargetMissing,
            "Display unavailable; explicit remapping is required",
            &slot.id,
        )
    })?;
    let [x, y, width, height] = slot.region;
    let area = monitor.work_area;
    let result = Rect {
        x: area.x + (x * f64::from(area.width)).round() as i32,
        y: area.y + (y * f64::from(area.height)).round() as i32,
        width: (width * f64::from(area.width)).round() as i32,
        height: (height * f64::from(area.height)).round() as i32,
    };
    result.validate()?;
    Ok(result)
}

/// Computes a final diff without native side effects or changes to authored state.
#[allow(clippy::too_many_lines)]
pub fn plan(
    config: &Configuration,
    runtime: &Runtime,
    snapshot: &Snapshot,
    request: &Request,
) -> Result<Plan> {
    config.validate()?;
    validate_binding_uniqueness(snapshot)?;
    for display in snapshot.displays.values() {
        display.work_area.validate()?;
        if !(48..=960).contains(&display.dpi) {
            return Err(conflict(&display.id, "Display DPI is invalid or unsupported"));
        }
    }
    if config.revision != request.expected_revision {
        return Err(Error::new(
            ErrorCode::StaleRevision,
            "Configuration changed since request",
            &request.id,
        ));
    }
    if runtime.paused || !runtime.suspended.is_disjoint(&request.scope) {
        return Err(Error::new(
            ErrorCode::PermissionDenied,
            "Management is paused; resume before applying a transition",
            &request.id,
        ));
    }
    let mut mapped = BTreeMap::new();
    for target in &request.targets {
        config.validate_target(target)?;
        for (root, slot) in &target.roots {
            if mapped.insert(slot.clone(), (target.view.clone(), root.clone())).is_some() {
                return Err(Error::new(
                    ErrorCode::OutOfScope,
                    "Multiple target roots address the same slot",
                    slot,
                ));
            }
        }
    }
    if request.scope.is_empty() || mapped.keys().cloned().collect::<BTreeSet<_>>() != request.scope
    {
        return Err(Error::new(
            ErrorCode::OutOfScope,
            "Target mappings must exactly match the explicit switching scope",
            &request.id,
        ));
    }
    let mut tab_choices = BTreeMap::new();
    for (view, root) in mapped.values() {
        tab_targets(&config.views[view].roots[root], &mut tab_choices);
    }
    for (group, child) in &request.selected_tabs {
        let choices = tab_choices.get(group).ok_or_else(|| {
            Error::new(ErrorCode::OutOfScope, "Tab Group is outside the target scope", group)
        })?;
        if !choices.contains(child) {
            return Err(Error::new(
                ErrorCode::TargetMissing,
                "Selected tab child is not in this Group",
                child,
            ));
        }
    }
    if request.mode == TransitionMode::Open && !request.retain.is_empty() {
        return Err(Error::new(
            ErrorCode::InvalidConfiguration,
            "Open mode cannot contain preservation targets",
            &request.id,
        ));
    }
    let generations = request
        .scope
        .iter()
        .map(|slot| (slot.clone(), runtime.generations.get(slot).copied().unwrap_or_default()))
        .collect();
    let mut result = Plan {
        id: request.id.clone(),
        config_revision: config.revision,
        topology_revision: snapshot.topology_revision,
        scope: request.scope.clone(),
        generations,
        expected: BTreeMap::new(),
        presentations: BTreeMap::new(),
        desired: BTreeMap::new(),
        mutations: Vec::new(),
        impact: Impact::default(),
        diagnostics: Vec::new(),
        idempotent: false,
    };
    // An ordinary recall preserves the current Visit and manual live state.
    if runtime.completed.contains(&request.id)
        || (request.mode == TransitionMode::Open
            && request.focus.is_none()
            && mapped.iter().all(|(slot, (view, root))| {
                runtime.presentations.get(slot).is_some_and(|active| {
                    &active.view == view
                        && &active.root == root
                        && request
                            .selected_tabs
                            .iter()
                            .all(|(group, child)| active.selected_tabs.get(group) == Some(child))
                })
            }))
    {
        result.idempotent = true;
        result.impact.unchanged =
            runtime.claims.values().filter(|claim| request.scope.contains(&claim.slot)).count();
        return Ok(result);
    }
    let mut bounds = BTreeMap::new();
    for (id, slot) in &config.slots {
        if request.scope.contains(id) || runtime.presentations.contains_key(id) {
            bounds.insert(id.clone(), slot_bounds(slot, snapshot)?);
        }
    }
    let bound_entries: Vec<_> = bounds.iter().collect();
    for (index, (id, rect)) in bound_entries.iter().enumerate() {
        for (other, other_rect) in &bound_entries[..index] {
            if rect.overlaps(**other_rect) {
                return Err(conflict(
                    id,
                    format!("Display Slot overlaps {other}; explicit overlays are unsupported"),
                ));
            }
        }
    }
    let mut assigned_retained = BTreeSet::new();
    for (slot_id, (view_id, root_role)) in &mapped {
        let slot = &config.slots[slot_id];
        let view = &config.views[view_id];
        let root = &view.roots[root_role];
        let area = bounds[slot_id];
        let prior = runtime
            .presentations
            .get(slot_id)
            .filter(|prior| prior.view == *view_id && prior.root == *root_role);
        let mut presentation = prior.cloned().unwrap_or_else(|| Presentation {
            view: view_id.clone(),
            root: root_role.clone(),
            visit: new_id("visit"),
            selected_tabs: BTreeMap::new(),
            variants: BTreeMap::new(),
            overrides: BTreeMap::new(),
        });
        if request.mode == TransitionMode::Restore {
            presentation.overrides.clear();
        }
        for (group, child) in &request.selected_tabs {
            presentation.selected_tabs.insert(group.clone(), child.clone());
        }
        let mut leaves = Vec::new();
        root.placements(&mut leaves);
        for retained in &request.retain {
            let source_claim = runtime.claims.get(retained);
            if source_claim.is_some_and(|claim| !request.scope.contains(&claim.slot)) {
                return Err(Error::new(
                    ErrorCode::ClaimConflict,
                    "Retained window is owned by another switching scope",
                    retained,
                ));
            }
            let local = leaves.iter().any(|leaf| &leaf.window == retained)
                || source_claim.is_some_and(|claim| &claim.slot == slot_id)
                || (mapped.len() == 1 && request.mode == TransitionMode::Bring);
            if local && !assigned_retained.contains(retained) {
                let observed = snapshot.windows.get(retained).ok_or_else(|| {
                    Error::new(
                        ErrorCode::StaleBinding,
                        "Retained source binding is unavailable",
                        retained,
                    )
                })?;
                presentation.overrides.insert(
                    retained.clone(),
                    VisitOverride {
                        mode: request.mode,
                        frame: observed.frame,
                        client: observed.client,
                        dpi: observed.dpi,
                        display: observed.display.clone(),
                    },
                );
                result.expected.insert(retained.clone(), observed.clone());
                assigned_retained.insert(retained.clone());
            }
        }
        let monitor_dpi = snapshot.displays[&slot.display].dpi;
        let mut evaluator = Evaluator {
            config,
            snapshot,
            slot,
            dpi: monitor_dpi,
            presentation: &mut presentation,
            desired: &mut result.desired,
            diagnostics: &mut result.diagnostics,
            reservations: Vec::new(),
        };
        // Reserve immutable geometry before allocating any siblings.
        for (window_id, exception) in &evaluator.presentation.overrides {
            if exception.mode == TransitionMode::KeepHere {
                if !area.contains(exception.frame) {
                    return Err(conflict(
                        window_id,
                        "Keep-here source is outside the requested destination slot",
                    ));
                }
                evaluator.reservations.push(exception.frame);
            }
        }
        for placement in &leaves {
            if config.windows[&placement.window].protection.geometry_lock {
                let observed = snapshot.windows.get(&placement.window).ok_or_else(|| {
                    Error::new(
                        ErrorCode::StaleBinding,
                        "Protected window missing",
                        &placement.window,
                    )
                })?;
                if !area.contains(observed.frame) {
                    return Err(conflict(
                        &placement.window,
                        "Geometry-locked window is outside destination scope",
                    ));
                }
                if !evaluator.reservations.contains(&observed.frame) {
                    evaluator.reservations.push(observed.frame);
                }
            }
        }
        for (index, rect) in evaluator.reservations.iter().enumerate() {
            for prior in &evaluator.reservations[..index] {
                if rect.overlaps(*prior) {
                    return Err(conflict(slot_id, "Retained reservations overlap"));
                }
            }
        }
        evaluator.layout(root, area, "base", false)?;
        // A carried window is a Visit-local occurrence, never saved membership.
        for (window, exception) in &presentation.overrides {
            if !result.desired.contains_key(window) {
                let mut frame = exception.frame;
                if exception.mode != TransitionMode::KeepHere {
                    frame.x = area.x;
                    frame.y = area.y;
                }
                if !area.contains(frame) {
                    return Err(conflict(window, "Temporarily carried window cannot fit"));
                }
                for desired in result.desired.values().filter(|desired| &desired.slot == slot_id) {
                    if desired.frame.overlaps(frame) {
                        return Err(conflict(
                            window,
                            "Carried window overlaps destination; use keep-here or a larger slot",
                        ));
                    }
                }
                result.desired.insert(
                    window.clone(),
                    Desired {
                        window: window.clone(),
                        placement: format!("carry:{}:{window}", presentation.visit),
                        slot: slot_id.clone(),
                        frame,
                        context: context_key(slot, "base"),
                        allocated: area,
                        strict_size: true,
                        carried: true,
                    },
                );
            }
        }
        result.presentations.insert(slot_id.clone(), presentation);
    }
    if assigned_retained != request.retain {
        return Err(Error::new(
            ErrorCode::AmbiguousBinding,
            "Every retained target needs a resolved destination slot",
            &request.id,
        ));
    }
    for (window, desired) in &result.desired {
        if runtime.claims.get(window).is_some_and(|claim| !request.scope.contains(&claim.slot)) {
            return Err(Error::new(
                ErrorCode::ClaimConflict,
                "Window has an active owning Placement outside this scope",
                window,
            ));
        }
        let Some(observed) = snapshot.windows.get(window) else {
            result.diagnostics.push(format!("{window}: binding missing; placeholder remains"));
            continue;
        };
        let reference = &config.windows[window];
        if reference.protection.keep_monitor
            && observed.display != config.slots[&desired.slot].display
        {
            return Err(conflict(window, "Keep-monitor protection prevents transfer"));
        }
        if observed.show_state != ShowState::Normal && desired.frame != observed.frame {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "Minimized/maximized windows require a separate show-state action; geometry is preserved",
                window,
            ));
        }
        let geometry = (desired.frame != observed.frame).then_some(desired.frame);
        let move_only = desired.frame.width == observed.frame.width
            && desired.frame.height == observed.frame.height;
        if geometry.is_some() && (!observed.can_move || (!move_only && !observed.can_resize)) {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "Requested geometry is outside the window capability profile",
                window,
            ));
        }
        if desired.strict_size && !move_only {
            return Err(conflict(window, "Strict preserved size would resize the source"));
        }
        if reference.protection.geometry_lock && geometry.is_some() {
            return Err(conflict(window, "Geometry protection prevents mutation"));
        }
        let visible =
            (!observed.visible && observed.show_state != ShowState::Minimized).then_some(true);
        let focus = request.focus.as_ref() == Some(window);
        if focus && reference.protection.prohibit_focus {
            return Err(Error::new(
                ErrorCode::PermissionDenied,
                "Focus requests prohibited",
                window,
            ));
        }
        result.expected.insert(window.clone(), observed.clone());
        if geometry.is_none() && visible.is_none() && !focus {
            result.impact.unchanged += 1;
        } else {
            if geometry.is_some() {
                if move_only {
                    result.impact.moved += 1;
                } else {
                    result.impact.resized += 1;
                }
            }
            if visible.is_some() {
                result.impact.shown += 1;
            }
            if focus {
                result.impact.focus_requests += 1;
            }
            result.mutations.push(Mutation {
                window: window.clone(),
                binding: observed.binding.clone(),
                geometry,
                move_only,
                visible,
                focus,
            });
        }
    }
    for (window, claim) in &runtime.claims {
        if !request.scope.contains(&claim.slot) || result.desired.contains_key(window) {
            continue;
        }
        let reference = &config.windows[window];
        if reference.protection.maintain_visible {
            return Err(conflict(window, "Maintain-visible target would leave its allocation"));
        }
        if let Some(observed) = snapshot.windows.get(window)
            && observed.visible
            && observed.show_state != ShowState::Minimized
        {
            if reference.allow_hide && observed.can_hide && !observed.has_owned_dialog {
                result.expected.insert(window.clone(), observed.clone());
                result.impact.hidden += 1;
                result.mutations.push(Mutation {
                    window: window.clone(),
                    binding: observed.binding.clone(),
                    geometry: None,
                    move_only: true,
                    visible: Some(false),
                    focus: false,
                });
            } else {
                result
                    .diagnostics
                    .push(format!("{window}: left reachable; hiding unsupported or not opted in"));
            }
        }
    }
    if request.focus.as_ref().is_some_and(|window| !result.desired.contains_key(window)) {
        return Err(Error::new(
            ErrorCode::OutOfScope,
            "Focus target is not in the selected destination branch",
            request.focus.clone().unwrap_or_default(),
        ));
    }
    // No intended geometry may cover any protected live content in any active scope.
    for (window, observed) in &snapshot.windows {
        if config.windows.get(window).is_some_and(|reference| reference.protection.maintain_visible)
            && observed.visible
        {
            result.expected.insert(window.clone(), observed.clone());
            for desired in result.desired.values() {
                if desired.window != *window && desired.frame.overlaps(observed.frame) {
                    return Err(conflict(
                        window,
                        format!(
                            "Destination {} overlaps maintained-visible content",
                            desired.placement
                        ),
                    ));
                }
            }
        }
    }
    Ok(result)
}

impl Plan {
    pub fn revalidate(
        &self,
        config: &Configuration,
        runtime: &Runtime,
        snapshot: &Snapshot,
    ) -> Result<()> {
        validate_binding_uniqueness(snapshot)?;
        let revision_changed = self.config_revision != config.revision;
        if runtime.paused
            || !runtime.suspended.is_disjoint(&self.scope)
            || revision_changed
            || self.topology_revision != snapshot.topology_revision
            || self.generations.iter().any(|(slot, generation)| {
                runtime.generations.get(slot).copied().unwrap_or_default() != *generation
            })
        {
            return Err(Error::new(
                ErrorCode::StaleRevision,
                "Plan snapshot or scope generation changed; preview again",
                &self.id,
            ));
        }
        for (window, expected) in &self.expected {
            if snapshot.windows.get(window) != Some(expected) {
                return Err(Error::new(
                    ErrorCode::StaleBinding,
                    "Window identity, geometry, visibility, DPI, or capability changed since preview",
                    window,
                ));
            }
        }
        for window in self.desired.keys() {
            if runtime.claims.get(window).is_some_and(|claim| !self.scope.contains(&claim.slot)) {
                return Err(Error::new(
                    ErrorCode::ClaimConflict,
                    "A newer presentation claimed this window after preview",
                    window,
                ));
            }
        }
        for mutation in &self.mutations {
            if runtime
                .claims
                .get(&mutation.window)
                .is_some_and(|claim| !self.scope.contains(&claim.slot))
            {
                return Err(Error::new(
                    ErrorCode::ClaimConflict,
                    "A newer owner prevents mutation",
                    &mutation.window,
                ));
            }
        }
        Ok(())
    }

    /// Called only once every geometry/visibility mutation has been observed as settled.
    pub fn commit(&self, runtime: &mut Runtime) {
        if self.idempotent {
            return;
        }
        runtime.claims.retain(|_, claim| !self.scope.contains(&claim.slot));
        runtime.presentations.retain(|slot, _| !self.scope.contains(slot));
        for (window, desired) in &self.desired {
            if self.expected.contains_key(window) {
                runtime.claims.insert(
                    window.clone(),
                    Claim { slot: desired.slot.clone(), placement: desired.placement.clone() },
                );
            }
        }
        for (slot, presentation) in &self.presentations {
            runtime.presentations.insert(slot.clone(), presentation.clone());
        }
        for slot in &self.scope {
            *runtime.generations.entry(slot.clone()).or_default() += 1;
        }
        if runtime.completed.len() >= 256 {
            runtime.completed.clear();
        }
        runtime.completed.insert(self.id.clone());
    }

    /// Slots are independent unless they share a real window, which planning rejects.
    /// Keep external protection observations as revalidation dependencies.
    #[must_use]
    pub fn component(&self, slot: &str, runtime: &Runtime) -> Self {
        let mut result = self.clone();
        result.scope = BTreeSet::from([slot.into()]);
        result.generations.retain(|id, _| id == slot);
        result.presentations.retain(|id, _| id == slot);
        result.desired.retain(|_, desired| desired.slot == slot);
        result.mutations.retain(|mutation| {
            self.desired.get(&mutation.window).map_or_else(
                || runtime.claims.get(&mutation.window).is_some_and(|claim| claim.slot == slot),
                |desired| desired.slot == slot,
            )
        });
        result
    }
}

struct Evaluator<'a> {
    config: &'a Configuration,
    snapshot: &'a Snapshot,
    slot: &'a DisplaySlot,
    dpi: u32,
    presentation: &'a mut Presentation,
    desired: &'a mut BTreeMap<Id, Desired>,
    diagnostics: &'a mut Vec<String>,
    reservations: Vec<Rect>,
}

impl Evaluator<'_> {
    fn layout(
        &mut self,
        node: &Node,
        allocated: Rect,
        variant: &str,
        preserve: bool,
    ) -> Result<()> {
        match node {
            Node::Placement(placement) => self.place(placement, allocated, variant, preserve),
            Node::Group(group) => self.group(group, allocated, preserve),
        }
    }

    #[allow(clippy::too_many_lines)] // One bounded immediate-child allocation pass.
    fn group(&mut self, group: &Group, area: Rect, preserve: bool) -> Result<()> {
        if group.children.is_empty() {
            return Ok(());
        }
        let scale = f64::from(self.dpi) / 96.0;
        let prior_variant = self.presentation.variants.get(&group.id);
        let selected_variant = group.variants.iter().find(|variant| {
            let margin = if prior_variant == Some(&variant.id) { variant.hysteresis } else { 0.0 };
            variant.below_width.is_some_and(|width| f64::from(area.width) / scale < width + margin)
                || variant
                    .below_height
                    .is_some_and(|height| f64::from(area.height) / scale < height + margin)
        });
        let variant_id = selected_variant.map_or("base", |variant| variant.id.as_str());
        let strategy = if group.strategy == Strategy::SemanticTabs {
            Strategy::SemanticTabs
        } else {
            selected_variant.map_or(group.strategy, |variant| variant.strategy)
        };
        self.presentation.variants.insert(group.id.clone(), variant_id.into());
        let ratios = selected_variant.map_or(&group.ratios, |variant| &variant.ratios);
        let context = BTreeMap::from([
            ("available_width".into(), f64::from(area.width) / scale),
            ("available_height".into(), f64::from(area.height) / scale),
            ("count".into(), group.children.len() as f64),
        ]);
        let annotate = |mut error: Error| {
            error.objects = vec![group.id.clone()];
            error
        };
        let gap = evaluate(&group.gap, &context).map_err(annotate)?;
        if !(0.0..=4096.0).contains(&gap) {
            return Err(conflict(&group.id, "Gap must be between 0 and 4096 logical units"));
        }
        let gap = (gap * scale).round() as i32;
        if matches!(strategy, Strategy::SemanticTabs | Strategy::ResponsiveTabs) {
            let selected = self
                .presentation
                .selected_tabs
                .get(&group.id)
                .and_then(|id| group.children.iter().find(|child| child.id() == id));
            let child = selected.unwrap_or(&group.children[0]);
            for sibling in &group.children {
                if sibling.id() != child.id() {
                    let mut leaves = Vec::new();
                    sibling.placements(&mut leaves);
                    if leaves
                        .iter()
                        .any(|leaf| self.config.windows[&leaf.window].protection.maintain_visible)
                    {
                        return Err(conflict(
                            &group.id,
                            "Tab selection would hide a maintained-visible descendant",
                        ));
                    }
                }
            }
            self.presentation.selected_tabs.insert(group.id.clone(), child.id().into());
            return self.layout(child, area, variant_id, preserve || group.preserve_child_sizes);
        }
        if strategy == Strategy::Free {
            for child in &group.children {
                self.layout(child, area, variant_id, preserve || group.preserve_child_sizes)?;
            }
            return Ok(());
        }
        // Carve out fixed reservations first; remaining siblings share the largest free rectangle.
        let fixed: Vec<bool> = group
            .children
            .iter()
            .map(|node| match node {
                Node::Placement(placement) => {
                    self.config.windows[&placement.window].protection.geometry_lock
                        || self
                            .presentation
                            .overrides
                            .get(&placement.window)
                            .is_some_and(|exception| exception.mode == TransitionMode::KeepHere)
                }
                Node::Group(_) => false,
            })
            .collect();
        let count = fixed.iter().filter(|fixed| !**fixed).count();
        let available = if count == 0 { area } else { largest_free(area, &self.reservations)? };
        let columns = if strategy == Strategy::Grid {
            let value = evaluate(&group.columns, &context).map_err(annotate)?;
            if value.fract() != 0.0 || !(1.0..=256.0).contains(&value) {
                return Err(conflict(&group.id, "Columns must be an integer from 1 to 256"));
            }
            (value as usize).min(count.max(1))
        } else {
            1
        };
        let mut offset = 0;
        let mut index = 0;
        let total_weight =
            (0..count).map(|index| ratios.get(index).copied().unwrap_or(1.0)).sum::<f64>();
        for (child, fixed) in group.children.iter().zip(fixed) {
            if fixed {
                self.layout(child, area, variant_id, true)?;
                continue;
            }
            let rect = match strategy {
                Strategy::Horizontal | Strategy::Vertical => {
                    let horizontal = strategy == Strategy::Horizontal;
                    let span = if horizontal { available.width } else { available.height }
                        - gap * (count.saturating_sub(1) as i32);
                    if span <= 0 {
                        return Err(conflict(&group.id, "Gaps exhaust available content space"));
                    }
                    let weight = ratios.get(index).copied().unwrap_or(1.0);
                    let length = if index + 1 == count {
                        span - offset + gap * index as i32
                    } else {
                        (f64::from(span) * weight / total_weight).floor() as i32
                    };
                    let rect = if horizontal {
                        Rect { x: available.x + offset, width: length, ..available }
                    } else {
                        Rect { y: available.y + offset, height: length, ..available }
                    };
                    offset += length + gap;
                    rect
                }
                Strategy::Grid => {
                    let rows = count.div_ceil(columns).max(1);
                    let w = (available.width - gap * (columns as i32 - 1)) / columns as i32;
                    let h = (available.height - gap * (rows as i32 - 1)) / rows as i32;
                    Rect {
                        x: available.x + (index % columns) as i32 * (w + gap),
                        y: available.y + (index / columns) as i32 * (h + gap),
                        width: w,
                        height: h,
                    }
                }
                _ => unreachable!("tab and free strategies handled above"),
            };
            rect.validate()?;
            self.layout(child, rect, variant_id, preserve || group.preserve_child_sizes)?;
            index += 1;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)] // Property precedence and hard constraints are evaluated together.
    fn place(
        &mut self,
        placement: &Placement,
        area: Rect,
        variant: &str,
        preserve: bool,
    ) -> Result<()> {
        if self.desired.contains_key(&placement.window) {
            return Err(Error::new(
                ErrorCode::ClaimConflict,
                "Two visible occurrences claim the same real window",
                &placement.window,
            ));
        }
        let key = context_key(self.slot, variant);
        let preference = placement
            .preferences
            .get(&key)
            .or_else(|| placement.preferences.get(&context_key(self.slot, "base")))
            .cloned()
            .unwrap_or_default();
        let Some(observed) = self.snapshot.windows.get(&placement.window) else {
            self.diagnostics.push(format!("{}: missing binding", placement.id));
            return Ok(());
        };
        let scale = f64::from(self.dpi) / 96.0;
        let decoration =
            [observed.frame.width - observed.client[0], observed.frame.height - observed.client[1]];
        let context = BTreeMap::from([
            ("available_width".into(), f64::from(area.width - decoration[0]) / scale),
            ("available_height".into(), f64::from(area.height - decoration[1]) / scale),
            ("count".into(), 1.0),
        ]);
        let mut frame = area;
        if let Some(position) = preference.position_override.or(preference.position) {
            frame.x = area.x + (position[0] * scale).round() as i32;
            frame.y = area.y + (position[1] * scale).round() as i32;
        }
        let size = preference.size_override.or(preference.client_size);
        if let Some(size) = size {
            frame.width = (size[0] * scale).round() as i32 + decoration[0];
            frame.height = (size[1] * scale).round() as i32 + decoration[1];
        }
        if preference.size_override.is_none() {
            for (formula, axis) in [(&preference.width_formula, 0), (&preference.height_formula, 1)]
            {
                if let Some(formula) = formula {
                    let value = evaluate(formula, &context).map_err(|mut error| {
                        error.objects = vec![placement.id.clone()];
                        error
                    })?;
                    if !(1.0..=65_536.0).contains(&value) {
                        return Err(conflict(
                            &placement.id,
                            "Formula size must be finite and positive",
                        ));
                    }
                    if axis == 0 {
                        frame.width = (value * scale).round() as i32 + decoration[0];
                    } else {
                        frame.height = (value * scale).round() as i32 + decoration[1];
                    }
                }
            }
        }
        let exception = self.presentation.overrides.get(&placement.window);
        let locked = self.config.windows[&placement.window].protection.geometry_lock;
        let strict_size = exception.is_some() || preserve || locked;
        if let Some(exception) = exception {
            if exception.dpi != self.dpi || exception.display != self.slot.display {
                return Err(Error::new(
                    ErrorCode::UnsupportedOperation,
                    "Strict preservation across display/DPI contexts is not validated; keep the source monitor",
                    &placement.window,
                ));
            }
            frame.width = exception.frame.width;
            frame.height = exception.frame.height;
            if exception.mode == TransitionMode::KeepHere {
                frame = exception.frame;
            }
        } else if locked {
            frame = observed.frame;
        } else if preserve {
            if observed.dpi != self.dpi {
                return Err(conflict(
                    &placement.id,
                    "Child-size preservation requires compatible DPI",
                ));
            }
            frame.width = observed.frame.width;
            frame.height = observed.frame.height;
        }
        if let Some(minimum) = placement.minimum_client
            && (f64::from(frame.width - decoration[0]) < minimum[0] * scale
                || f64::from(frame.height - decoration[1]) < minimum[1] * scale)
        {
            return Err(conflict(
                &placement.id,
                format!("Minimum client size {minimum:?} exceeds allocated content"),
            ));
        }
        frame.validate()?;
        let slot_area = slot_bounds(self.slot, self.snapshot)?;
        if !slot_area.contains(frame)
            || (!locked
                && exception.is_none_or(|exception| exception.mode != TransitionMode::KeepHere)
                && !area.contains(frame))
        {
            return Err(conflict(
                &placement.id,
                "Preferred or preserved window cannot fit its allocated rectangle; prior arrangement retained",
            ));
        }
        for reservation in &self.reservations {
            if frame != *reservation && frame.overlaps(*reservation) {
                return Err(conflict(&placement.id, "Placement overlaps a retained reservation"));
            }
        }
        self.desired.insert(
            placement.window.clone(),
            Desired {
                window: placement.window.clone(),
                placement: placement.id.clone(),
                slot: self.slot.id.clone(),
                frame,
                context: key,
                allocated: area,
                strict_size,
                carried: false,
            },
        );
        Ok(())
    }
}

/// Bounded rectangle subtraction. Never solves fit by covering a protected reservation.
fn largest_free(area: Rect, reservations: &[Rect]) -> Result<Rect> {
    let mut candidates = vec![area];
    for reservation in reservations {
        let mut next = Vec::new();
        for rect in candidates {
            if !rect.overlaps(*reservation) {
                next.push(rect);
                continue;
            }
            let left = reservation.x.max(rect.x);
            let top = reservation.y.max(rect.y);
            let right = (reservation.x + reservation.width).min(rect.x + rect.width);
            let bottom = (reservation.y + reservation.height).min(rect.y + rect.height);
            for candidate in [
                Rect { width: left - rect.x, ..rect },
                Rect { x: right, width: rect.x + rect.width - right, ..rect },
                Rect { height: top - rect.y, ..rect },
                Rect { y: bottom, height: rect.y + rect.height - bottom, ..rect },
            ] {
                if candidate.width > 0 && candidate.height > 0 {
                    next.push(candidate);
                }
            }
        }
        if next.len() > 256 {
            return Err(conflict("reservations", "Reservation solver budget exceeded"));
        }
        candidates = next;
    }
    candidates
        .into_iter()
        .max_by_key(|rect| i64::from(rect.width) * i64::from(rect.height))
        .ok_or_else(|| conflict("reservations", "Retained windows occupy all available space"))
}
