#![allow(clippy::float_cmp)] // Exact expected values are part of the formula contract.
use crate::*;
use std::collections::{BTreeMap, BTreeSet};

fn fixture() -> (Configuration, Snapshot, Target) {
    let mut config = Configuration::default();
    let view = config.views.keys().next().cloned().unwrap_or_default();
    config.windows.insert(
        "preview".into(),
        WindowRef {
            id: "preview".into(),
            alias: "Preview".into(),
            tags: vec![],
            application_hint: None,
            allow_hide: true,
            protection: Protection::default(),
            output_protection: OutputProtection::None,
            capabilities: CapabilityProfile::default(),
        },
    );
    let slot = DisplaySlot {
        id: "work".into(),
        name: "Work".into(),
        display: "monitor".into(),
        region: [0.0, 0.0, 1.0, 1.0],
        designated_public: false,
        fallback_displays: Vec::new(),
    };
    let mut placement = Placement::new("preview".into(), "preview".into());
    placement.preferences.insert(
        context_key(&slot, "base"),
        Preference { client_size: Some([800.0, 500.0]), ..Preference::default() },
    );
    if let Some(view) = config.views.get_mut(&view)
        && let Some(Node::Group(group)) = view.roots.get_mut("main")
    {
        group.children.push(Node::Placement(placement));
    }
    config.slots.insert(slot.id.clone(), slot);
    let observed = ObservedWindow {
        binding: Binding {
            handle: 1,
            process: 2,
            process_started: 3,
            session: 1,
            generation: 4,
            token_property: "test".into(),
        },
        frame: Rect { x: 20, y: 30, width: 620, height: 440 },
        client: [600, 400],
        dpi: 96,
        display: "monitor".into(),
        visible: true,
        show_state: ShowState::Normal,
        can_move: true,
        can_resize: true,
        can_hide: true,
        has_owned_dialog: false,
    };
    let snapshot = Snapshot {
        focused: None,
        now_ms: 1000,
        topology_revision: 1,
        displays: BTreeMap::from([(
            "monitor".into(),
            Display {
                id: "monitor".into(),
                name: "Monitor".into(),
                work_area: Rect { x: 0, y: 0, width: 2000, height: 1200 },
                dpi: 96,
            },
        )]),
        windows: BTreeMap::from([("preview".into(), observed)]),
    };
    (config, snapshot, Target { view, roots: BTreeMap::from([("main".into(), "work".into())]) })
}

fn settled_snapshot(plan: &Plan, snapshot: &Snapshot) -> Snapshot {
    let mut settled = snapshot.clone();
    for mutation in &plan.mutations {
        if let Some(observed) = settled.windows.get_mut(&mutation.window) {
            if let Some(frame) = mutation.geometry {
                observed.client[0] += frame.width - observed.frame.width;
                observed.client[1] += frame.height - observed.frame.height;
                observed.frame = frame;
            }
            if let Some(visible) = mutation.visible {
                observed.visible = visible;
            }
            if let Some(state) = mutation.show_state {
                observed.show_state = state;
            }
        }
    }
    settled
}

#[test]
fn native_undo_is_scoped_and_revalidates_new_owners_and_manual_changes() -> Result<()> {
    let (config, snapshot, target) = fixture();
    let prior = Runtime::default();
    let transition = plan(&config, &prior, &snapshot, &Request::open(&config, target))?;
    let after = settled_snapshot(&transition, &snapshot);
    let mut runtime = prior.clone();
    transition.commit(&mut runtime);
    let undo = UndoRecord::capture(&transition, &prior, &after, &runtime);
    runtime.generations.insert("unrelated".into(), 100);
    let reverse = undo.reverse(&config, &runtime, &after)?;
    assert_eq!(reverse.mutations[0].geometry, Some(snapshot.windows["preview"].frame));
    assert!(reverse.mutations.iter().all(|mutation| !mutation.focus));
    let mut moved = after.clone();
    if let Some(window) = moved.windows.get_mut("preview") {
        window.frame.x += 1;
    }
    assert_eq!(
        undo.reverse(&config, &runtime, &moved).err().map(|error| error.code),
        Some(ErrorCode::StaleBinding)
    );
    runtime
        .claims
        .insert("preview".into(), Claim { slot: "other".into(), placement: "new".into() });
    assert_eq!(
        undo.reverse(&config, &runtime, &after).err().map(|error| error.code),
        Some(ErrorCode::ClaimConflict)
    );
    runtime.claims.remove("preview");
    reverse.commit(&mut runtime);
    assert!(runtime.presentations.is_empty());
    assert!(runtime.claims.is_empty());
    assert_eq!(runtime.generations["unrelated"], 100);
    Ok(())
}

#[test]
fn suspended_failure_does_not_block_disjoint_work() -> Result<()> {
    let (config, snapshot, target) = fixture();
    let mut runtime = Runtime::default();
    runtime.suspended.insert("failed".into());
    let transition = plan(&config, &runtime, &snapshot, &Request::open(&config, target.clone()))?;
    let component = transition.component("work", &runtime);
    assert_eq!(component.scope, BTreeSet::from(["work".into()]));
    component.commit(&mut runtime);
    assert!(runtime.suspended.contains("failed"));
    runtime.suspended.insert("work".into());
    assert_eq!(
        plan(&config, &runtime, &snapshot, &Request::open(&config, target))
            .err()
            .map(|error| error.code),
        Some(ErrorCode::PermissionDenied)
    );
    Ok(())
}

#[test]
fn preserved_size_is_visit_local_and_idempotent_recall_keeps_it() -> Result<()> {
    let (config, snapshot, target) = fixture();
    let saved = serde_json::to_vec(&config)
        .map_err(|error| Error::new(ErrorCode::StorageFailure, error.to_string(), "test"))?;
    let mut runtime = Runtime::default();
    let mut request = Request::open(&config, target.clone());
    request.mode = TransitionMode::KeepSize;
    request.retain.insert("preview".into());
    let plan = plan(&config, &runtime, &snapshot, &request)?;
    assert!(plan.mutations.iter().all(|mutation| mutation.move_only));
    assert_eq!(plan.desired["preview"].frame.width, 620);
    plan.commit(&mut runtime);
    assert!(!runtime.presentations["work"].overrides.is_empty());
    let ordinary = crate::plan(&config, &runtime, &snapshot, &Request::open(&config, target))?;
    assert!(ordinary.idempotent);
    assert!(ordinary.mutations.is_empty());
    assert_eq!(
        saved,
        serde_json::to_vec(&config).map_err(|error| Error::new(
            ErrorCode::StorageFailure,
            error.to_string(),
            "test"
        ))?
    );
    Ok(())
}

#[test]
fn restore_reapplies_destination_size_without_mutating_saved_state() -> Result<()> {
    let (config, snapshot, target) = fixture();
    let mut runtime = Runtime::default();
    let mut request = Request::open(&config, target.clone());
    request.mode = TransitionMode::KeepSize;
    request.retain.insert("preview".into());
    plan(&config, &runtime, &snapshot, &request)?.commit(&mut runtime);
    request = Request::open(&config, target);
    request.mode = TransitionMode::Restore;
    let restore = plan(&config, &runtime, &snapshot, &request)?;
    assert_eq!(restore.desired["preview"].frame.width, 820);
    assert!(restore.presentations["work"].overrides.is_empty());
    Ok(())
}

#[test]
fn hard_preserve_too_large_blocks_and_scope_cannot_widen() {
    let (mut config, snapshot, target) = fixture();
    if let Some(slot) = config.slots.get_mut("work") {
        slot.region[2] = 0.2;
    }
    let mut request = Request::open(&config, target);
    request.mode = TransitionMode::KeepSize;
    request.retain.insert("preview".into());
    assert_eq!(
        plan(&config, &Runtime::default(), &snapshot, &request).err().map(|error| error.code),
        Some(ErrorCode::UnsatisfiableConstraints)
    );
    request.scope.insert("game".into());
    assert_eq!(
        plan(&config, &Runtime::default(), &snapshot, &request).err().map(|error| error.code),
        Some(ErrorCode::OutOfScope)
    );
}

#[test]
fn claim_elsewhere_and_stale_binding_are_rejected() -> Result<()> {
    let (config, mut snapshot, target) = fixture();
    let mut runtime = Runtime::default();
    runtime
        .claims
        .insert("preview".into(), Claim { slot: "elsewhere".into(), placement: "other".into() });
    let request = Request::open(&config, target);
    assert_eq!(
        plan(&config, &runtime, &snapshot, &request).err().map(|error| error.code),
        Some(ErrorCode::ClaimConflict)
    );
    runtime.claims.clear();
    let plan = plan(&config, &runtime, &snapshot, &request)?;
    if let Some(observed) = snapshot.windows.get_mut("preview") {
        observed.binding.generation += 1;
    }
    assert_eq!(
        plan.revalidate(&config, &runtime, &snapshot).err().map(|error| error.code),
        Some(ErrorCode::StaleBinding)
    );
    Ok(())
}

#[test]
fn formulas_are_bounded_pure_typed_and_lazy() -> Result<()> {
    let context = BTreeMap::from([("available_width".into(), 900.0), ("count".into(), 16.0)]);
    assert_eq!(evaluate("min(available_width * 0.3, 600)", &context)?, 270.0);
    assert_eq!(
        evaluate("available_width < 1000 && count > 1 ? floor(count ^ 0.5) : 1 / 0", &context)?,
        4.0
    );
    for formula in
        ["1/0", "unknown + 2", "system('run')", "clamp(4,10,2)", "true + 1", "2 ^ 99999", "1 2"]
    {
        assert!(evaluate(formula, &context).is_err(), "{formula}");
    }
    assert_eq!(
        evaluate(&"(".repeat(5000), &context).err().map(|error| error.code),
        Some(ErrorCode::FormulaBudgetExceeded)
    );
    Ok(())
}

#[test]
fn unknown_negative_query_never_proves_public_and_exclusions_win() {
    let (config, _, _) = fixture();
    let window = &config.windows["preview"];
    assert_eq!(Query::Not(Box::new(Query::Private)).matches(window), Truth::Unknown);
    let collection = Collection {
        id: "c".into(),
        name: "c".into(),
        query: Query::All,
        include: BTreeSet::from(["preview".into()]),
        exclude: BTreeSet::from(["preview".into()]),
    };
    assert!(!collection.selects(window));
}

#[test]
fn persistence_backup_invalid_schema_and_property_isolation()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let (mut config, _, target) = fixture();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("config.json");
    config.save(&path)?;
    let copy = config.copy_view(&target.view, "Review".into())?;
    let mut leaves = Vec::new();
    config.views[&copy].roots["main"].placements(&mut leaves);
    let placement = leaves[0].id.clone();
    config.save_properties(&copy, &placement, "context", None, Some([1000.0, 700.0]))?;
    config.save(&path)?;
    assert!(backup_path(&path).is_file());
    assert_eq!(Configuration::load(&path)?.revision, config.revision);
    let mut original = Vec::new();
    config.views[&target.view].roots["main"].placements(&mut original);
    assert!(!original[0].preferences.contains_key("context"));
    std::fs::write(&path, b"{\"version\":9}")?;
    assert!(Configuration::load(&path).is_err());
    assert!(config.save(&path).is_err());
    Ok(())
}

#[test]
fn strict_commands_reject_unknown_fields() {
    let (config, _, target) = fixture();
    let request = Request::open(&config, target);
    let mut json = serde_json::to_value(request).unwrap_or_default();
    json["hidden_scope"] = serde_json::json!(["game"]);
    assert!(serde_json::from_value::<Request>(json).is_err());
}

#[test]
fn packages_require_explicit_mapping_and_have_no_live_identity() -> Result<()> {
    let (mut config, _, target) = fixture();
    let package = LayoutPackage::from_view(&config.views[&target.view]);
    let encoded = serde_json::to_string(&package)
        .map_err(|error| Error::new(ErrorCode::StorageFailure, error.to_string(), "test"))?;
    assert!(!encoded.contains("process_started"));
    let workspace = config.views[&target.view].workspace.clone();
    assert!(package.install(&mut config, &workspace, &BTreeMap::new()).is_err());
    let id = package.install(
        &mut config,
        &workspace,
        &BTreeMap::from([("preview".into(), "preview".into())]),
    )?;
    assert_ne!(id, target.view);
    config.validate()
}

#[test]
fn package_resources_distinguish_duplicate_roles_and_reuse_shared_windows() -> Result<()> {
    let (mut config, _, target) = fixture();
    let mut other = config.windows["preview"].clone();
    other.id = "other".into();
    config.windows.insert(other.id.clone(), other);
    let group = group_mut(&mut config, &target)?;
    group.children.push(Node::Placement(Placement::new("other".into(), "preview".into())));
    group.children.push(Node::Placement(Placement::new("preview".into(), "alternative".into())));
    let package = LayoutPackage::from_view(&config.views[&target.view]);
    assert_eq!(package.required_roles, vec!["preview", "preview-2"]);
    let Node::Group(exported) = &package.roots["main"] else {
        return Err(Error::new(ErrorCode::TargetMissing, "test group", "test"));
    };
    let placeholders: Vec<_> = exported
        .children
        .iter()
        .filter_map(|node| {
            if let Node::Placement(placement) = node {
                Some(placement.window.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(placeholders, vec!["preview", "preview-2", "preview"]);
    let original_ids: BTreeSet<_> =
        group_mut(&mut config, &target)?.children.iter().map(|node| node.id().to_owned()).collect();
    assert!(exported.children.iter().all(|node| !original_ids.contains(node.id())));
    let workspace = config.views[&target.view].workspace.clone();
    let mappings = BTreeMap::from([
        ("preview".into(), "preview".into()),
        ("preview-2".into(), "other".into()),
    ]);
    let installed = package.install(&mut config, &workspace, &mappings)?;
    let Node::Group(imported) = &config.views[&installed].roots["main"] else {
        return Err(Error::new(ErrorCode::TargetMissing, "test group", "test"));
    };
    let resources: Vec<_> = imported
        .children
        .iter()
        .filter_map(|node| {
            if let Node::Placement(placement) = node {
                Some(placement.window.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(resources, vec!["preview", "other", "preview"]);
    let mut extra = mappings;
    extra.insert("unexpected".into(), "preview".into());
    assert!(package.install(&mut config, &workspace, &extra).is_err());
    Ok(())
}

fn group_mut<'a>(config: &'a mut Configuration, target: &Target) -> Result<&'a mut Group> {
    match config.views.get_mut(&target.view).and_then(|view| view.roots.get_mut("main")) {
        Some(Node::Group(group)) => Ok(group),
        _ => Err(Error::new(ErrorCode::TargetMissing, "Fixture group missing", "test")),
    }
}

#[test]
fn semantic_tabs_keep_occurrences_independent_even_when_widened() -> Result<()> {
    let (mut config, snapshot, target) = fixture();
    let group = group_mut(&mut config, &target)?;
    group.strategy = Strategy::SemanticTabs;
    let mut second = group.children[0].clone();
    if let Node::Placement(placement) = &mut second {
        placement.id = new_id("placement");
        for preference in placement.preferences.values_mut() {
            preference.client_size = Some([1000.0, 700.0]);
        }
    }
    let id = second.id().to_owned();
    group.children.push(second);
    let mut request = Request::open(&config, target);
    request
        .selected_tabs
        .insert(group_mut(&mut config, &request.targets[0])?.id.clone(), id.clone());
    let result = plan(&config, &Runtime::default(), &snapshot, &request)?;
    assert_eq!(result.desired.len(), 1);
    assert_eq!(result.desired["preview"].placement, id);
    assert_eq!(result.desired["preview"].frame.width, 1020);
    Ok(())
}

#[test]
fn keep_here_reserves_space_for_surrounding_children() -> Result<()> {
    let (mut config, mut snapshot, target) = fixture();
    let mut other = config.windows["preview"].clone();
    other.id = "tools".into();
    config.windows.insert(other.id.clone(), other);
    let mut observed = snapshot.windows["preview"].clone();
    observed.binding.handle = 9;
    observed.binding.generation = 10;
    observed.frame.x = 1000;
    snapshot.windows.insert("tools".into(), observed);
    let group = group_mut(&mut config, &target)?;
    group.strategy = Strategy::Horizontal;
    group.children.push(Node::Placement(Placement::new("tools".into(), "tools".into())));
    let mut request = Request::open(&config, target);
    request.mode = TransitionMode::KeepHere;
    request.retain.insert("preview".into());
    let result = plan(&config, &Runtime::default(), &snapshot, &request)?;
    assert_eq!(result.desired["preview"].frame, snapshot.windows["preview"].frame);
    assert!(!result.desired["tools"].frame.overlaps(result.desired["preview"].frame));
    assert!(!result.mutations.iter().any(|mutation| mutation.window == "preview"));
    Ok(())
}

#[test]
fn leaving_a_visit_drops_its_override_and_normal_return_uses_saved_size() -> Result<()> {
    let (mut config, snapshot, target) = fixture();
    let copy = config.copy_view(&target.view, "Review".into())?;
    let mut runtime = Runtime::default();
    let mut request = Request::open(&config, target.clone());
    request.mode = TransitionMode::KeepSize;
    request.retain.insert("preview".into());
    plan(&config, &runtime, &snapshot, &request)?.commit(&mut runtime);
    let other = Target { view: copy, roots: target.roots.clone() };
    plan(&config, &runtime, &snapshot, &Request::open(&config, other))?.commit(&mut runtime);
    let result = plan(&config, &runtime, &snapshot, &Request::open(&config, target))?;
    assert_eq!(result.desired["preview"].frame.width, 820);
    assert!(result.presentations["work"].overrides.is_empty());
    Ok(())
}

#[test]
fn unrelated_scope_and_new_claims_are_not_overwritten_by_old_plans() -> Result<()> {
    let (config, snapshot, target) = fixture();
    let mut runtime = Runtime::default();
    runtime.generations.insert("game".into(), 5);
    let result = plan(&config, &runtime, &snapshot, &Request::open(&config, target))?;
    runtime.generations.insert("game".into(), 6);
    assert!(result.revalidate(&config, &runtime, &snapshot).is_ok());
    runtime
        .claims
        .insert("preview".into(), Claim { slot: "game".into(), placement: "protected".into() });
    assert_eq!(
        result.revalidate(&config, &runtime, &snapshot).err().map(|error| error.code),
        Some(ErrorCode::ClaimConflict)
    );
    Ok(())
}

#[test]
fn responsive_folding_restores_wide_ratios_and_variant_preferences() -> Result<()> {
    let (mut config, mut snapshot, target) = fixture();
    let mut other = config.windows["preview"].clone();
    other.id = "tools".into();
    config.windows.insert(other.id.clone(), other);
    let mut observed = snapshot.windows["preview"].clone();
    observed.binding.handle = 8;
    snapshot.windows.insert("tools".into(), observed);
    let group = group_mut(&mut config, &target)?;
    group.strategy = Strategy::Horizontal;
    group.ratios = vec![2.0, 1.0];
    if let Node::Placement(placement) = &mut group.children[0] {
        placement.preferences.clear();
    }
    group.children.push(Node::Placement(Placement::new("tools".into(), "tools".into())));
    group.variants.push(Variant {
        id: "compact".into(),
        below_width: Some(720.0),
        below_height: None,
        hysteresis: 24.0,
        strategy: Strategy::ResponsiveTabs,
        ratios: vec![],
    });
    let request = Request::open(&config, target.clone());
    let mut runtime = Runtime::default();
    let wide = plan(&config, &runtime, &snapshot, &request)?;
    assert_eq!(wide.desired.len(), 2);
    let wide_rect = wide.desired["preview"].frame;
    wide.commit(&mut runtime);
    if let Some(display) = snapshot.displays.get_mut("monitor") {
        display.work_area.width = 600;
    }
    snapshot.topology_revision += 1;
    let mut restore = Request::open(&config, target.clone());
    restore.mode = TransitionMode::Restore;
    let compact = plan(&config, &runtime, &snapshot, &restore)?;
    assert_eq!(compact.desired.len(), 1);
    compact.commit(&mut runtime);
    restore.id = new_id("request");
    if let Some(display) = snapshot.displays.get_mut("monitor") {
        display.work_area.width = 2000;
    }
    snapshot.topology_revision += 1;
    let wide_again = plan(&config, &runtime, &snapshot, &restore)?;
    assert_eq!(wide_again.desired["preview"].frame, wide_rect);
    assert_eq!(group_mut(&mut config, &target)?.ratios, vec![2.0, 1.0]);
    Ok(())
}

#[test]
fn generated_layouts_have_one_final_mutation_per_window_and_no_saved_state_drift() -> Result<()> {
    for count in [0, 1, 4, 8, 16] {
        for width in [800, 1200, 2000] {
            let (mut config, mut snapshot, target) = fixture();
            group_mut(&mut config, &target)?.children.clear();
            snapshot.windows.clear();
            for index in 0..count {
                let id = format!("generated-{index}");
                let mut window = config.windows["preview"].clone();
                window.id.clone_from(&id);
                config.windows.insert(id.clone(), window);
                let observed = ObservedWindow {
                    binding: Binding {
                        handle: index + 1,
                        process: 2,
                        process_started: 3,
                        session: 1,
                        generation: index + 4,
                        token_property: "test".into(),
                    },
                    frame: Rect { x: 0, y: 0, width: 620, height: 440 },
                    client: [600, 400],
                    dpi: 96,
                    display: "monitor".into(),
                    visible: true,
                    show_state: ShowState::Normal,
                    can_move: true,
                    can_resize: true,
                    can_hide: true,
                    has_owned_dialog: false,
                };
                snapshot.windows.insert(id.clone(), observed);
                group_mut(&mut config, &target)?
                    .children
                    .push(Node::Placement(Placement::new(id, "fixture".into())));
            }
            if let Some(display) = snapshot.displays.get_mut("monitor") {
                display.work_area.width = width;
            }
            let saved = serde_json::to_string(&config).map_err(|error| {
                Error::new(ErrorCode::StorageFailure, error.to_string(), "test")
            })?;
            let result =
                plan(&config, &Runtime::default(), &snapshot, &Request::open(&config, target))?;
            let unique: BTreeSet<_> =
                result.mutations.iter().map(|mutation| &mutation.window).collect();
            assert_eq!(unique.len(), result.mutations.len());
            assert_eq!(result.desired.len() as u64, count);
            let rectangles: Vec<_> = result.desired.values().map(|desired| desired.frame).collect();
            for (index, rect) in rectangles.iter().enumerate() {
                for prior in &rectangles[..index] {
                    assert!(!rect.overlaps(*prior));
                }
            }
            assert_eq!(
                saved,
                serde_json::to_string(&config).map_err(|error| Error::new(
                    ErrorCode::StorageFailure,
                    error.to_string(),
                    "test"
                ))?
            );
        }
    }
    Ok(())
}

#[test]
fn tab_identity_and_scope_are_validated_before_layout() {
    let (config, snapshot, target) = fixture();
    let mut request = Request::open(&config, target.clone());
    request.selected_tabs.insert("unrelated-group".into(), "unknown".into());
    assert_eq!(
        plan(&config, &Runtime::default(), &snapshot, &request).err().map(|error| error.code),
        Some(ErrorCode::OutOfScope)
    );
    request.selected_tabs.clear();
    request
        .selected_tabs
        .insert(config.views[&target.view].roots["main"].id().into(), "unknown".into());
    assert_eq!(
        plan(&config, &Runtime::default(), &snapshot, &request).err().map(|error| error.code),
        Some(ErrorCode::TargetMissing)
    );
}

#[test]
fn game_in_unrelated_slot_receives_zero_operations_and_stale_protection_is_detected() -> Result<()>
{
    let (mut config, mut snapshot, target) = fixture();
    if let Some(slot) = config.slots.get_mut("work") {
        slot.region[2] = 0.5;
    }
    config.slots.insert(
        "game".into(),
        DisplaySlot {
            id: "game".into(),
            name: "Game".into(),
            display: "monitor".into(),
            region: [0.5, 0.0, 0.5, 1.0],
            designated_public: false,
            fallback_displays: Vec::new(),
        },
    );
    let mut reference = config.windows["preview"].clone();
    reference.id = "game-window".into();
    reference.protection.geometry_lock = true;
    reference.protection.maintain_visible = true;
    reference.protection.prohibit_focus = true;
    config.windows.insert(reference.id.clone(), reference);
    let mut game = snapshot.windows["preview"].clone();
    game.binding.handle = 22;
    game.frame = Rect { x: 1000, y: 0, width: 1000, height: 1200 };
    game.client = [1000, 1200];
    game.can_move = false;
    game.can_resize = false;
    snapshot.windows.insert("game-window".into(), game);
    let mut runtime = Runtime::default();
    runtime
        .claims
        .insert("game-window".into(), Claim { slot: "game".into(), placement: "play".into() });
    runtime.presentations.insert(
        "game".into(),
        Presentation {
            view: target.view.clone(),
            root: "main".into(),
            context_display: "monitor".into(),
            context_area: snapshot.displays["monitor"].work_area,
            context_dpi: 96,
            visit: "game-visit".into(),
            selected_tabs: BTreeMap::new(),
            variants: BTreeMap::new(),
            group_bounds: BTreeMap::new(),
            fallbacks: BTreeMap::new(),
            overrides: BTreeMap::new(),
            protection: Protection::default(),
        },
    );
    let result = plan(&config, &runtime, &snapshot, &Request::open(&config, target))?;
    assert!(!result.mutations.iter().any(|mutation| mutation.window == "game-window"));
    result.commit(&mut runtime);
    assert_eq!(runtime.claims["game-window"].slot, "game");
    assert_eq!(runtime.presentations["game"].visit, "game-visit");
    if let Some(game) = snapshot.windows.get_mut("game-window") {
        game.frame.x += 1;
    }
    // Use a fresh Runtime with unchanged plan generations to isolate protection observation staleness.
    let mut before_commit = runtime.clone();
    before_commit.generations = result.generations.clone();
    assert_eq!(
        result.revalidate(&config, &before_commit, &snapshot).err().map(|error| error.code),
        Some(ErrorCode::StaleBinding)
    );
    Ok(())
}

#[test]
fn invalid_constant_formula_never_replaces_valid_configuration()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let (mut config, _, target) = fixture();
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("config.json");
    config.save(&path)?;
    let original = std::fs::read(&path)?;
    for expression in ["-1", "1/0", "unknown(1)"] {
        group_mut(&mut config, &target)?.gap = expression.into();
        assert!(config.save(&path).is_err());
        assert_eq!(std::fs::read(&path)?, original);
    }
    Ok(())
}

#[test]
fn distinct_references_cannot_claim_one_native_window() {
    let (config, mut snapshot, target) = fixture();
    snapshot.windows.insert("another-reference".into(), snapshot.windows["preview"].clone());
    let request = Request::open(&config, target);
    assert_eq!(
        plan(&config, &Runtime::default(), &snapshot, &request).err().map(|error| error.code),
        Some(ErrorCode::AmbiguousBinding)
    );
}

#[test]
fn selector_membership_is_staged_and_restores_stable_preferences() -> Result<()> {
    let (mut config, _, target) = fixture();
    group_mut(&mut config, &target)?.children.clear();
    config.collections.insert(
        "selected".into(),
        Collection {
            id: "selected".into(),
            name: "Work".into(),
            query: Query::All,
            include: BTreeSet::new(),
            exclude: BTreeSet::new(),
        },
    );
    group_mut(&mut config, &target)?.membership = Some(Membership {
        collection: "selected".into(),
        role: "local".into(),
        generated: BTreeMap::new(),
        retired: BTreeMap::new(),
    });
    let (mut staged, delta) = config.stage_memberships()?;
    assert_eq!(delta.added.len(), 1);
    assert!(group_mut(&mut config, &target)?.children.is_empty());
    let id = delta.added[0].clone();
    staged.save_properties(&target.view, &id, "context", None, Some([420.0, 260.0]))?;
    if let Some(collection) = staged.collections.get_mut("selected") {
        collection.exclude.insert("preview".into());
    }
    let (mut removed, delta) = staged.stage_memberships()?;
    assert_eq!(delta.removed, vec![id.clone()]);
    assert!(group_mut(&mut removed, &target)?.children.is_empty());
    if let Some(collection) = removed.collections.get_mut("selected") {
        collection.exclude.clear();
    }
    let (mut returned, delta) = removed.stage_memberships()?;
    assert_eq!(delta.added, vec![id]);
    let Node::Placement(placement) = &group_mut(&mut returned, &target)?.children[0] else {
        return Err(Error::new(ErrorCode::InvalidConfiguration, "Expected selector leaf", "test"));
    };
    assert_eq!(placement.preferences["context"].size_override, Some([420.0, 260.0]));
    let exported = LayoutPackage::from_view(&returned.views[&target.view]);
    let json = serde_json::to_string(&exported)
        .map_err(|error| Error::new(ErrorCode::StorageFailure, error.to_string(), "test"))?;
    assert!(!json.contains("selected"));
    assert!(!json.contains("420.0"));
    let copied = returned.copy_view(&target.view, "copy".into())?;
    assert_ne!(copied, target.view);
    returned.validate()?;
    Ok(())
}

#[test]
fn flow_wraps_without_resize_and_responsive_tabs_follow_focus_unless_explicit() -> Result<()> {
    let (mut config, mut snapshot, target) = fixture();
    for (index, id) in ["second", "third"].iter().enumerate() {
        let mut reference = config.windows["preview"].clone();
        reference.id = (*id).into();
        config.windows.insert((*id).into(), reference);
        let mut observed = snapshot.windows["preview"].clone();
        observed.binding.handle = index as u64 + 100;
        snapshot.windows.insert((*id).into(), observed);
        group_mut(&mut config, &target)?
            .children
            .push(Node::Placement(Placement::new((*id).into(), (*id).into())));
    }
    group_mut(&mut config, &target)?.strategy = Strategy::Flow;
    if let Some(slot) = config.slots.get_mut("work") {
        slot.region[2] = 0.65;
    }
    let flow =
        plan(&config, &Runtime::default(), &snapshot, &Request::open(&config, target.clone()))?;
    assert_eq!(flow.desired["preview"].frame.width, 620);
    assert!(flow.desired["third"].frame.y > flow.desired["preview"].frame.y);
    assert!(flow.mutations.iter().all(|mutation| mutation.move_only));
    group_mut(&mut config, &target)?.strategy = Strategy::ResponsiveTabs;
    snapshot.focused = Some("third".into());
    let focused =
        plan(&config, &Runtime::default(), &snapshot, &Request::open(&config, target.clone()))?;
    assert_eq!(focused.desired.keys().cloned().collect::<Vec<_>>(), vec!["third".to_owned()]);
    let mut request = Request::open(&config, target.clone());
    let group = group_mut(&mut config, &target)?;
    request.selected_tabs.insert(group.id.clone(), group.children[0].id().into());
    let explicit = plan(&config, &Runtime::default(), &snapshot, &request)?;
    assert!(explicit.desired.contains_key("preview"));
    Ok(())
}

#[test]
fn manual_edit_saves_only_changed_properties_and_promotion_keeps_other_exception() -> Result<()> {
    let (config, snapshot, target) = fixture();
    let mut request = Request::open(&config, target.clone());
    request.mode = TransitionMode::KeepHere;
    request.retain.insert("preview".into());
    let transition = plan(&config, &Runtime::default(), &snapshot, &request)?;
    let before = &snapshot.windows["preview"];
    let mut moved = before.clone();
    moved.frame.x += 12;
    let edit =
        ManualEdit::observed(target.view.clone(), &transition.desired["preview"], before, &moved)
            .ok_or_else(|| {
            Error::new(ErrorCode::InvalidConfiguration, "Expected manual edit", "test")
        })?;
    assert!(edit.position.is_some());
    assert!(edit.size.is_none());
    let mut runtime = Runtime::default();
    transition.commit(&mut runtime);
    runtime.promote_properties("work", "preview", true, false);
    assert!(!runtime.presentations["work"].overrides["preview"].preserve_position);
    assert!(runtime.presentations["work"].overrides["preview"].preserve_size);
    runtime.promote_properties("work", "preview", false, true);
    assert!(runtime.presentations["work"].overrides.is_empty());
    let mut rebinding = moved;
    rebinding.binding.generation += 1;
    assert!(
        ManualEdit::observed(target.view, &transition.desired["preview"], before, &rebinding)
            .is_none()
    );
    Ok(())
}

#[test]
fn output_provider_expiry_disconnect_and_capability_scope_are_enforced() -> Result<()> {
    let (mut config, mut snapshot, target) = fixture();
    config
        .windows
        .get_mut("preview")
        .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "fixture", "preview"))?
        .output_protection = OutputProtection::RequireVerifiedPrivate;
    let mut runtime = Runtime::default();
    let request = Request::open(&config, target);
    assert_eq!(
        plan(&config, &runtime, &snapshot, &request).err().map(|error| error.code),
        Some(ErrorCode::OutputStateUnknown)
    );
    runtime.providers.register(
        "fixture-provider".into(),
        "session".into(),
        BTreeSet::from(["preview".into()]),
        true,
        true,
    )?;
    let message = ProviderMessage {
        provider: "fixture-provider".into(),
        session: "session".into(),
        sequence: 1,
        window: "preview".into(),
        observed_ms: 1000,
        ttl_ms: 100,
        output: Some(OutputState::VerifiedPrivate),
        attention: Some(AttentionKind::ApprovalNeeded),
    };
    runtime.providers.accept(message.clone(), 1000)?;
    let transition = plan(&config, &runtime, &snapshot, &request)?;
    assert!(runtime.providers.accept(message.clone(), 1000).is_err());
    let mut malicious = message;
    malicious.sequence = 2;
    malicious.window = "ungranted".into();
    assert!(runtime.providers.accept(malicious, 1000).is_err());
    snapshot.now_ms = 1100;
    assert_eq!(runtime.providers.output("preview", 1100), OutputState::Unknown);
    assert_eq!(
        transition.revalidate(&config, &runtime, &snapshot).err().map(|error| error.code),
        Some(ErrorCode::OutputStateUnknown)
    );
    snapshot.now_ms = 1000;
    runtime.providers.disconnect("fixture-provider");
    assert_eq!(runtime.providers.output("preview", 1000), OutputState::Unknown);
    assert!(runtime.providers.attention(1000).is_empty());
    Ok(())
}

#[test]
fn group_and_visit_protections_have_independent_lifetimes_and_attention_is_scoped() -> Result<()> {
    let (mut config, snapshot, target) = fixture();
    group_mut(&mut config, &target)?.protection.geometry_lock = true;
    let mut runtime = Runtime::default();
    let transition = plan(&config, &runtime, &snapshot, &Request::open(&config, target.clone()))?;
    assert!(transition.mutations.is_empty());
    assert_eq!(transition.desired["preview"].frame, snapshot.windows["preview"].frame);
    transition.commit(&mut runtime);
    runtime
        .presentations
        .get_mut("work")
        .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "fixture", "work"))?
        .protection
        .prohibit_focus = true;
    assert!(effective_protection(&config, &runtime, "preview", None, None).prohibit_focus);
    runtime.attention_targets.insert("preview".into(), target.clone());
    let attention = attention_request(&config, &runtime, "preview")?;
    assert_eq!(attention.scope, BTreeSet::from(["work".into()]));
    assert!(attention.focus.is_none());
    let copy = config.copy_view(&target.view, "another visit".into())?;
    let replacement = Target { view: copy, roots: target.roots };
    let next = plan(&config, &runtime, &snapshot, &Request::open(&config, replacement))?;
    next.commit(&mut runtime);
    assert!(!runtime.presentations["work"].protection.prohibit_focus);
    Ok(())
}

#[test]
fn manual_minimization_is_not_undone_by_force_restore() -> Result<()> {
    let (mut config, mut snapshot, target) = fixture();
    config
        .windows
        .get_mut("preview")
        .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "fixture", "preview"))?
        .protection
        .maintain_visible = true;
    let mut runtime = Runtime::default();
    plan(&config, &runtime, &snapshot, &Request::open(&config, target.clone()))?
        .commit(&mut runtime);
    snapshot
        .windows
        .get_mut("preview")
        .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "fixture", "preview"))?
        .show_state = ShowState::Minimized;
    let mut request = Request::open(&config, target);
    request.mode = TransitionMode::Restore;
    let transition = plan(&config, &runtime, &snapshot, &request)?;
    assert!(transition.mutations.is_empty());
    assert!(transition.diagnostics.iter().any(|item| item.contains("user-minimized")));
    Ok(())
}

#[test]
fn control_fallback_never_uses_public_intersections_or_changes_saved_topology() {
    let (mut config, mut snapshot, _) = fixture();
    assert!(control_bounds(&config, &snapshot).is_some());
    let mut public = config.slots["work"].clone();
    public.id = "public".into();
    public.designated_public = true;
    config.slots.insert(public.id.clone(), public);
    assert!(control_bounds(&config, &snapshot).is_none());
    let mut private = config.slots["work"].clone();
    private.id = "private-control".into();
    private.display = "private-monitor".into();
    config.slots.insert(private.id.clone(), private);
    let mut display = snapshot.displays["monitor"].clone();
    display.id = "private-monitor".into();
    display.work_area.x = -2000;
    snapshot.displays.insert(display.id.clone(), display.clone());
    let expected = control_bounds(&config, &snapshot);
    assert!(expected.is_some_and(|bounds| bounds.x < 0));
    snapshot.displays.remove("private-monitor");
    assert!(control_bounds(&config, &snapshot).is_none());
    assert_eq!(config.slots["private-control"].display, "private-monitor");
    snapshot.displays.insert(display.id.clone(), display);
    assert_eq!(control_bounds(&config, &snapshot), expected);
}

fn two_window_fixture() -> Result<(Configuration, Snapshot, Target)> {
    let (mut config, mut snapshot, target) = fixture();
    let mut reference = config.windows["preview"].clone();
    reference.id = "second".into();
    config.windows.insert(reference.id.clone(), reference);
    let mut observed = snapshot.windows["preview"].clone();
    observed.binding.handle = 100;
    observed.frame.x = 800;
    snapshot.windows.insert("second".into(), observed);
    let group = group_mut(&mut config, &target)?;
    group.children.push(Node::Placement(Placement::new("second".into(), "second".into())));
    group.strategy = Strategy::Horizontal;
    for child in &mut group.children {
        if let Node::Placement(placement) = child {
            placement.preferences.clear();
        }
    }
    Ok((config, snapshot, target))
}

#[test]
fn all_three_group_preservation_modes_have_distinct_geometry() -> Result<()> {
    let (mut config, snapshot, target) = two_window_fixture()?;
    group_mut(&mut config, &target)?.preservation = GroupPreservation::Outer;
    let outer =
        plan(&config, &Runtime::default(), &snapshot, &Request::open(&config, target.clone()))?;
    assert_eq!(outer.desired["preview"].allocated.x, 20);
    assert_eq!(outer.desired["preview"].frame.height, 440);
    assert_ne!(outer.desired["preview"].frame.width, 620);
    group_mut(&mut config, &target)?.preservation = GroupPreservation::Children;
    group_mut(&mut config, &target)?.alignment = Alignment::End;
    let children =
        plan(&config, &Runtime::default(), &snapshot, &Request::open(&config, target.clone()))?;
    assert_eq!(children.desired["preview"].frame.width, 620);
    assert_eq!(children.desired["second"].frame.x + children.desired["second"].frame.width, 2000);
    assert!(children.mutations.iter().all(|mutation| mutation.move_only));
    group_mut(&mut config, &target)?.preservation = GroupPreservation::Arrangement;
    let arrangement =
        plan(&config, &Runtime::default(), &snapshot, &Request::open(&config, target))?;
    assert_eq!(arrangement.desired["second"].frame.x - arrangement.desired["preview"].frame.x, 780);
    assert_eq!(arrangement.desired["second"].frame.width, 620);
    Ok(())
}

#[test]
fn fixed_size_children_leave_flexible_remainder_and_allowed_fallback_folds() -> Result<()> {
    let (mut config, snapshot, target) = two_window_fixture()?;
    let mut request = Request::open(&config, target.clone());
    request.mode = TransitionMode::KeepSize;
    request.retain.insert("preview".into());
    let mixed = plan(&config, &Runtime::default(), &snapshot, &request)?;
    assert_eq!(mixed.desired["preview"].frame.width, 620);
    assert_eq!(mixed.desired["second"].frame.width, 1372);
    if let Some(slot) = config.slots.get_mut("work") {
        slot.region[2] = 0.55;
    }
    let group = group_mut(&mut config, &target)?;
    group.allowed_fallbacks = vec![Strategy::ResponsiveTabs];
    for child in &mut group.children {
        if let Node::Placement(placement) = child {
            placement.minimum_client = Some([800.0, 200.0]);
        }
    }
    let folded = plan(&config, &Runtime::default(), &snapshot, &Request::open(&config, target))?;
    assert_eq!(folded.desired.len(), 1);
    assert!(folded.diagnostics.iter().any(|message| message.contains("allowed fallback")));
    assert!(
        folded
            .mutations
            .iter()
            .any(|mutation| mutation.window == "second" && mutation.visible == Some(false))
    );
    Ok(())
}

#[test]
fn failed_child_rule_keeps_only_its_owned_subtree_and_valid_sibling_progresses() -> Result<()> {
    let (mut config, snapshot, target) = two_window_fixture()?;
    let root = group_mut(&mut config, &target)?;
    let children = std::mem::take(&mut root.children);
    for (index, child) in children.into_iter().enumerate() {
        let mut group = Group::new(format!("child-{index}"));
        group.children.push(child);
        root.children.push(Node::Group(group));
    }
    let prior =
        plan(&config, &Runtime::default(), &snapshot, &Request::open(&config, target.clone()))?;
    let observed = settled_snapshot(&prior, &snapshot);
    let mut runtime = Runtime::default();
    prior.commit(&mut runtime);
    let root = group_mut(&mut config, &target)?;
    for (index, child) in root.children.iter_mut().enumerate() {
        if let Node::Group(group) = child
            && let Node::Placement(placement) = &mut group.children[0]
        {
            placement
                .preferences
                .entry(prior.desired[&placement.window].context.clone())
                .or_default()
                .width_formula = Some(
                if index == 0 { "1 / (available_width - available_width)" } else { "500" }.into(),
            );
        }
    }
    let mut request = Request::open(&config, target);
    request.mode = TransitionMode::Restore;
    let next = plan(&config, &runtime, &observed, &request)?;
    assert_eq!(next.desired["preview"].frame, observed.windows["preview"].frame);
    assert!(next.mutations.iter().all(|mutation| mutation.window != "preview"));
    assert!(
        next.mutations
            .iter()
            .any(|mutation| mutation.window == "second" && mutation.geometry.is_some())
    );
    assert!(next.diagnostics.iter().any(|message| message.contains("dependent subtree retained")));
    Ok(())
}

#[test]
fn explicit_rescue_and_show_state_share_scope_validation_and_undo() -> Result<()> {
    let (mut config, mut snapshot, _) = fixture();
    snapshot
        .windows
        .get_mut("preview")
        .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "fixture", "preview"))?
        .frame
        .x = 9000;
    let action = WindowAction {
        id: "rescue".into(),
        expected_revision: config.revision,
        window: "preview".into(),
        slot: "work".into(),
        action: WindowActionKind::Rescue,
    };
    let runtime = Runtime::default();
    let rescue = plan_window_action(&config, &runtime, &snapshot, &action)?;
    assert!(rescue.mutations[0].move_only);
    assert_eq!(rescue.mutations[0].geometry.map(|frame| frame.x), Some(1380));
    let mut minimize = action;
    minimize.id = "minimize".into();
    minimize.action = WindowActionKind::ShowState(ShowState::Minimized);
    assert!(plan_window_action(&config, &runtime, &snapshot, &minimize).is_err());
    config
        .windows
        .get_mut("preview")
        .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "fixture", "preview"))?
        .capabilities
        .allow_show_state = true;
    let transition = plan_window_action(&config, &runtime, &snapshot, &minimize)?;
    let after = settled_snapshot(&transition, &snapshot);
    let mut current = runtime.clone();
    transition.commit(&mut current);
    let undo = UndoRecord::capture(&transition, &runtime, &after, &current)
        .reverse(&config, &current, &after)?;
    assert_eq!(undo.mutations[0].show_state, Some(ShowState::Normal));
    current
        .claims
        .insert("preview".into(), Claim { slot: "elsewhere".into(), placement: "new".into() });
    assert_eq!(
        plan_window_action(&config, &current, &snapshot, &minimize).err().map(|error| error.code),
        Some(ErrorCode::ClaimConflict)
    );
    Ok(())
}

#[test]
fn explicit_topology_fallback_has_independent_preferences_and_reconnect_restores_original()
-> Result<()> {
    let (mut config, mut snapshot, target) = fixture();
    config
        .slots
        .get_mut("work")
        .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "fixture", "work"))?
        .fallback_displays
        .push("replacement".into());
    let original = snapshot
        .displays
        .remove("monitor")
        .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "fixture", "monitor"))?;
    let mut replacement = original.clone();
    replacement.id = "replacement".into();
    snapshot.displays.insert(replacement.id.clone(), replacement);
    snapshot
        .windows
        .get_mut("preview")
        .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "fixture", "preview"))?
        .display = "replacement".into();
    let runtime = Runtime::default();
    let fallback = plan(&config, &runtime, &snapshot, &Request::open(&config, target.clone()))?;
    assert!(fallback.desired["preview"].context.contains("replacement"));
    assert_eq!(config.slots["work"].display, "monitor");
    config.save_properties(
        &target.view,
        &fallback.desired["preview"].placement,
        &fallback.desired["preview"].context,
        None,
        Some([440.0, 260.0]),
    )?;
    let mut active = Runtime::default();
    fallback.commit(&mut active);
    let mut current = settled_snapshot(&fallback, &snapshot);
    current.displays.insert(original.id.clone(), original);
    current.topology_revision += 1;
    let restored = plan(&config, &active, &current, &Request::open(&config, target))?;
    assert!(!restored.idempotent);
    assert_eq!(restored.desired["preview"].frame.width, 820);
    assert!(restored.desired["preview"].context.contains("monitor"));
    Ok(())
}

#[test]
fn private_fallback_rejects_public_intersection_and_unrelated_missing_monitor_does_not_block()
-> Result<()> {
    let (mut config, snapshot, target) = fixture();
    let mut public = config.slots["work"].clone();
    public.id = "public".into();
    public.designated_public = true;
    config.slots.insert(public.id.clone(), public);
    let mut missing = config.slots["work"].clone();
    missing.id = "missing".into();
    missing.display = "absent".into();
    missing.fallback_displays.push("monitor".into());
    assert!(resolve_slot(&config, &missing, &snapshot).is_err());
    config.slots.insert(missing.id.clone(), missing);
    config.slots.remove("public");
    config
        .slots
        .get_mut("missing")
        .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "fixture", "missing"))?
        .fallback_displays
        .clear();
    let mut runtime = Runtime::default();
    let mut request = Request::open(&config, target);
    let initial = plan(&config, &runtime, &snapshot, &request)?;
    let mut absent = initial.presentations["work"].clone();
    absent.context_display = "absent".into();
    runtime.presentations.insert("missing".into(), absent);
    request.id = "next".into();
    assert!(plan(&config, &runtime, &snapshot, &request).is_ok());
    assert!(runtime.presentations.contains_key("missing"));
    Ok(())
}
