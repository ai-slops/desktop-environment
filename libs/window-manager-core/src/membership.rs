use crate::{Configuration, Error, ErrorCode, Id, Node, Placement, Result};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MembershipDelta {
    pub added: Vec<Id>,
    pub removed: Vec<Id>,
}

impl Configuration {
    /// Returns a staged independent configuration. Existing active presentations do not change.
    pub fn stage_memberships(&self) -> Result<(Self, MembershipDelta)> {
        self.validate()?;
        let mut draft = self.clone();
        let mut delta = MembershipDelta::default();
        for view in draft.views.values_mut() {
            for node in view.roots.values_mut() {
                reconcile(node, self, &mut delta)?;
            }
        }
        draft.revision += 1;
        draft.validate()?;
        Ok((draft, delta))
    }
}

fn reconcile(node: &mut Node, config: &Configuration, delta: &mut MembershipDelta) -> Result<()> {
    let Node::Group(group) = node else {
        return Ok(());
    };
    if let Some(mut membership) = group.membership.take() {
        let collection = config.collections.get(&membership.collection).ok_or_else(|| {
            Error::new(
                ErrorCode::TargetMissing,
                "Selector Collection missing",
                &membership.collection,
            )
        })?;
        let selected: BTreeSet<_> = config
            .windows
            .values()
            .filter(|window| {
                !membership.exclude.contains(&window.id)
                    && (membership.include.contains(&window.id) || collection.selects(window))
            })
            .map(|window| window.id.clone())
            .collect();
        let mut index = 0;
        while index < group.children.len() {
            let child = &group.children[index];
            if let Node::Placement(placement) = &child
                && membership.generated.get(&placement.window) == Some(&placement.id)
                && !selected.contains(&placement.window)
            {
                delta.removed.push(placement.id.clone());
                let (child, weights) = group.take_child(index);
                if let Node::Placement(placement) = child {
                    membership.weights.insert(placement.window.clone(), weights);
                    membership.retired.insert(placement.window.clone(), placement);
                }
                continue;
            }
            index += 1;
        }
        for window in selected {
            // Manual occurrences remain independent; don't create a duplicate immediate occurrence.
            if group.children.iter().any(
                |child| matches!(child, Node::Placement(placement) if placement.window == window),
            ) {
                continue;
            }
            let placement = membership
                .retired
                .remove(&window)
                .unwrap_or_else(|| Placement::new(window.clone(), membership.role.clone()));
            let weights = membership.weights.remove(&window).unwrap_or_default();
            membership.generated.insert(window, placement.id.clone());
            delta.added.push(placement.id.clone());
            group.insert_child(group.children.len(), Node::Placement(placement), &weights);
        }
        group.membership = Some(membership);
    }
    for child in &mut group.children {
        reconcile(child, config, delta)?;
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct AssignmentOrigin {
    pub group: crate::Id,
    pub inferred: bool,
    pub priority: i32,
    pub window: crate::Id,
    pub certain: bool,
}
#[must_use]
pub fn assignment_origins(
    config: &crate::Configuration,
    request: &crate::Request,
) -> std::collections::BTreeMap<crate::Id, AssignmentOrigin> {
    fn visit(
        node: &crate::Node,
        parent: Option<&crate::Group>,
        explicit_branch: bool,
        certain: bool,
        request: &crate::Request,
        output: &mut std::collections::BTreeMap<crate::Id, AssignmentOrigin>,
    ) {
        match node {
            crate::Node::Placement(placement) => {
                let inferred = !explicit_branch
                    && parent.is_some_and(|group| {
                        group.membership.as_ref().is_some_and(|membership| {
                            membership.generated.get(&placement.window) == Some(&placement.id)
                        })
                    });
                output.insert(
                    placement.id.clone(),
                    AssignmentOrigin {
                        group: parent.map_or_else(String::new, |group| group.id.clone()),
                        inferred,
                        priority: parent.map_or(0, |group| group.rule_priority),
                        window: placement.window.clone(),
                        certain,
                    },
                );
            }
            crate::Node::Group(group) => {
                for child in &group.children {
                    let may_fold =
                        matches!(
                            group.strategy,
                            crate::Strategy::SemanticTabs | crate::Strategy::ResponsiveTabs
                        ) || group.variants.iter().any(|variant| {
                            matches!(
                                variant.strategy,
                                crate::Strategy::SemanticTabs | crate::Strategy::ResponsiveTabs
                            )
                        }) || group.allowed_fallbacks.contains(&crate::Strategy::ResponsiveTabs);
                    visit(
                        child,
                        Some(group),
                        explicit_branch
                            || (request.mode != crate::TransitionMode::Reflow
                                && request
                                    .selected_tabs
                                    .get(&group.id)
                                    .is_some_and(|selected| child.find(selected).is_some())),
                        certain && !may_fold,
                        request,
                        output,
                    );
                }
            }
        }
    }
    let mut output = std::collections::BTreeMap::new();
    for target in &request.targets {
        if let Some(view) = config.views.get(&target.view) {
            for role in target.roots.keys() {
                if let Some(root) = view.roots.get(role) {
                    visit(root, None, false, request.expansion.is_none(), request, &mut output);
                }
            }
        }
    }
    output
}
pub fn assignment_wins(
    candidate: &AssignmentOrigin,
    candidate_id: &str,
    prior: &AssignmentOrigin,
    prior_id: &str,
) -> crate::Result<bool> {
    if candidate_id == prior_id || !candidate.inferred && !prior.inferred {
        return Err(crate::Error::new(
            crate::ErrorCode::ClaimConflict,
            "Two explicit interactive occurrences require an explicit transfer",
            candidate_id,
        ));
    }
    Ok(if candidate.inferred != prior.inferred {
        !candidate.inferred
    } else if candidate.priority != prior.priority {
        candidate.priority > prior.priority
    } else {
        (&candidate.group, candidate_id) < (&prior.group, prior_id)
    })
}
#[must_use]
pub fn suppress_assignments(
    node: &crate::Node,
    suppressed: &std::collections::BTreeSet<crate::Id>,
) -> Option<crate::Node> {
    match node {
        crate::Node::Placement(placement) => {
            (!suppressed.contains(&placement.id)).then(|| node.clone())
        }
        crate::Node::Group(group) => {
            let mut copy = group.clone();
            let mut index = 0;
            while index < copy.children.len() {
                if let Some(child) = suppress_assignments(&copy.children[index], suppressed) {
                    copy.children[index] = child;
                    index += 1;
                } else {
                    copy.take_child(index);
                }
            }
            if !group.children.is_empty() && copy.children.is_empty() {
                None
            } else {
                Some(crate::Node::Group(copy))
            }
        }
    }
}
