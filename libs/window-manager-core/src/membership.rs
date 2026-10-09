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
    if let Some(membership) = &mut group.membership {
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
            .filter(|window| collection.selects(window))
            .map(|window| window.id.clone())
            .collect();
        let mut keep = Vec::new();
        for child in std::mem::take(&mut group.children) {
            if let Node::Placement(placement) = &child
                && membership.generated.get(&placement.window) == Some(&placement.id)
                && !selected.contains(&placement.window)
            {
                delta.removed.push(placement.id.clone());
                membership.retired.insert(placement.window.clone(), placement.clone());
                continue;
            }
            keep.push(child);
        }
        group.children = keep;
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
            membership.generated.insert(window, placement.id.clone());
            delta.added.push(placement.id.clone());
            group.children.push(Node::Placement(placement));
        }
    }
    for child in &mut group.children {
        reconcile(child, config, delta)?;
    }
    Ok(())
}
