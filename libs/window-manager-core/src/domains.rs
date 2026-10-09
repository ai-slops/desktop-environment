use crate::{
    Configuration, Error, ErrorCode, Id, Node, Plan, Request, Result, Runtime, Snapshot, Target,
    plan, resolve_slot, slot_bounds,
};
use std::collections::{BTreeMap, BTreeSet};

fn groups(node: &Node, output: &mut BTreeSet<Id>) {
    if let Node::Group(group) = node {
        output.insert(group.id.clone());
        for child in &group.children {
            groups(child, output);
        }
    }
}

/// Conservative connected domains include shared resources, prior ownership and overlapping regions.
fn domains(
    config: &Configuration,
    runtime: &Runtime,
    snapshot: &Snapshot,
    request: &Request,
) -> Result<Vec<BTreeSet<Id>>> {
    let mut resources = BTreeMap::<Id, BTreeSet<Id>>::new();
    let mut tab_groups = BTreeMap::new();
    for target in &request.targets {
        config.validate_target(target)?;
        for (root, slot) in &target.roots {
            let node = &config.views[&target.view].roots[root];
            let mut leaves = Vec::new();
            node.placements(&mut leaves);
            if resources
                .insert(slot.clone(), leaves.iter().map(|leaf| leaf.window.clone()).collect())
                .is_some()
            {
                return Err(Error::new(ErrorCode::OutOfScope, "Duplicate target slot", slot));
            }
            let mut local_groups = BTreeSet::new();
            groups(node, &mut local_groups);
            tab_groups.insert(slot.clone(), local_groups);
        }
    }
    if resources.keys().cloned().collect::<BTreeSet<_>>() != request.scope
        || request.scope.len() > 128
    {
        return Err(Error::new(
            ErrorCode::OutOfScope,
            "Scope must exactly match targets and contain at most 128 slots",
            &request.id,
        ));
    }
    for (window, claim) in &runtime.claims {
        if let Some(resources) = resources.get_mut(&claim.slot) {
            resources.insert(window.clone());
        }
    }
    let allowed_groups: BTreeSet<_> = tab_groups.values().flatten().cloned().collect();
    if request.focus.as_ref().is_some_and(|window| {
        !request.retain.contains(window)
            && !resources.values().any(|windows| windows.contains(window))
    }) {
        return Err(Error::new(
            ErrorCode::OutOfScope,
            "Focus target outside requested roots",
            &request.id,
        ));
    }
    if request.selected_tabs.keys().any(|id| !allowed_groups.contains(id)) {
        return Err(Error::new(ErrorCode::OutOfScope, "Tab outside requested scope", &request.id));
    }
    // A retain operation can carry a resource between roots: conservatively keep it atomic.
    if !request.retain.is_empty() {
        return Ok(vec![request.scope.clone()]);
    }
    let areas: BTreeMap<_, _> = request
        .scope
        .iter()
        .filter_map(|id| {
            resolve_slot(config, &config.slots[id], snapshot)
                .and_then(|slot| slot_bounds(&slot, snapshot))
                .ok()
                .map(|area| (id.clone(), area))
        })
        .collect();
    let mut result: Vec<BTreeSet<Id>> = Vec::new();
    for (slot, windows) in &resources {
        let mut connected = BTreeSet::from([slot.clone()]);
        let mut index = 0;
        while index < result.len() {
            if result[index].iter().any(|other| {
                !resources[other].is_disjoint(windows)
                    || areas.get(slot).zip(areas.get(other)).is_some_and(|(a, b)| a.overlaps(*b))
            }) {
                connected.extend(result.remove(index));
            } else {
                index += 1;
            }
        }
        result.push(connected);
    }
    Ok(result)
}

/// Invalid authority rejects the request; independent constraint failures can yield a smaller explicit plan.
#[allow(clippy::too_many_lines)] // Validate once, partition conservatively, and merge only independent final diffs.
pub fn plan_independent(
    config: &Configuration,
    runtime: &Runtime,
    snapshot: &Snapshot,
    request: &Request,
) -> Result<Plan> {
    let original = match plan(config, runtime, snapshot, request) {
        Ok(mut plan) => {
            plan.domains = domains(config, runtime, snapshot, request)?;
            return Ok(plan);
        }
        Err(error) => error,
    };
    if !matches!(
        original.code,
        ErrorCode::UnsatisfiableConstraints
            | ErrorCode::UnsupportedOperation
            | ErrorCode::TargetMissing
            | ErrorCode::ClaimConflict
            | ErrorCode::OutputStateUnknown
            | ErrorCode::FormulaInvalid
            | ErrorCode::FormulaBudgetExceeded
    ) {
        return Err(original);
    }
    let connected = domains(config, runtime, snapshot, request)?;
    if connected.len() < 2 {
        return Err(original);
    }
    // A malformed tab choice cannot be discarded while splitting the request.
    for target in &request.targets {
        for root in target.roots.keys() {
            fn validate_tabs(node: &Node, choices: &BTreeMap<Id, Id>) -> Result<()> {
                if let Node::Group(group) = node {
                    if choices
                        .get(&group.id)
                        .is_some_and(|id| !group.children.iter().any(|child| child.id() == id))
                    {
                        return Err(Error::new(
                            ErrorCode::TargetMissing,
                            "Invalid tab child",
                            &group.id,
                        ));
                    }
                    for child in &group.children {
                        validate_tabs(child, choices)?;
                    }
                }
                Ok(())
            }
            validate_tabs(&config.views[&target.view].roots[root], &request.selected_tabs)?;
        }
    }
    let mut successful = Vec::new();
    let mut blocked = BTreeMap::new();
    for domain in connected {
        let mut component = request.clone();
        component.scope.clone_from(&domain);
        component.targets = request
            .targets
            .iter()
            .filter_map(|target| {
                let roots: BTreeMap<_, _> = target
                    .roots
                    .iter()
                    .filter(|(_, slot)| domain.contains(*slot))
                    .map(|(root, slot)| (root.clone(), slot.clone()))
                    .collect();
                (!roots.is_empty()).then(|| Target { view: target.view.clone(), roots })
            })
            .collect();
        let mut local_groups = BTreeSet::new();
        let mut windows = BTreeSet::new();
        for target in &component.targets {
            for root in target.roots.keys() {
                let node = &config.views[&target.view].roots[root];
                groups(node, &mut local_groups);
                let mut leaves = Vec::new();
                node.placements(&mut leaves);
                windows.extend(leaves.iter().map(|leaf| leaf.window.clone()));
            }
        }
        component.selected_tabs.retain(|group, _| local_groups.contains(group));
        component.focus = request.focus.clone().filter(|window| windows.contains(window));
        match plan(config, runtime, snapshot, &component) {
            Ok(mut planned) => {
                planned.domains = vec![domain];
                successful.push(planned);
            }
            Err(error) => {
                for slot in domain {
                    blocked.insert(slot, error.clone());
                }
            }
        }
    }
    if successful.is_empty() {
        return Err(original);
    }
    let all_idempotent = successful.iter().all(|plan| plan.idempotent);
    let mut merged = successful[0].clone();
    merged.scope.clear();
    merged.generations.clear();
    merged.expected.clear();
    merged.presentations.clear();
    merged.desired.clear();
    merged.mutations.clear();
    merged.mutation_slots.clear();
    merged.domains.clear();
    merged.impact = crate::Impact::default();
    merged.diagnostics.clear();
    for component in successful {
        merged.impact.unchanged += component.impact.unchanged;
        merged.impact.moved += component.impact.moved;
        merged.impact.resized += component.impact.resized;
        merged.impact.shown += component.impact.shown;
        merged.impact.hidden += component.impact.hidden;
        merged.impact.focus_requests += component.impact.focus_requests;
        merged.diagnostics.extend(component.diagnostics);
        if component.idempotent && !all_idempotent {
            merged.diagnostics.push(format!("Unchanged scopes: {:?}", component.scope));
            continue;
        }
        merged.scope.extend(component.scope);
        merged.generations.extend(component.generations);
        merged.expected.extend(component.expected);
        merged.presentations.extend(component.presentations);
        merged.desired.extend(component.desired);
        merged.mutations.extend(component.mutations);
        merged.mutation_slots.extend(component.mutation_slots);
        merged.domains.extend(component.domains);
    }
    for (slot, error) in &blocked {
        merged.diagnostics.push(format!("{slot}: blocked independently: {error}"));
    }
    merged.blocked = blocked;
    merged.idempotent = all_idempotent;
    Ok(merged)
}
