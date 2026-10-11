use crate::{Configuration, Id, Node, Protection, Runtime};

fn ancestors(node: &Node, placement: &str, inherited: &Protection) -> Option<Protection> {
    match node {
        Node::Placement(leaf) => (leaf.id == placement).then(|| inherited.union(&leaf.protection)),
        Node::Group(group) => group
            .children
            .iter()
            .find_map(|child| ancestors(child, placement, &inherited.union(&group.protection))),
    }
}

/// Window, Placement, Group, current Presentation and active Composition constraints add independently.
#[must_use]
pub fn effective_protection(
    config: &Configuration,
    runtime: &Runtime,
    window: &str,
    placement: Option<&str>,
    slot: Option<&str>,
) -> Protection {
    let mut result = config
        .windows
        .get(window)
        .map_or_else(Protection::default, |reference| reference.protection.clone());
    let claim = runtime.claims.get(window);
    let placement = placement.or_else(|| claim.map(|claim| claim.placement.as_str()));
    let slot = slot.or_else(|| claim.map(|claim| claim.slot.as_str()));
    if let Some(placement) = placement {
        for view in config.views.values() {
            if let Some(protection) = view
                .roots
                .values()
                .find_map(|root| ancestors(root, placement, &Protection::default()))
            {
                result = result.union(&protection);
                break;
            }
        }
    }
    if let Some(slot) = slot {
        if let Some(presentation) = runtime.presentations.get(slot) {
            let same_visit = placement.is_none()
                || config
                    .views
                    .get(&presentation.view)
                    .and_then(|view| view.roots.get(&presentation.root))
                    .and_then(|root| {
                        ancestors(root, placement.unwrap_or_default(), &Protection::default())
                    })
                    .is_some();
            if same_visit {
                result = result.union(&presentation.protection);
            }
        }
        for composition in config.compositions.values() {
            let contains =
                composition.targets.iter().any(|target| target.roots.values().any(|id| id == slot));
            let active = composition.targets.iter().all(|target| {
                target.roots.iter().all(|(root, slot)| {
                    runtime.presentations.get(slot).is_some_and(|presentation| {
                        presentation.view == target.view && &presentation.root == root
                    })
                })
            });
            if contains && active {
                result = result.union(&composition.protection);
            }
        }
    }
    result
}

/// Required tab path for one resource; it never changes unrelated slots or requests activation.
pub fn tab_path(
    node: &Node,
    window: &str,
    output: &mut std::collections::BTreeMap<Id, Id>,
) -> bool {
    match node {
        Node::Placement(placement) => placement.window == window,
        Node::Group(group) => {
            for child in &group.children {
                if tab_path(child, window, output) {
                    output.insert(group.id.clone(), child.id().into());
                    return true;
                }
            }
            false
        }
    }
}
