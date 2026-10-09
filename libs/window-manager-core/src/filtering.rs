use crate::{Configuration, Error, ErrorCode, Node, Query, Result, Truth, WindowRef};

pub fn validate_filter(query: &Query) -> Result<()> {
    let mut pending = vec![(query, 0)];
    let mut count = 0;
    while let Some((query, depth)) = pending.pop() {
        count += 1;
        if count > 512 || depth > 32 {
            return Err(Error::new(
                ErrorCode::InvalidConfiguration,
                "Query node/depth budget exceeded",
                "filter",
            ));
        }
        match query {
            Query::Not(inner) => pending.push((inner, depth + 1)),
            Query::And(children) | Query::Or(children) => {
                if children.len() > 256 {
                    return Err(Error::new(
                        ErrorCode::InvalidConfiguration,
                        "Query width budget exceeded",
                        "filter",
                    ));
                }
                pending.extend(children.iter().map(|child| (child, depth + 1)));
            }
            Query::Tag(name) | Query::Application(name) | Query::Alias(name)
                if name.len() > 200 =>
            {
                return Err(Error::new(
                    ErrorCode::InvalidConfiguration,
                    "Query text budget exceeded",
                    "filter",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

#[must_use]
pub fn filter_allows(query: Option<&Query>, window: &WindowRef) -> bool {
    query.is_none_or(|query| query.matches(window) == Truth::Yes)
}

/// Runtime copy only; removing a child also removes its associated vector weight.
#[must_use]
pub fn filter_tree(node: &Node, query: &Query, config: &Configuration) -> Option<Node> {
    match node {
        Node::Placement(placement) => {
            filter_allows(Some(query), &config.windows[&placement.window]).then(|| node.clone())
        }
        Node::Group(group) => {
            let mut filtered = group.clone();
            let mut index = 0;
            while index < filtered.children.len() {
                if let Some(child) = filter_tree(&filtered.children[index], query, config) {
                    filtered.children[index] = child;
                    index += 1;
                } else {
                    filtered.take_child(index);
                }
            }
            Some(Node::Group(filtered))
        }
    }
}

impl Configuration {
    /// Explicit independent snapshot of matching arrangement references. Selector caches
    /// are detached so reconciliation cannot silently restore excluded query results.
    pub fn save_filtered_view(
        &mut self,
        source: &str,
        name: String,
        query: &Query,
    ) -> Result<crate::Id> {
        validate_filter(query)?;
        self.validate()?;
        if name.trim().is_empty() || name.len() > 200 {
            return Err(Error::new(
                ErrorCode::InvalidConfiguration,
                "Filtered View name must contain 1–200 bytes",
                source,
            ));
        }
        let mut copy =
            self.views.get(source).cloned().ok_or_else(|| {
                Error::new(ErrorCode::TargetMissing, "Source View missing", source)
            })?;
        copy.id = crate::new_id("view");
        copy.name = name;
        for root in copy.roots.values_mut() {
            *root = filter_tree(root, query, self)
                .unwrap_or_else(|| Node::Group(crate::Group::new("Empty filtered root".into())));
            detach(root);
            crate::replace_node_ids(root);
        }
        let id = copy.id.clone();
        let mut draft = self.clone();
        draft.views.insert(id.clone(), copy);
        draft.revision += 1;
        draft.validate()?;
        *self = draft;
        Ok(id)
    }
}

fn detach(node: &mut Node) {
    if let Node::Group(group) = node {
        group.membership = None;
        for child in &mut group.children {
            detach(child);
        }
    }
}
