use crate::{Configuration, Error, ErrorCode, Id, Node, Request, Result, Target};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct PlacementLocation {
    pub view: Id,
    pub root: String,
    pub placement: Id,
    pub path: String,
}

#[must_use]
pub fn placement_locations(config: &Configuration, window: &str) -> Vec<PlacementLocation> {
    fn visit(
        node: &Node,
        window: &str,
        view: &str,
        role: &str,
        path: &str,
        output: &mut Vec<PlacementLocation>,
    ) {
        match node {
            Node::Placement(placement) if placement.window == window => {
                output.push(PlacementLocation {
                    view: view.into(),
                    root: role.into(),
                    placement: placement.id.clone(),
                    path: format!("{path} / {}", placement.role),
                });
            }
            Node::Group(group) => {
                for child in &group.children {
                    visit(child, window, view, role, &format!("{path} / {}", group.name), output);
                }
            }
            Node::Placement(_) => {}
        }
    }
    let mut locations = Vec::new();
    for view in config.views.values() {
        for (role, root) in &view.roots {
            visit(root, window, &view.id, role, &format!("{} / {role}", view.name), &mut locations);
        }
    }
    locations
}

/// Resolves the exact occurrence, rather than guessing the first occurrence of a shared resource.
pub fn placement_request(
    config: &Configuration,
    view: &str,
    placement: &str,
    slot: &str,
) -> Result<Request> {
    fn select(node: &Node, placement: &str, choices: &mut BTreeMap<Id, Id>) {
        if let Node::Group(group) = node
            && let Some(child) = group.children.iter().find(|child| child.find(placement).is_some())
        {
            choices.insert(group.id.clone(), child.id().into());
            select(child, placement, choices);
        }
    }
    config.validate()?;
    let view = config
        .views
        .get(view)
        .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "View missing", view))?;
    let (role, root) = view
        .roots
        .iter()
        .find(|(_, root)| matches!(root.find(placement), Some(Node::Placement(_))))
        .ok_or_else(|| {
            Error::new(ErrorCode::TargetMissing, "Placement missing from View", placement)
        })?;
    let target =
        Target { view: view.id.clone(), roots: BTreeMap::from([(role.clone(), slot.into())]) };
    config.validate_target(&target)?;
    let mut request = Request::open(config, target);
    select(root, placement, &mut request.selected_tabs);
    Ok(request)
}
