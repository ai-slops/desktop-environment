use crate::{
    Configuration, Error, ErrorCode, Id, Node, Query, Result, Target, View, WindowRef, new_id,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const MAX_CONFIGURATION_BYTES: u64 = 4 * 1024 * 1024;

fn storage(error: impl std::fmt::Display) -> Error {
    Error::new(ErrorCode::StorageFailure, error.to_string(), "configuration")
}
fn invalid(object: &str, message: &str) -> Error {
    Error::new(ErrorCode::InvalidConfiguration, message, object)
}

impl Configuration {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(invalid("version", "Unsupported schema version"));
        }
        let mut ids = BTreeSet::new();
        let mut add = |id: &str| -> Result<()> {
            if id.is_empty() || id.len() > 200 || !ids.insert(id.to_owned()) {
                return Err(invalid(id, "Empty, excessive, or duplicate stable ID"));
            }
            if ids.len() > 4096 {
                return Err(invalid("configuration", "Object budget exceeded"));
            }
            Ok(())
        };
        for (key, workspace) in &self.workspaces {
            if key != &workspace.id {
                return Err(invalid(key, "Map key does not match identity"));
            }
            add(key)?;
            if let Some(view) = &workspace.remembered_view
                && self.views.get(view).is_none_or(|view| view.workspace != *key)
            {
                return Err(invalid(view, "Remembered View is outside Workspace"));
            }
        }
        for (key, window) in &self.windows {
            if key != &window.id {
                return Err(invalid(key, "Map key does not match identity"));
            }
            add(key)?;
        }
        for (key, collection) in &self.collections {
            if key != &collection.id {
                return Err(invalid(key, "Map key does not match identity"));
            }
            add(key)?;
            validate_query(&collection.query, 0)?;
            for id in collection.include.iter().chain(&collection.exclude) {
                if !self.windows.contains_key(id) {
                    return Err(invalid(id, "Unknown Collection member"));
                }
            }
        }
        for (key, view) in &self.views {
            if key != &view.id
                || !self.workspaces.contains_key(&view.workspace)
                || view.roots.is_empty()
            {
                return Err(invalid(key, "Invalid View identity, Workspace, or roots"));
            }
            add(key)?;
            for root in view.roots.values() {
                validate_node(root, &self.windows, &mut add, 0)?;
            }
        }
        for (key, slot) in &self.slots {
            if key != &slot.id {
                return Err(invalid(key, "Map key does not match identity"));
            }
            add(key)?;
            let [x, y, w, h] = slot.region;
            if !slot.region.iter().all(|n| n.is_finite())
                || x < 0.0
                || y < 0.0
                || w <= 0.0
                || h <= 0.0
                || x + w > 1.000_001
                || y + h > 1.000_001
                || slot.display.is_empty()
            {
                return Err(invalid(key, "Slot must be inside a named monitor work area"));
            }
        }
        for (key, composition) in &self.compositions {
            if key != &composition.id {
                return Err(invalid(key, "Map key does not match identity"));
            }
            add(key)?;
            let mut slots = BTreeSet::new();
            for target in &composition.targets {
                self.validate_target(target)?;
                for slot in target.roots.values() {
                    if !slots.insert(slot) {
                        return Err(invalid(slot, "Composition binds a slot more than once"));
                    }
                }
            }
        }
        let mut numbers = BTreeSet::new();
        for shortcut in &self.shortcuts {
            if !(1..=9).contains(&shortcut.number) || !numbers.insert(shortcut.number) {
                return Err(invalid("shortcut", "Shortcut must use a unique number 1–9"));
            }
            self.validate_target(&shortcut.target)?;
        }
        if ids.len() > 4096 {
            return Err(invalid("configuration", "Object budget exceeded"));
        }
        Ok(())
    }

    pub fn validate_target(&self, target: &Target) -> Result<()> {
        let view = self
            .views
            .get(&target.view)
            .ok_or_else(|| invalid(&target.view, "Unknown target View"))?;
        if target.roots.is_empty() {
            return Err(invalid(&target.view, "A target needs an explicit root-to-slot mapping"));
        }
        let mut unique = BTreeSet::new();
        for (root, slot) in &target.roots {
            if !view.roots.contains_key(root)
                || !self.slots.contains_key(slot)
                || !unique.insert(slot)
            {
                return Err(invalid(slot, "Invalid or duplicate target root/slot"));
            }
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(storage(error)),
            Ok(metadata) if metadata.len() > MAX_CONFIGURATION_BYTES => {
                return Err(invalid("configuration", "Configuration size budget exceeded"));
            }
            Ok(_) => {}
        }
        let file = std::fs::File::open(path).map_err(storage)?;
        let config: Self = serde_json::from_reader(file).map_err(storage)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate before replacing; the previous valid document is retained as a known-good backup.
    #[allow(clippy::items_after_statements)] // Formula validation is local to the atomic commit boundary.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        fn formulas(node: &Node) -> Result<()> {
            match node {
                Node::Placement(placement) => {
                    for preference in placement.preferences.values() {
                        for expression in [&preference.width_formula, &preference.height_formula]
                            .into_iter()
                            .flatten()
                        {
                            crate::validate_property_formula(expression, 1.0, 65_536.0, false)?;
                        }
                    }
                }
                Node::Group(group) => {
                    crate::validate_property_formula(&group.gap, 0.0, 4096.0, false)?;
                    crate::validate_property_formula(&group.columns, 1.0, 256.0, true)?;
                    for child in &group.children {
                        formulas(child)?;
                    }
                }
            }
            Ok(())
        }
        for view in self.views.values() {
            for root in view.roots.values() {
                formulas(root)?;
            }
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(storage)?;
        if bytes.len() as u64 > MAX_CONFIGURATION_BYTES {
            return Err(invalid("configuration", "Configuration size budget exceeded"));
        }
        if path.exists() {
            Self::load(path)?;
            atomic_write(&backup_path(path), &std::fs::read(path).map_err(storage)?)?;
        }
        atomic_write(path, &bytes)
    }

    /// Independent structural copy. Resource references remain shared intentionally.
    pub fn copy_view(&mut self, id: &str, name: String) -> Result<Id> {
        let mut view = self.views.get(id).cloned().ok_or_else(|| invalid(id, "View missing"))?;
        view.id = new_id("view");
        view.name = name;
        for root in view.roots.values_mut() {
            replace_node_ids(root);
        }
        let id = view.id.clone();
        self.views.insert(id.clone(), view);
        self.revision += 1;
        Ok(id)
    }

    /// Only explicitly addressed properties are saved; formulas remain authored rules.
    pub fn save_properties(
        &mut self,
        view: &str,
        placement: &str,
        context: &str,
        position: Option<[f64; 2]>,
        size: Option<[f64; 2]>,
    ) -> Result<()> {
        if position
            .is_some_and(|position| !position.iter().all(|n| n.is_finite() && n.abs() <= 65_536.0))
            || size.is_some_and(|size| {
                !size.iter().all(|n| n.is_finite() && *n > 0.0 && *n <= 65_536.0)
            })
        {
            return Err(invalid(placement, "Invalid addressed geometry"));
        }
        let view = self.views.get_mut(view).ok_or_else(|| invalid(view, "View missing"))?;
        let placement = view
            .roots
            .values_mut()
            .find_map(|root| root.placement_mut(placement))
            .ok_or_else(|| invalid(placement, "Placement missing"))?;
        let preference = placement.preferences.entry(context.into()).or_default();
        if let Some(position) = position {
            preference.position_override = Some(position);
        }
        if let Some(size) = size {
            preference.size_override = Some(size);
        }
        self.revision += 1;
        self.validate()
    }
}

fn replace_node_ids(node: &mut Node) {
    match node {
        Node::Placement(placement) => placement.id = new_id("placement"),
        Node::Group(group) => {
            group.id = new_id("group");
            for child in &mut group.children {
                replace_node_ids(child);
            }
        }
    }
}

fn validate_query(query: &Query, depth: usize) -> Result<()> {
    if depth > 32 {
        return Err(invalid("query", "Query depth budget exceeded"));
    }
    match query {
        Query::Not(query) => validate_query(query, depth + 1)?,
        Query::And(queries) | Query::Or(queries) => {
            if queries.len() > 256 {
                return Err(invalid("query", "Query width budget exceeded"));
            }
            for query in queries {
                validate_query(query, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_node(
    node: &Node,
    windows: &std::collections::BTreeMap<Id, WindowRef>,
    add: &mut impl FnMut(&str) -> Result<()>,
    depth: usize,
) -> Result<()> {
    if depth > 32 {
        return Err(invalid(node.id(), "Group depth budget exceeded"));
    }
    add(node.id())?;
    match node {
        Node::Placement(placement) => {
            if !windows.contains_key(&placement.window) {
                return Err(invalid(&placement.window, "Unknown Window reference"));
            }
            if placement.minimum_client.is_some_and(|size| {
                !size.iter().all(|n| n.is_finite() && *n > 0.0 && *n <= 65_536.0)
            }) {
                return Err(invalid(&placement.id, "Invalid minimum client size"));
            }
            for preference in placement.preferences.values() {
                for position in
                    [preference.position, preference.position_override].into_iter().flatten()
                {
                    if !position.iter().all(|n| n.is_finite() && n.abs() <= 65_536.0) {
                        return Err(invalid(&placement.id, "Invalid logical position"));
                    }
                }
                for size in
                    [preference.client_size, preference.size_override, placement.minimum_client]
                        .into_iter()
                        .flatten()
                {
                    if !size.iter().all(|n| n.is_finite() && *n > 0.0 && *n <= 65_536.0) {
                        return Err(invalid(&placement.id, "Invalid logical size"));
                    }
                }
            }
        }
        Node::Group(group) => {
            if group.children.len() > 256 {
                return Err(invalid(&group.id, "Group child budget exceeded"));
            }
            for ratios in std::iter::once(&group.ratios)
                .chain(group.variants.iter().map(|variant| &variant.ratios))
            {
                if !ratios
                    .iter()
                    .all(|ratio| ratio.is_finite() && *ratio > 0.0 && *ratio <= 1_000_000.0)
                {
                    return Err(invalid(&group.id, "Split ratios must be finite and positive"));
                }
            }
            let mut variant_ids = BTreeSet::new();
            for variant in &group.variants {
                if variant.id.is_empty()
                    || variant.id == "base"
                    || !variant_ids.insert(&variant.id)
                    || !variant.hysteresis.is_finite()
                    || variant.hysteresis < 0.0
                    || ![variant.below_width, variant.below_height]
                        .into_iter()
                        .flatten()
                        .all(|n| n.is_finite() && n > 0.0)
                {
                    return Err(invalid(&group.id, "Invalid responsive variant"));
                }
            }
            for child in &group.children {
                validate_node(child, windows, add, depth + 1)?;
            }
        }
    }
    Ok(())
}

#[must_use]
pub fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("json.bak")
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(storage)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(storage)?;
    temporary.write_all(bytes).map_err(storage)?;
    temporary.as_file().sync_all().map_err(storage)?;
    temporary.persist(path).map_err(|error| storage(format!("{}: {error}", path.display())))?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutPackage {
    pub version: u32,
    pub language_version: u32,
    pub id: Id,
    pub name: String,
    pub required_roles: Vec<String>,
    pub roots: std::collections::BTreeMap<String, Node>,
}

impl LayoutPackage {
    /// No live handles, titles, tags, application hints, or display identifiers are exported.
    #[must_use]
    pub fn from_view(view: &View) -> Self {
        fn redact(node: &mut Node, roles: &mut BTreeSet<String>) {
            match node {
                Node::Placement(placement) => {
                    roles.insert(placement.role.clone());
                    placement.window = placement.role.clone();
                    placement.preferences.clear();
                }
                Node::Group(group) => {
                    group.name = "Group".into();
                    for child in &mut group.children {
                        redact(child, roles);
                    }
                }
            }
        }
        let mut roots = view.roots.clone();
        let mut roles = BTreeSet::new();
        for root in roots.values_mut() {
            redact(root, &mut roles);
        }
        Self {
            version: 1,
            language_version: 1,
            id: new_id("package"),
            name: "Layout package".into(),
            required_roles: roles.into_iter().collect(),
            roots,
        }
    }

    /// Explicit role mapping is mandatory. Import installs a copy and never applies windows.
    #[allow(clippy::items_after_statements)] // The mapping helper is local to the installation transaction.
    pub fn install(
        &self,
        config: &mut Configuration,
        workspace: &str,
        mappings: &std::collections::BTreeMap<String, Id>,
    ) -> Result<Id> {
        if self.version != 1 || self.language_version != 1 {
            return Err(invalid(&self.id, "Unsupported package/language version"));
        }
        fn inspect(
            node: &Node,
            ids: &mut BTreeSet<Id>,
            roles: &mut BTreeSet<String>,
            depth: usize,
        ) -> Result<()> {
            if depth > 32 || ids.len() >= 4096 || !ids.insert(node.id().into()) {
                return Err(invalid(
                    node.id(),
                    "Package has duplicate IDs or exceeds structure limits",
                ));
            }
            match node {
                Node::Placement(placement) => {
                    if placement.role.is_empty() {
                        return Err(invalid(&placement.id, "Empty role placeholder"));
                    }
                    roles.insert(placement.role.clone());
                }
                Node::Group(group) => {
                    crate::validate_formula(&group.gap)?;
                    crate::validate_formula(&group.columns)?;
                    for child in &group.children {
                        inspect(child, ids, roles, depth + 1)?;
                    }
                }
            }
            Ok(())
        }
        let mut ids = BTreeSet::new();
        let mut roles = BTreeSet::new();
        for root in self.roots.values() {
            inspect(root, &mut ids, &mut roles, 0)?;
        }
        if roles != self.required_roles.iter().cloned().collect() {
            return Err(invalid(&self.id, "Declared roles differ from package placeholders"));
        }
        for role in &self.required_roles {
            if mappings.get(role).is_none_or(|window| !config.windows.contains_key(window)) {
                return Err(invalid(role, "Window role needs explicit mapping"));
            }
        }
        fn map(
            node: &mut Node,
            mappings: &std::collections::BTreeMap<String, Id>,
            depth: usize,
        ) -> Result<()> {
            if depth > 32 {
                return Err(invalid(node.id(), "Package depth exceeded"));
            }
            match node {
                Node::Placement(placement) => {
                    placement.window = mappings
                        .get(&placement.role)
                        .cloned()
                        .ok_or_else(|| invalid(&placement.role, "Unmapped role"))?;
                    placement.preferences.clear();
                }
                Node::Group(group) => {
                    for child in &mut group.children {
                        map(child, mappings, depth + 1)?;
                    }
                }
            }
            Ok(())
        }
        let mut draft = config.clone();
        let id = new_id("view");
        let mut roots = self.roots.clone();
        for root in roots.values_mut() {
            map(root, mappings, 0)?;
            replace_node_ids(root);
        }
        draft.views.insert(
            id.clone(),
            View { id: id.clone(), workspace: workspace.into(), name: self.name.clone(), roots },
        );
        draft.revision += 1;
        draft.validate()?;
        *config = draft;
        Ok(id)
    }
}
