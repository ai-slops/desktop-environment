use crate::{
    Binding, Configuration, Display, Error, ErrorCode, Id, Node, ObservedWindow, Rect, Request,
    Result, Runtime, ShowState, Snapshot, Target, new_id, plan,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Simulation {
    pub target: Target,
    pub width: i32,
    pub height: i32,
    pub dpi: u32,
    pub group: Option<Id>,
    pub count: Option<usize>,
    pub minimum: Option<[f64; 2]>,
    pub fixed_children: bool,
    pub missing: BTreeSet<Id>,
    pub display_present: bool,
}

/// Deliberately omits native mutations and bindings; this cannot become an executable Plan.
#[derive(Clone, Debug, Serialize)]
pub struct SimulationReport {
    pub area: Rect,
    pub dpi: u32,
    pub desired: BTreeMap<Id, crate::Desired>,
    pub impact: crate::Impact,
    pub diagnostics: Vec<String>,
    pub variants: BTreeMap<Id, Id>,
    pub parameters: BTreeMap<String, String>,
    pub error: Option<Error>,
    pub group_inputs: BTreeMap<Id, BTreeMap<String, f64>>,
}

#[allow(clippy::too_many_lines, clippy::cast_precision_loss, clippy::items_after_statements)] // Bounded synthetic fixture construction, helpers stay local to the no-native boundary.
pub fn simulate(config: &Configuration, input: &Simulation) -> Result<SimulationReport> {
    let area = Rect { x: 0, y: 0, width: input.width, height: input.height };
    area.validate()?;
    if !(48..=768).contains(&input.dpi)
        || input.count.is_some_and(|count| count > 256)
        || input.target.roots.len() != 1
    {
        return Err(Error::new(
            ErrorCode::InvalidConfiguration,
            "Simulation dimensions/DPI/count or root mapping exceeds bounds",
            "simulation",
        ));
    }
    config.validate_target(&input.target)?;
    let mut draft = config.clone();
    let mut extra = Vec::new();
    let view = draft.views.get_mut(&input.target.view).ok_or_else(|| {
        Error::new(ErrorCode::TargetMissing, "Simulation View missing", "simulation")
    })?;
    let role = input.target.roots.keys().next().ok_or_else(|| {
        Error::new(ErrorCode::TargetMissing, "Simulation root missing", "simulation")
    })?;
    let root = view
        .roots
        .get_mut(role)
        .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "Simulation root missing", role))?;
    if let Some(count) = input.count {
        let group_id = input.group.as_deref().unwrap_or_else(|| root.id());
        let group_id = group_id.to_owned();
        let group = root.group_mut(&group_id).ok_or_else(|| {
            Error::new(
                ErrorCode::TargetMissing,
                "Synthetic count needs an addressed Group",
                &group_id,
            )
        })?;
        group.membership = None;
        group.children.truncate(count);
        while group.children.len() < count {
            let window = new_id("synthetic-window");
            group
                .children
                .push(Node::Placement(crate::Placement::new(window.clone(), "synthetic".into())));
            extra.push(window);
        }
        group.ratios.truncate(count);
        for variant in &mut group.variants {
            variant.ratios.truncate(count);
        }
    }
    fn set_constraints(node: &mut Node, minimum: Option<[f64; 2]>, fixed: bool) {
        match node {
            Node::Placement(placement) => {
                if minimum.is_some() {
                    placement.minimum_client = minimum;
                }
            }
            Node::Group(group) => {
                group.preserve_child_sizes |= fixed;
                for child in &mut group.children {
                    set_constraints(child, minimum, fixed);
                }
            }
        }
    }
    set_constraints(root, input.minimum, input.fixed_children);
    for window in extra {
        draft.windows.insert(
            window.clone(),
            crate::WindowRef {
                id: window,
                alias: "Synthetic candidate".into(),
                tags: Vec::new(),
                application_hint: None,
                allow_hide: true,
                public_content: false,
                protection: crate::Protection::default(),
                output_protection: crate::OutputProtection::None,
                capabilities: crate::CapabilityProfile::default(),
            },
        );
    }
    let slot_id = input.target.roots.values().next().ok_or_else(|| {
        Error::new(ErrorCode::TargetMissing, "Simulation slot missing", "simulation")
    })?;
    let slot = draft
        .slots
        .get_mut(slot_id)
        .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "Simulation slot missing", slot_id))?;
    let original_display = slot.display.clone();
    slot.display = "simulation-display".into();
    slot.region = [0.0, 0.0, 1.0, 1.0];
    slot.designated_public = false;
    slot.fallback_displays.clear();
    let mut snapshot = Snapshot { topology_revision: 1, ..Snapshot::default() };
    if input.display_present {
        snapshot.displays.insert(
            slot.display.clone(),
            Display {
                id: slot.display.clone(),
                name: "Synthetic display".into(),
                work_area: area,
                dpi: input.dpi,
            },
        );
    }
    let mut leaves = Vec::new();
    draft.views[&input.target.view].roots[role].placements(&mut leaves);
    let candidates: BTreeSet<_> = leaves.iter().map(|leaf| leaf.window.clone()).collect();
    for (index, window) in candidates.iter().enumerate() {
        if input.missing.contains(window) {
            continue;
        }
        let width = (520 * input.dpi / 96).min(65_536);
        let height = (340 * input.dpi / 96).min(65_536);
        let frame = Rect {
            x: 0,
            y: 0,
            width: i32::try_from(width).unwrap_or(520),
            height: i32::try_from(height).unwrap_or(340),
        };
        snapshot.windows.insert(
            window.clone(),
            ObservedWindow {
                binding: Binding {
                    handle: u64::try_from(index + 1).unwrap_or(1),
                    process: 1,
                    process_started: 1,
                    session: 1,
                    generation: 1,
                    token_property: "simulation-only".into(),
                },
                frame,
                client: [frame.width - 20, frame.height - 40],
                dpi: input.dpi,
                display: "simulation-display".into(),
                visible: true,
                show_state: ShowState::Normal,
                can_move: true,
                can_resize: true,
                normal_resize_supported: true,
                can_hide: true,
                has_owned_dialog: false,
            },
        );
    }
    // Saved ordinary preferences remain meaningful without exporting real monitor identities.
    fn remap(node: &mut Node, slot: &str, old: &str) {
        match node {
            Node::Placement(placement) => {
                let mapped = placement
                    .preferences
                    .iter()
                    .filter_map(|(key, preference)| {
                        let mut parts: Vec<String> = serde_json::from_str(key).ok()?;
                        if parts.len() != 3 || parts[0] != slot || parts[1] != old {
                            return None;
                        }
                        parts[1] = "simulation-display".into();
                        Some((serde_json::to_string(&parts).ok()?, preference.clone()))
                    })
                    .collect::<Vec<_>>();
                placement.preferences.extend(mapped);
            }
            Node::Group(group) => {
                for child in &mut group.children {
                    remap(child, slot, old);
                }
            }
        }
    }
    if let Some(root) =
        draft.views.get_mut(&input.target.view).and_then(|view| view.roots.get_mut(role))
    {
        remap(root, slot_id, &original_display);
    }
    let mut report = SimulationReport {
        area,
        dpi: input.dpi,
        desired: BTreeMap::new(),
        impact: crate::Impact::default(),
        diagnostics: Vec::new(),
        variants: BTreeMap::new(),
        parameters: crate::declared_parameters(&draft.views[&input.target.view].roots),
        error: None,
        group_inputs: BTreeMap::new(),
    };
    match plan(&draft, &Runtime::default(), &snapshot, &Request::open(&draft, input.target.clone()))
    {
        Ok(plan) => {
            report.desired = plan.desired;
            report.impact = plan.impact;
            report.diagnostics = plan.diagnostics;
            for presentation in plan.presentations.values() {
                report.variants.extend(presentation.variants.clone());
                report.group_inputs.extend(presentation.group_inputs.clone());
            }
        }
        Err(error) => report.error = Some(error),
    }
    Ok(report)
}
