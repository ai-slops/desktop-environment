use crate::{Configuration, Error, ErrorCode, Id, Node, Result, Target, View, WindowRef, new_id};
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
            crate::validate_filter(&collection.query)?;
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
                validate_membership_sources(root, self)?;
                crate::validate_rules(root, &BTreeSet::new())?;
            }
        }
        for (key, slot) in &self.slots {
            if key != &slot.id {
                return Err(invalid(key, "Map key does not match identity"));
            }
            add(key)?;
            validate_slot(slot)?;
        }
        for (key, composition) in &self.compositions {
            if key != &composition.id {
                return Err(invalid(key, "Map key does not match identity"));
            }
            add(key)?;
            if composition.targets.is_empty() || composition.targets.len() > 128 {
                return Err(invalid(key, "Composition needs 1–128 explicit targets"));
            }
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
            shortcut.target.validate(self)?;
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
        let config: Self = crate::read_json(path)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate before replacing; the previous valid document is retained as a known-good backup.
    #[allow(clippy::items_after_statements)] // Formula validation is local to the atomic commit boundary.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
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
        let baseline = placement.default_preference.clone();
        let preference = placement.preferences.entry(context.into()).or_insert(baseline);
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

pub fn replace_node_ids(node: &mut Node) {
    match node {
        Node::Placement(placement) => placement.id = new_id("placement"),
        Node::Group(group) => {
            group.id = new_id("group");
            for child in &mut group.children {
                replace_node_ids(child);
            }
            if let Some(membership) = &mut group.membership {
                for placement in membership.retired.values_mut() {
                    placement.id = new_id("placement");
                }
                for (window, id) in &mut membership.generated {
                    if let Some(placement) = membership.retired.get(window) { id.clone_from(&placement.id); }
                    else if let Some(Node::Placement(placement)) = group.children.iter().find(|child| matches!(child, Node::Placement(placement) if &placement.window == window)) { id.clone_from(&placement.id); }
                }
            }
        }
    }
}

#[allow(clippy::too_many_lines)] // Recursive validation includes member caches in the same global identity budget.
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
            for preference in
                std::iter::once(&placement.default_preference).chain(placement.preferences.values())
            {
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
            if !(-1_000_000..=1_000_000).contains(&group.rule_priority) {
                return Err(invalid(&group.id, "Rule priority exceeds bounds"));
            }
            if group.allowed_fallbacks.len() > 3
                || group.allowed_fallbacks.iter().any(|strategy| {
                    !matches!(strategy, crate::Strategy::Flow | crate::Strategy::ResponsiveTabs)
                })
                || group.strategy == crate::Strategy::SemanticTabs
                    && !group.allowed_fallbacks.is_empty()
            {
                return Err(invalid(
                    &group.id,
                    "Unsupported fallback or semantic alternatives cannot be unfolded",
                ));
            }
            if let Some(membership) = &group.membership {
                if membership.weights.len() > 256
                    || membership.weights.iter().any(|(window, weights)| {
                        !membership.retired.contains_key(window) || !weights.valid()
                    })
                {
                    return Err(invalid(&group.id, "Invalid retired child weights"));
                }
                if membership
                    .include
                    .iter()
                    .chain(&membership.exclude)
                    .any(|id| !windows.contains_key(id))
                {
                    return Err(invalid(&group.id, "Unknown local membership override"));
                }
                if membership.generated.len() > 256 || membership.retired.len() > 256 {
                    return Err(invalid(&group.id, "Membership budget exceeded"));
                }
                for (window, placement) in &membership.retired {
                    if window != &placement.window
                        || membership.generated.get(window) != Some(&placement.id)
                    {
                        return Err(invalid(&group.id, "Retired member identity mismatch"));
                    }
                    validate_node(&Node::Placement(placement.clone()), windows, add, depth + 1)?;
                }
                for (window, id) in &membership.generated {
                    if !membership.retired.contains_key(window) && !group.children.iter().any(|child| matches!(child, Node::Placement(placement) if &placement.id == id && &placement.window == window)) { return Err(invalid(&group.id, "Generated member is missing")); }
                }
            }
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

fn validate_membership_sources(node: &Node, config: &Configuration) -> Result<()> {
    if let Node::Group(group) = node {
        if let Some(membership) = &group.membership
            && !config.collections.contains_key(&membership.collection)
        {
            return Err(invalid(&group.id, "Unknown selector Collection"));
        }
        for child in &group.children {
            validate_membership_sources(child, config)?;
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
    #[serde(default)]
    pub required_parameters: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub supported_strategies: BTreeSet<crate::Strategy>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

fn package_strategies(
    roots: &std::collections::BTreeMap<String, Node>,
) -> BTreeSet<crate::Strategy> {
    fn collect(node: &Node, strategies: &mut BTreeSet<crate::Strategy>) {
        if let Node::Group(group) = node {
            strategies.insert(group.strategy);
            strategies.extend(group.variants.iter().map(|variant| variant.strategy));
            strategies.extend(&group.allowed_fallbacks);
            for child in &group.children {
                collect(child, strategies);
            }
        }
    }
    let mut strategies = BTreeSet::new();
    for root in roots.values() {
        collect(root, &mut strategies);
    }
    strategies
}

impl LayoutPackage {
    /// No live handles, titles, tags, application hints, or display identifiers are exported.
    #[must_use]
    pub fn from_view(view: &View) -> Self {
        fn redact(
            node: &mut Node,
            roles: &mut BTreeSet<String>,
            resources: &mut std::collections::BTreeMap<Id, String>,
        ) {
            match node {
                Node::Placement(placement) => {
                    let role = resources.entry(placement.window.clone()).or_insert_with(|| {
                        let base =
                            if placement.role.is_empty() { "window" } else { &placement.role };
                        let mut candidate = base.to_owned();
                        let mut suffix = 2;
                        while roles.contains(&candidate) {
                            candidate = format!("{base}-{suffix}");
                            suffix += 1;
                        }
                        roles.insert(candidate.clone());
                        candidate
                    });
                    placement.window.clone_from(role);
                    let common = |values: Vec<Option<String>>| {
                        let mut values = values.into_iter();
                        let first = values.next()??;
                        values.all(|value| value.as_ref() == Some(&first)).then_some(first)
                    };
                    placement.default_preference = crate::Preference {
                        width_formula: placement.default_preference.width_formula.clone().or_else(
                            || {
                                common(
                                    placement
                                        .preferences
                                        .values()
                                        .map(|preference| preference.width_formula.clone())
                                        .collect(),
                                )
                            },
                        ),
                        height_formula: placement
                            .default_preference
                            .height_formula
                            .clone()
                            .or_else(|| {
                                common(
                                    placement
                                        .preferences
                                        .values()
                                        .map(|preference| preference.height_formula.clone())
                                        .collect(),
                                )
                            }),
                        ..crate::Preference::default()
                    };
                    placement.preferences.clear();
                }
                Node::Group(group) => {
                    group.name = "Group".into();
                    group.membership = None;
                    for child in &mut group.children {
                        redact(child, roles, resources);
                    }
                }
            }
        }
        let mut roots = view.roots.clone();
        let mut roles = BTreeSet::new();
        let mut resources = std::collections::BTreeMap::new();
        for root in roots.values_mut() {
            redact(root, &mut roles, &mut resources);
            replace_node_ids(root);
        }
        Self {
            version: 1,
            language_version: 1,
            id: new_id("package"),
            name: "Layout package".into(),
            required_roles: roles.into_iter().collect(),
            required_parameters: crate::declared_parameters(&roots),
            supported_strategies: package_strategies(&roots),
            dependencies: vec!["expression-language:1".into()],
            roots,
        }
    }

    #[allow(clippy::items_after_statements)] // Validate package content before resolving any local resource.
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 || self.language_version != 1 {
            return Err(invalid(&self.id, "Unsupported package/language version"));
        }
        if self.id.is_empty()
            || self.id.len() > 200
            || self.name.is_empty()
            || self.name.len() > 200
            || self.roots.is_empty()
        {
            return Err(invalid(&self.id, "Invalid package identity/name/roots"));
        }
        if self.dependencies.len() > 1
            || self.dependencies.iter().any(|dependency| dependency != "expression-language:1")
        {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "Package declares an unavailable dependency; installation has no executable capabilities",
                &self.id,
            ));
        }
        if self.required_parameters != crate::declared_parameters(&self.roots)
            || !self.supported_strategies.is_empty()
                && self.supported_strategies != package_strategies(&self.roots)
        {
            return Err(invalid(
                &self.id,
                "Declared package parameters or strategies differ from content",
            ));
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
                    if placement.window.is_empty() {
                        return Err(invalid(&placement.id, "Empty role placeholder"));
                    }
                    roles.insert(placement.window.clone());
                }
                Node::Group(group) => {
                    if group.membership.is_some() {
                        return Err(invalid(
                            &group.id,
                            "Packages must map roles explicitly; local selectors cannot be imported",
                        ));
                    }

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
            crate::validate_rules(root, &BTreeSet::new())?;
        }
        if roles != self.required_roles.iter().cloned().collect()
            || roles.len() != self.required_roles.len()
        {
            return Err(invalid(&self.id, "Declared roles differ from package placeholders"));
        }
        let windows = roles
            .iter()
            .map(|role| (role.clone(), WindowRef::unbound(role.clone(), role.clone())))
            .collect();
        let mut node_ids = BTreeSet::new();
        let mut add = |id: &str| {
            if id.is_empty() || id.len() > 200 || !node_ids.insert(id.to_owned()) {
                Err(invalid(id, "Invalid package node ID"))
            } else {
                Ok(())
            }
        };
        for root in self.roots.values() {
            validate_node(root, &windows, &mut add, 0)?;
        }
        Ok(())
    }

    /// Explicit role mapping is mandatory. Import installs a copy and never applies windows.
    #[allow(clippy::items_after_statements)] // The mapping helper is local to the installation transaction.
    pub fn install(
        &self,
        config: &mut Configuration,
        workspace: &str,
        mappings: &std::collections::BTreeMap<String, Id>,
    ) -> Result<Id> {
        self.validate()?;
        if self.required_roles.iter().cloned().collect::<BTreeSet<_>>()
            != mappings.keys().cloned().collect()
        {
            return Err(invalid(&self.id, "Mapping must exactly match required roles"));
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
                        .get(&placement.window)
                        .cloned()
                        .ok_or_else(|| invalid(&placement.window, "Unmapped role"))?;
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

fn validate_slot(slot: &crate::DisplaySlot) -> Result<()> {
    if slot.fallback_displays.len() > 8
        || slot
            .fallback_displays
            .iter()
            .any(|id| id.is_empty() || id.len() > 200 || id == &slot.display)
        || slot.fallback_displays.iter().collect::<BTreeSet<_>>().len()
            != slot.fallback_displays.len()
    {
        return Err(invalid(&slot.id, "Invalid/duplicate explicit fallback display"));
    }
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
        return Err(invalid(&slot.id, "Slot must be inside a named monitor work area"));
    }
    Ok(())
}
