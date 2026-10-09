use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub type Id = String;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    StaleRevision,
    StaleBinding,
    TargetMissing,
    AmbiguousBinding,
    ClaimConflict,
    OutOfScope,
    UnsatisfiableConstraints,
    UnsupportedOperation,
    FormulaInvalid,
    FormulaBudgetExceeded,
    ApplicationTimeout,
    OutputStateUnknown,
    PermissionDenied,
    InvalidConfiguration,
    StorageFailure,
}

#[derive(Clone, Debug, thiserror::Error, Serialize, Deserialize)]
#[error("{code:?}: {message}")]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    pub objects: Vec<Id>,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>, object: impl Into<Id>) -> Self {
        Self { code, message: message.into(), objects: vec![object.into()] }
    }
}

/// Opaque identity independent of tree paths and enumeration order.
#[must_use]
pub fn new_id(prefix: &str) -> Id {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!(
        "{prefix}-{:x}-{nanos:x}-{:x}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Physical screen/frame pixels; negative virtual-screen coordinates are valid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn validate(self) -> Result<()> {
        if self.x.unsigned_abs() > 1_000_000
            || self.y.unsigned_abs() > 1_000_000
            || self.width <= 0
            || self.height <= 0
            || self.width > 65_536
            || self.height > 65_536
            || self.x.checked_add(self.width).is_none()
            || self.y.checked_add(self.height).is_none()
        {
            return Err(Error::new(
                ErrorCode::UnsatisfiableConstraints,
                "Invalid physical rectangle",
                "geometry",
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn contains(self, other: Self) -> bool {
        i64::from(other.x) >= i64::from(self.x)
            && i64::from(other.y) >= i64::from(self.y)
            && i64::from(other.x) + i64::from(other.width)
                <= i64::from(self.x) + i64::from(self.width)
            && i64::from(other.y) + i64::from(other.height)
                <= i64::from(self.y) + i64::from(self.height)
    }

    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        i64::from(self.x) < i64::from(other.x) + i64::from(other.width)
            && i64::from(other.x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(self.y) < i64::from(other.y) + i64::from(other.height)
            && i64::from(other.y) < i64::from(self.y) + i64::from(self.height)
    }
}

/// Relative to the allocated group, in 96-DPI logical units. Size is client content.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preference {
    pub position: Option<[f64; 2]>,
    pub client_size: Option<[f64; 2]>,
    pub width_formula: Option<String>,
    pub height_formula: Option<String>,
    pub position_override: Option<[f64; 2]>,
    pub size_override: Option<[f64; 2]>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
// These constraints are independent dimensions, not mutually exclusive states.
#[allow(clippy::struct_excessive_bools)]
pub struct Protection {
    pub geometry_lock: bool,
    pub maintain_visible: bool,
    pub keep_monitor: bool,
    pub prohibit_focus: bool,
}
impl Protection {
    #[must_use]
    pub const fn union(&self, other: &Self) -> Self {
        Self {
            geometry_lock: self.geometry_lock || other.geometry_lock,
            maintain_visible: self.maintain_visible || other.maintain_visible,
            keep_monitor: self.keep_monitor || other.keep_monitor,
            prohibit_focus: self.prohibit_focus || other.prohibit_focus,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputProtection {
    #[default]
    None,
    RequireVerifiedPrivate,
    FreezeWhileLinkedOrUnknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityProfile {
    pub allow_move: bool,
    pub allow_resize: bool,
    pub allow_show_state: bool,
}
impl Default for CapabilityProfile {
    fn default() -> Self {
        Self { allow_move: true, allow_resize: true, allow_show_state: false }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tag {
    pub name: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowRef {
    pub id: Id,
    pub alias: String,
    pub tags: Vec<Tag>,
    pub application_hint: Option<String>,
    /// Hide is opt-in per reference after compatibility testing, not inferred from the executable.
    pub allow_hide: bool,
    pub protection: Protection,
    #[serde(default)]
    pub output_protection: OutputProtection,
    #[serde(default)]
    pub capabilities: CapabilityProfile,
}

impl WindowRef {
    #[must_use]
    pub fn unbound(id: Id, alias: String) -> Self {
        Self {
            id,
            alias,
            tags: Vec::new(),
            application_hint: None,
            allow_hide: false,
            protection: Protection::default(),
            output_protection: OutputProtection::None,
            capabilities: CapabilityProfile::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Truth {
    Yes,
    No,
    Unknown,
}

impl Truth {
    #[must_use]
    pub const fn invert(self) -> Self {
        match self {
            Self::Yes => Self::No,
            Self::No => Self::Yes,
            Self::Unknown => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    All,
    Tag(String),
    Application(String),
    Alias(String),
    Private,
    Not(Box<Self>),
    And(Vec<Self>),
    Or(Vec<Self>),
}

impl Query {
    #[must_use]
    pub fn matches(&self, window: &WindowRef) -> Truth {
        match self {
            Self::All => Truth::Yes,
            Self::Alias(alias) => {
                if window.alias.contains(alias) {
                    Truth::Yes
                } else {
                    Truth::No
                }
            }
            Self::Tag(tag) => {
                if window.tags.iter().any(|item| &item.name == tag) {
                    Truth::Yes
                } else {
                    Truth::No
                }
            }
            Self::Application(app) => window
                .application_hint
                .as_ref()
                .map_or(Truth::Unknown, |value| if value == app { Truth::Yes } else { Truth::No }),
            Self::Private => Truth::Unknown,
            Self::Not(query) => query.matches(window).invert(),
            Self::And(queries) => {
                let results: Vec<_> = queries.iter().map(|query| query.matches(window)).collect();
                if results.contains(&Truth::No) {
                    Truth::No
                } else if results.contains(&Truth::Unknown) {
                    Truth::Unknown
                } else {
                    Truth::Yes
                }
            }
            Self::Or(queries) => {
                let results: Vec<_> = queries.iter().map(|query| query.matches(window)).collect();
                if results.contains(&Truth::Yes) {
                    Truth::Yes
                } else if results.contains(&Truth::Unknown) {
                    Truth::Unknown
                } else {
                    Truth::No
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collection {
    pub id: Id,
    pub name: String,
    pub query: Query,
    pub include: BTreeSet<Id>,
    pub exclude: BTreeSet<Id>,
}

impl Collection {
    #[must_use]
    pub fn selects(&self, window: &WindowRef) -> bool {
        !self.exclude.contains(&window.id)
            && (self.include.contains(&window.id) || self.query.matches(window) == Truth::Yes)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub id: Id,
    pub window: Id,
    pub role: String,
    pub preferences: BTreeMap<String, Preference>,
    pub minimum_client: Option<[f64; 2]>,
    #[serde(default)]
    pub protection: Protection,
    #[serde(default)]
    pub default_preference: Preference,
}

impl Placement {
    #[must_use]
    pub fn new(window: Id, role: String) -> Self {
        Self {
            id: new_id("placement"),
            window,
            role,
            preferences: BTreeMap::new(),
            minimum_client: None,
            protection: Protection::default(),
            default_preference: Preference::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    Horizontal,
    Vertical,
    Grid,
    Flow,
    #[default]
    Free,
    SemanticTabs,
    ResponsiveTabs,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupPreservation {
    #[default]
    Fill,
    Outer,
    Children,
    Arrangement,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub id: Id,
    pub below_width: Option<f64>,
    pub below_height: Option<f64>,
    pub hysteresis: f64,
    pub strategy: Strategy,
    pub ratios: Vec<f64>,
    #[serde(default)]
    pub condition: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub id: Id,
    pub name: String,
    pub strategy: Strategy,
    pub children: Vec<Node>,
    pub ratios: Vec<f64>,
    pub gap: String,
    pub columns: String,
    pub variants: Vec<Variant>,
    pub preserve_child_sizes: bool,
    #[serde(default)]
    pub membership: Option<Box<Membership>>,
    #[serde(default)]
    pub protection: Protection,
    #[serde(default)]
    pub preservation: GroupPreservation,
    #[serde(default)]
    pub allowed_fallbacks: Vec<Strategy>,
    #[serde(default)]
    pub alignment: Alignment,
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
    #[serde(default)]
    pub sort_formula: Option<String>,
}

impl Group {
    #[must_use]
    pub fn new(name: String) -> Self {
        Self {
            id: new_id("group"),
            name,
            strategy: Strategy::Grid,
            children: Vec::new(),
            ratios: Vec::new(),
            gap: "8".into(),
            columns: "max(1, floor(count ^ 0.5))".into(),
            variants: Vec::new(),
            preserve_child_sizes: false,
            membership: None,
            protection: Protection::default(),
            preservation: GroupPreservation::Fill,
            allowed_fallbacks: Vec::new(),
            alignment: Alignment::Start,
            parameters: BTreeMap::new(),
            sort_formula: None,
        }
    }
}

/// Selector-generated leaves keep their identities/preferences when temporarily excluded.
/// Membership is reconciled in a draft and explicitly accepted, never during native callbacks.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Membership {
    pub collection: Id,
    pub role: String,
    pub generated: BTreeMap<Id, Id>,
    pub retired: BTreeMap<Id, Placement>,
    #[serde(default)]
    pub include: BTreeSet<Id>,
    #[serde(default)]
    pub exclude: BTreeSet<Id>,
    #[serde(default)]
    pub weights: BTreeMap<Id, crate::ChildWeights>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    Group(Group),
    Placement(Placement),
}

impl Node {
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&Self> {
        if self.id() == id {
            return Some(self);
        }
        match self {
            Self::Placement(_) => None,
            Self::Group(group) => group.children.iter().find_map(|child| child.find(id)),
        }
    }

    pub fn group_mut(&mut self, id: &str) -> Option<&mut Group> {
        match self {
            Self::Placement(_) => None,
            Self::Group(group) => {
                if group.id == id {
                    Some(group)
                } else {
                    group.children.iter_mut().find_map(|child| child.group_mut(id))
                }
            }
        }
    }
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Group(group) => &group.id,
            Self::Placement(placement) => &placement.id,
        }
    }

    pub fn placements<'a>(&'a self, output: &mut Vec<&'a Placement>) {
        match self {
            Self::Placement(placement) => output.push(placement),
            Self::Group(group) => {
                for child in &group.children {
                    child.placements(output);
                }
            }
        }
    }

    pub fn placement_mut(&mut self, id: &str) -> Option<&mut Placement> {
        match self {
            Self::Placement(placement) => (placement.id == id).then_some(placement),
            Self::Group(group) => {
                group.children.iter_mut().find_map(|child| child.placement_mut(id))
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub id: Id,
    pub workspace: Id,
    pub name: String,
    pub roots: BTreeMap<String, Node>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub id: Id,
    pub name: String,
    pub remembered_view: Option<Id>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplaySlot {
    pub id: Id,
    pub name: String,
    pub display: Id,
    /// Fraction of monitor work area: x, y, width, height. Does not depend on monitor order.
    pub region: [f64; 4],
    pub designated_public: bool,
    /// Explicit ordered display choices. Fallback never rewrites original preferences.
    #[serde(default)]
    pub fallback_displays: Vec<Id>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub view: Id,
    pub roots: BTreeMap<String, Id>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Composition {
    pub id: Id,
    pub name: String,
    pub targets: Vec<Target>,
    #[serde(default)]
    pub protection: Protection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shortcut {
    pub number: u32,
    pub target: Target,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub version: u32,
    pub revision: u64,
    pub workspaces: BTreeMap<Id, Workspace>,
    pub windows: BTreeMap<Id, WindowRef>,
    pub collections: BTreeMap<Id, Collection>,
    pub views: BTreeMap<Id, View>,
    pub slots: BTreeMap<Id, DisplaySlot>,
    pub compositions: BTreeMap<Id, Composition>,
    pub shortcuts: Vec<Shortcut>,
}

impl Default for Configuration {
    fn default() -> Self {
        let workspace = Workspace {
            id: new_id("workspace"),
            name: "기본 작업 공간".into(),
            remembered_view: None,
        };
        let view = View {
            id: new_id("view"),
            workspace: workspace.id.clone(),
            name: "기본 배치".into(),
            roots: BTreeMap::from([("main".into(), Node::Group(Group::new("기본 그룹".into())))]),
        };
        Self {
            version: 1,
            revision: 0,
            workspaces: BTreeMap::from([(workspace.id.clone(), workspace)]),
            views: BTreeMap::from([(view.id.clone(), view)]),
            windows: BTreeMap::new(),
            collections: BTreeMap::new(),
            slots: BTreeMap::new(),
            compositions: BTreeMap::new(),
            shortcuts: Vec::new(),
        }
    }
}

/// Native handles never appear in authored configuration or exported packages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub handle: u64,
    pub process: u32,
    pub process_started: u64,
    pub session: u32,
    pub generation: u64,
    pub token_property: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowState {
    #[default]
    Normal,
    Minimized,
    Maximized,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
// Observed capabilities and visibility are independent platform evidence.
#[allow(clippy::struct_excessive_bools)]
pub struct ObservedWindow {
    pub binding: Binding,
    pub frame: Rect,
    pub client: [i32; 2],
    pub dpi: u32,
    pub display: Id,
    pub visible: bool,
    pub show_state: ShowState,
    pub can_move: bool,
    pub can_resize: bool,
    pub can_hide: bool,
    pub has_owned_dialog: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Display {
    pub id: Id,
    pub name: String,
    pub work_area: Rect,
    pub dpi: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub topology_revision: u64,
    pub displays: BTreeMap<Id, Display>,
    pub windows: BTreeMap<Id, ObservedWindow>,
    pub focused: Option<Id>,
    pub now_ms: u64,
}

#[must_use]
pub fn context_key(slot: &DisplaySlot, variant: &str) -> String {
    // JSON array encoding avoids collisions when user IDs include punctuation.
    serde_json::to_string(&[&slot.id, &slot.display, variant]).unwrap_or_default()
}
