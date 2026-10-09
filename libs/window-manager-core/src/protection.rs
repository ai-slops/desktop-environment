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

/// Designation only: never advertises capture safety. Public intersections cannot host controls.
#[must_use]
pub fn control_bounds(config: &Configuration, snapshot: &crate::Snapshot) -> Option<crate::Rect> {
    let public: Vec<_> = config
        .slots
        .values()
        .filter(|slot| slot.designated_public)
        .filter_map(|slot| crate::slot_bounds(slot, snapshot).ok())
        .collect();
    config.slots.values().filter(|slot| !slot.designated_public).find_map(|slot| {
        let slot = crate::resolve_slot(config, slot, snapshot).ok()?;
        let display = snapshot.displays.get(&slot.display)?;
        let bounds = crate::slot_bounds(&slot, snapshot).ok()?;
        let scale = f64::from(display.dpi) / 96.0;
        (f64::from(bounds.width) >= 1040.0 * scale
            && f64::from(bounds.height) >= 704.0 * scale
            && !public.iter().any(|area| area.overlaps(bounds)))
        .then_some(bounds)
    })
}

/// Designation is a persistent content-sharing warning, never proof of capture privacy.
#[must_use]
pub fn shared_public_content(
    config: &crate::Configuration,
    runtime: &crate::Runtime,
    window: &str,
    now_ms: u64,
) -> bool {
    if config.windows.get(window).is_some_and(|reference| reference.public_content)
        || runtime.public_content.contains(window)
        || runtime.providers.output(window, now_ms) == crate::OutputState::OutputLinked
        || runtime.claims.get(window).is_some_and(|claim| {
            config.slots.get(&claim.slot).is_some_and(|slot| slot.designated_public)
        })
    {
        return true;
    }
    let targets =
        config.compositions.values().flat_map(|composition| composition.targets.clone()).chain(
            config
                .shortcuts
                .iter()
                .filter_map(|shortcut| shortcut.target.resolve(config).ok())
                .flat_map(|request| request.targets),
        );
    targets.into_iter().any(|target| {
        target.roots.iter().any(|(role, slot)| {
            config.slots.get(slot).is_some_and(|slot| slot.designated_public)
                && config.views.get(&target.view).and_then(|view| view.roots.get(role)).is_some_and(
                    |root| {
                        let mut placements = Vec::new();
                        root.placements(&mut placements);
                        placements.iter().any(|placement| placement.window == window)
                    },
                )
        })
    })
}
