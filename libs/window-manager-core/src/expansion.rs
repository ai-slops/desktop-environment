use crate::{
    Configuration, Error, ErrorCode, Id, Node, Rect, Request, Result, Runtime, Snapshot,
    resolve_slot, slot_bounds,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpansionArea {
    Group,
    Slot,
    Monitor,
    Slots,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expansion {
    pub node: Id,
    pub area: ExpansionArea,
    pub borrow_slots: BTreeSet<Id>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpansionMemory {
    pub selected_tabs: BTreeMap<Id, Id>,
    pub variants: BTreeMap<Id, Id>,
    pub borrowed: BTreeMap<Id, crate::Presentation>,
}

pub struct ExpansionLayout<'a> {
    pub node: &'a Node,
    pub area: Rect,
    pub immutable: BTreeSet<Id>,
    pub memory: ExpansionMemory,
}

fn invalid(object: &str, message: &str) -> Error {
    Error::new(ErrorCode::UnsatisfiableConstraints, message, object)
}

pub fn validate_expansion_scope(
    config: &Configuration,
    request: &Request,
    mapped: &BTreeSet<Id>,
) -> Result<()> {
    let mut authorized = mapped.clone();
    if !mapped.is_disjoint(&request.release)
        || request.release.iter().any(|id| !config.slots.contains_key(id))
    {
        return Err(Error::new(
            ErrorCode::OutOfScope,
            "Released slots must be separate existing slots",
            &request.id,
        ));
    }
    authorized.extend(request.release.iter().cloned());
    if let Some(expansion) = &request.expansion {
        if mapped.len() != 1
            || !mapped.is_disjoint(&expansion.borrow_slots)
            || expansion.borrow_slots.iter().any(|id| !config.slots.contains_key(id))
        {
            return Err(Error::new(
                ErrorCode::OutOfScope,
                "Expansion addresses exactly one root and separate existing borrowed slots",
                &request.id,
            ));
        }
        if matches!(expansion.area, ExpansionArea::Group | ExpansionArea::Slot)
            && !expansion.borrow_slots.is_empty()
        {
            return Err(Error::new(
                ErrorCode::OutOfScope,
                "Local expansion cannot borrow another slot",
                &request.id,
            ));
        }
        authorized.extend(expansion.borrow_slots.iter().cloned());
    }
    if authorized != request.scope {
        return Err(Error::new(
            ErrorCode::OutOfScope,
            "Scope must explicitly include every mapped and borrowed slot",
            &request.id,
        ));
    }
    Ok(())
}

fn parent<'a>(node: &'a Node, target: &str) -> Option<&'a crate::Group> {
    let Node::Group(group) = node else {
        return None;
    };
    if group.children.iter().any(|child| child.id() == target) {
        return Some(group);
    }
    group.children.iter().find_map(|child| parent(child, target))
}

/// Expansion changes a Visit and only explicitly addressed native domains, never the authored tree.
#[allow(clippy::too_many_lines)] // Each distinct expansion policy validates its own physical authority.
pub fn expansion_layout<'a>(
    config: &Configuration,
    runtime: &Runtime,
    snapshot: &Snapshot,
    request: &Request,
    slot: &str,
    root: &'a Node,
    original_area: Rect,
) -> Result<Option<ExpansionLayout<'a>>> {
    let Some(expansion) = &request.expansion else {
        return Ok(None);
    };
    let target = root.find(&expansion.node).ok_or_else(|| {
        Error::new(
            ErrorCode::OutOfScope,
            "Expanded node is outside the requested root",
            &expansion.node,
        )
    })?;
    let resolved = resolve_slot(config, &config.slots[slot], snapshot)?;
    let mut memory = ExpansionMemory {
        selected_tabs: BTreeMap::new(),
        variants: BTreeMap::new(),
        borrowed: BTreeMap::new(),
    };
    if let Some(active) = runtime.presentations.get(slot) {
        if active.expansion.as_ref() != Some(expansion) && active.expansion.is_some() {
            return Err(invalid(slot, "Collapse the existing expansion before changing its scope"));
        }
        memory.selected_tabs = active.selected_tabs.clone();
        memory.variants = active.variants.clone();
    }
    for borrowed in &expansion.borrow_slots {
        if let Some(active) = runtime.presentations.get(borrowed) {
            if active.expansion.is_some() {
                return Err(invalid(
                    borrowed,
                    "Borrowing an already expanded presentation is unsupported",
                ));
            }
            memory.borrowed.insert(borrowed.clone(), active.clone());
        }
    }
    let mut immutable = BTreeSet::new();
    let area = match expansion.area {
        ExpansionArea::Group => {
            let parent = parent(root, &expansion.node).ok_or_else(|| {
                invalid(&expansion.node, "The root has no parent Group; expand to its Slot instead")
            })?;
            let active = runtime.presentations.get(slot).ok_or_else(|| {
                invalid(slot, "Focus within a Group requires an active presentation")
            })?;
            let area = active
                .group_bounds
                .get(&parent.id)
                .copied()
                .ok_or_else(|| invalid(&parent.id, "Parent Group allocation is unavailable"))?;
            // Own stable IDs rather than references into a temporary clone.
            let parent_windows: BTreeSet<Id> = parent
                .children
                .iter()
                .flat_map(|child| {
                    let mut leaves = Vec::new();
                    child.placements(&mut leaves);
                    leaves.into_iter().map(|leaf| leaf.window.clone()).collect::<Vec<_>>()
                })
                .collect();
            if !runtime
                .claims
                .iter()
                .any(|(window, claim)| claim.slot == slot && parent_windows.contains(window))
            {
                return Err(invalid(&parent.id, "Parent Group is not currently presented"));
            }
            for (window, claim) in &runtime.claims {
                if claim.slot == slot && !parent_windows.contains(window) {
                    immutable.insert(window.clone());
                    if snapshot
                        .windows
                        .get(window)
                        .is_some_and(|observed| observed.frame.overlaps(area))
                    {
                        return Err(invalid(
                            window,
                            "An unchanged sibling occupies the parent Group allocation",
                        ));
                    }
                }
            }
            area
        }
        ExpansionArea::Slot => original_area,
        ExpansionArea::Monitor => {
            let area = snapshot.displays[&resolved.display].work_area;
            for (other, other_slot) in &config.slots {
                if other != slot
                    && resolve_slot(config, other_slot, snapshot)
                        .and_then(|slot| slot_bounds(&slot, snapshot))
                        .is_ok_and(|bounds| bounds.overlaps(area))
                    && !expansion.borrow_slots.contains(other)
                {
                    return Err(Error::new(
                        ErrorCode::OutOfScope,
                        "Monitor expansion must explicitly include every intersected slot",
                        other,
                    ));
                }
            }
            area
        }
        ExpansionArea::Slots => {
            let mut rectangles = vec![original_area];
            for borrowed in &expansion.borrow_slots {
                let other = resolve_slot(config, &config.slots[borrowed], snapshot)?;
                if other.display != resolved.display {
                    return Err(invalid(
                        borrowed,
                        "One interactive expansion requires a single display/DPI",
                    ));
                }
                rectangles.push(slot_bounds(&other, snapshot)?);
            }
            let x = rectangles.iter().map(|rect| rect.x).min().unwrap_or(original_area.x);
            let y = rectangles.iter().map(|rect| rect.y).min().unwrap_or(original_area.y);
            let right = rectangles
                .iter()
                .map(|rect| i64::from(rect.x) + i64::from(rect.width))
                .max()
                .unwrap_or_default();
            let bottom = rectangles
                .iter()
                .map(|rect| i64::from(rect.y) + i64::from(rect.height))
                .max()
                .unwrap_or_default();
            let area = Rect {
                x,
                y,
                width: i32::try_from(right - i64::from(x))
                    .map_err(|_| invalid(slot, "Expanded width exceeds limits"))?,
                height: i32::try_from(bottom - i64::from(y))
                    .map_err(|_| invalid(slot, "Expanded height exceeds limits"))?,
            };
            if rectangles
                .iter()
                .map(|rect| i64::from(rect.width) * i64::from(rect.height))
                .sum::<i64>()
                != i64::from(area.width) * i64::from(area.height)
                || rectangles.iter().enumerate().any(|(index, rect)| {
                    rectangles[..index].iter().any(|other| other.overlaps(*rect))
                })
            {
                return Err(invalid(
                    slot,
                    "Borrowed slots must form one contiguous nonoverlapping rectangle",
                ));
            }
            area
        }
    };
    area.validate()?;
    if !config.slots[slot].designated_public
        && config.slots.values().filter(|other| other.designated_public).any(|other| {
            resolve_slot(config, other, snapshot)
                .and_then(|resolved| slot_bounds(&resolved, snapshot))
                .is_ok_and(|bounds| bounds.overlaps(area))
        })
    {
        return Err(Error::new(
            ErrorCode::PermissionDenied,
            "Private expansion would enter a public-designated region",
            slot,
        ));
    }
    Ok(Some(ExpansionLayout { node: target, area, immutable, memory }))
}

impl Runtime {
    pub fn collapse_request(&self, config: &Configuration, slot: &str) -> Result<Request> {
        let active =
            self.presentations.get(slot).ok_or_else(|| invalid(slot, "Presentation missing"))?;
        let memory = active
            .before_expansion
            .as_ref()
            .ok_or_else(|| invalid(slot, "No expansion to collapse"))?;
        let mut request = Request::open(
            config,
            crate::Target {
                view: active.view.clone(),
                roots: BTreeMap::from([(active.root.clone(), slot.into())]),
            },
        );
        request.selected_tabs = memory.selected_tabs.clone();
        request.filter.clone_from(&active.filter);
        for (slot, presentation) in &memory.borrowed {
            request.targets.push(crate::Target {
                view: presentation.view.clone(),
                roots: BTreeMap::from([(presentation.root.clone(), slot.clone())]),
            });
            request.scope.insert(slot.clone());
            request.selected_tabs.extend(presentation.selected_tabs.clone());
        }
        if let Some(expansion) = &active.expansion {
            for borrowed in &expansion.borrow_slots {
                request.scope.insert(borrowed.clone());
                if !memory.borrowed.contains_key(borrowed) {
                    request.release.insert(borrowed.clone());
                }
            }
        }
        Ok(request)
    }
}
