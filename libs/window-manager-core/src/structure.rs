use crate::{Configuration, Error, ErrorCode, Group, Id, Node, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeAddress {
    pub view: Id,
    pub node: Id,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDestination {
    pub view: Id,
    pub group: Id,
    pub index: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructureAction {
    Wrap {
        view: Id,
        parent: Id,
        children: BTreeSet<Id>,
        name: String,
        strategy: crate::Strategy,
    },
    Unwrap {
        source: NodeAddress,
    },
    Remove {
        source: NodeAddress,
    },
    Move {
        source: NodeAddress,
        destination: NodeDestination,
    },
    Copy {
        source: NodeAddress,
        destination: NodeDestination,
    },
    CopySize {
        source: NodeAddress,
        source_context: String,
        destinations: Vec<(NodeAddress, String)>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StructureEdit {
    pub expected_revision: u64,
    pub action: StructureAction,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChildWeights {
    base: Option<f64>,
    variants: BTreeMap<Id, f64>,
}

impl ChildWeights {
    #[must_use]
    pub fn divided(&self, count: usize) -> Self {
        let divisor = f64::from(u32::try_from(count.max(1)).unwrap_or(u32::MAX));
        Self {
            base: self.base.map(|value| value / divisor),
            variants: self
                .variants
                .iter()
                .map(|(id, value)| (id.clone(), value / divisor))
                .collect(),
        }
    }
    #[must_use]
    pub fn valid(&self) -> bool {
        self.variants.len() <= 64
            && self
                .base
                .iter()
                .chain(self.variants.values())
                .all(|value| value.is_finite() && *value > 0.0 && *value <= 1_000_000.0)
    }
}

impl Group {
    /// Weights travel with their child, never with its former vector index.
    pub fn take_child(&mut self, index: usize) -> (Node, ChildWeights) {
        let weights = ChildWeights {
            base: (index < self.ratios.len()).then(|| self.ratios.remove(index)),
            variants: self
                .variants
                .iter_mut()
                .filter_map(|variant| {
                    (index < variant.ratios.len())
                        .then(|| (variant.id.clone(), variant.ratios.remove(index)))
                })
                .collect(),
        };
        (self.children.remove(index), weights)
    }

    pub fn insert_child(&mut self, index: usize, node: Node, weights: &ChildWeights) {
        if !self.ratios.is_empty() || weights.base.is_some() {
            self.ratios.resize(self.children.len(), 1.0);
            self.ratios.insert(index, weights.base.unwrap_or(1.0));
        }
        for variant in &mut self.variants {
            if !variant.ratios.is_empty() || weights.variants.contains_key(&variant.id) {
                variant.ratios.resize(self.children.len(), 1.0);
                variant
                    .ratios
                    .insert(index, weights.variants.get(&variant.id).copied().unwrap_or(1.0));
            }
        }
        self.children.insert(index, node);
    }
}

fn missing(id: &str) -> Error {
    Error::new(ErrorCode::TargetMissing, "Addressed View, node or Group missing", id)
}

impl Configuration {
    pub fn node(&self, address: &NodeAddress) -> Result<&Node> {
        self.views
            .get(&address.view)
            .and_then(|view| view.roots.values().find_map(|root| root.find(&address.node)))
            .ok_or_else(|| missing(&address.node))
    }

    /// Produces a validated independent draft; live ownership and application lifetime are untouched.
    pub fn edit_structure(&self, edit: &StructureEdit) -> Result<Self> {
        if self.revision != edit.expected_revision {
            return Err(Error::new(
                ErrorCode::StaleRevision,
                "Structure revision changed",
                "structure",
            ));
        }
        let mut draft = self.clone();
        match &edit.action {
            StructureAction::Wrap { view, parent, children, name, strategy } => {
                wrap_children(&mut draft, view, parent, children, name, *strategy)?;
            }
            StructureAction::Unwrap { source } => unwrap_group(&mut draft, source)?,
            StructureAction::Remove { source } => remove_reference(&mut draft, source)?,
            StructureAction::Move { source, destination }
            | StructureAction::Copy { source, destination } => {
                let original = self.node(source)?;
                if source.view == destination.view && original.find(&destination.group).is_some() {
                    return Err(Error::new(
                        ErrorCode::InvalidConfiguration,
                        "A subtree cannot be inserted into itself",
                        &source.node,
                    ));
                }
                let (mut node, weights) = if matches!(edit.action, StructureAction::Move { .. }) {
                    take(&mut draft, source)?
                } else {
                    let mut node = original.clone();
                    crate::store::replace_node_ids(&mut node);
                    (node, ChildWeights::default())
                };
                // Explicit movement detaches source-generated ownership. The destination gets an independent occurrence.
                let group = draft
                    .views
                    .get_mut(&destination.view)
                    .and_then(|view| {
                        view.roots.values_mut().find_map(|root| root.group_mut(&destination.group))
                    })
                    .ok_or_else(|| missing(&destination.group))?;
                let index = destination.index.unwrap_or(group.children.len());
                if index > group.children.len() {
                    return Err(Error::new(
                        ErrorCode::InvalidConfiguration,
                        "Insertion index is outside Group",
                        &destination.group,
                    ));
                }
                if let Node::Placement(placement) = &mut node
                    && let Some(membership) = &mut group.membership
                {
                    membership.exclude.remove(&placement.window);
                    if let Some(existing) = membership.retired.remove(&placement.window) {
                        membership.generated.remove(&existing.window);
                    }
                    membership.weights.remove(&placement.window);
                }
                group.insert_child(index, node, &weights);
            }
            StructureAction::CopySize { source, source_context, destinations } => {
                if destinations.is_empty() || destinations.len() > 256 {
                    return Err(Error::new(
                        ErrorCode::InvalidConfiguration,
                        "Address 1 through 256 destination Placements",
                        "structure",
                    ));
                }
                let Node::Placement(source_node) = self.node(source)? else {
                    return Err(Error::new(
                        ErrorCode::InvalidConfiguration,
                        "Size source must be a Placement",
                        &source.node,
                    ));
                };
                let size = source_node
                    .preferences
                    .get(source_context)
                    .and_then(|preference| preference.size_override.or(preference.client_size))
                    .ok_or_else(|| missing(source_context))?;
                for (address, context) in destinations {
                    draft.save_properties(
                        &address.view,
                        &address.node,
                        context,
                        None,
                        Some(size),
                    )?;
                }
            }
        }
        draft.revision = self.revision + 1;
        draft.validate()?;
        Ok(draft)
    }
}

fn parent_location(config: &Configuration, address: &NodeAddress) -> Option<(Id, usize)> {
    fn find(node: &Node, target: &str) -> Option<(Id, usize)> {
        let Node::Group(group) = node else {
            return None;
        };
        group
            .children
            .iter()
            .position(|child| child.id() == target)
            .map(|index| (group.id.clone(), index))
            .or_else(|| group.children.iter().find_map(|child| find(child, target)))
    }
    config.views.get(&address.view)?.roots.values().find_map(|root| find(root, &address.node))
}

fn take(config: &mut Configuration, address: &NodeAddress) -> Result<(Node, ChildWeights)> {
    fn remove(node: &mut Node, id: &str) -> Option<(Node, ChildWeights)> {
        let Node::Group(group) = node else {
            return None;
        };
        if let Some(index) = group.children.iter().position(|child| child.id() == id) {
            let (node, weights) = group.take_child(index);
            if let Node::Placement(placement) = &node
                && let Some(membership) = &mut group.membership
            {
                membership.generated.remove(&placement.window);
                membership.retired.remove(&placement.window);
                membership.weights.remove(&placement.window);
                membership.include.remove(&placement.window);
                membership.exclude.insert(placement.window.clone());
            }
            return Some((node, weights));
        }
        group.children.iter_mut().find_map(|child| remove(child, id))
    }
    let view = config.views.get_mut(&address.view).ok_or_else(|| missing(&address.view))?;
    if let Some(role) =
        view.roots.iter().find_map(|(role, node)| (node.id() == address.node).then(|| role.clone()))
    {
        return view
            .roots
            .remove(&role)
            .map(|node| (node, ChildWeights::default()))
            .ok_or_else(|| missing(&address.node));
    }
    view.roots
        .values_mut()
        .find_map(|root| remove(root, &address.node))
        .ok_or_else(|| missing(&address.node))
}

fn wrap_children(
    config: &mut Configuration,
    view: &Id,
    parent: &Id,
    children: &BTreeSet<Id>,
    name: &str,
    strategy: crate::Strategy,
) -> Result<()> {
    let group = config
        .views
        .get_mut(view)
        .and_then(|view| view.roots.values_mut().find_map(|root| root.group_mut(parent)))
        .ok_or_else(|| missing(parent))?;
    if children.is_empty()
        || children.len() > 256
        || children.iter().any(|id| !group.children.iter().any(|child| child.id() == id))
    {
        return Err(Error::new(
            ErrorCode::OutOfScope,
            "Wrap selects existing immediate children of one parent Group",
            parent,
        ));
    }
    let index = group
        .children
        .iter()
        .position(|child| children.contains(child.id()))
        .ok_or_else(|| missing(parent))?;
    let ordered = group
        .children
        .iter()
        .filter(|child| children.contains(child.id()))
        .map(|child| child.id().to_owned())
        .collect::<Vec<_>>();
    let mut wrapped = Group::new(name.to_owned());
    wrapped.strategy = strategy;
    let mut outer = ChildWeights::default();
    for id in ordered {
        let (node, weights) = take(config, &NodeAddress { view: view.clone(), node: id })?;
        if let Some(value) = weights.base {
            *outer.base.get_or_insert(0.0) += value;
        }
        for (variant, value) in &weights.variants {
            *outer.variants.entry(variant.clone()).or_default() += value;
        }
        wrapped.insert_child(wrapped.children.len(), node, &weights);
    }
    let group = config
        .views
        .get_mut(view)
        .and_then(|view| view.roots.values_mut().find_map(|root| root.group_mut(parent)))
        .ok_or_else(|| missing(parent))?;
    group.insert_child(index, Node::Group(wrapped), &outer);
    Ok(())
}

fn unwrap_group(config: &mut Configuration, source: &NodeAddress) -> Result<()> {
    let (parent, index) = parent_location(config, source).ok_or_else(|| {
        Error::new(
            ErrorCode::OutOfScope,
            "Unwrap requires a Group with one explicit parent",
            &source.node,
        )
    })?;
    let (node, weights) = take(config, source)?;
    let Node::Group(mut nested) = node else {
        return Err(Error::new(
            ErrorCode::InvalidConfiguration,
            "Unwrap addresses a Group",
            &source.node,
        ));
    };
    let weights = weights.divided(nested.children.len());
    let group = config
        .views
        .get_mut(&source.view)
        .and_then(|view| view.roots.values_mut().find_map(|root| root.group_mut(&parent)))
        .ok_or_else(|| missing(&parent))?;
    for (offset, node) in std::mem::take(&mut nested.children).into_iter().enumerate() {
        group.insert_child(index + offset, node, &weights);
    }
    Ok(())
}

fn remove_reference(config: &mut Configuration, source: &NodeAddress) -> Result<()> {
    let role = config.views.get(&source.view).and_then(|view| {
        view.roots.iter().find(|(_, node)| node.id() == source.node).map(|(role, _)| role.clone())
    });
    take(config, source)?;
    if let Some(role) = role
        && let Some(view) = config.views.get_mut(&source.view)
        && view.roots.is_empty()
    {
        view.roots.insert(role, Node::Group(Group::new("Empty root".into())));
    }
    Ok(())
}
