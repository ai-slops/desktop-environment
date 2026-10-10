//! Pure reference fixtures; this benchmark cannot submit native operations.
use serde as _;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;
use tempfile as _;
use thiserror as _;
use window_manager_core::{
    Binding, Configuration, Display, DisplaySlot, Group, Node, ObservedWindow, Placement, Plan,
    Rect, Request, Runtime, ShowState, Snapshot, Strategy, Target, TransitionMode, Variant,
    WindowRef, plan_independent,
};

fn fixture(count: usize) -> (Configuration, Snapshot, Target) {
    let mut config = Configuration::default();
    let view = config.views.keys().next().cloned().unwrap_or_default();
    let slot = DisplaySlot {
        id: "work".into(),
        name: "Fixture".into(),
        display: "monitor".into(),
        region: [0.0, 0.0, 1.0, 1.0],
        designated_public: false,
        fallback_displays: vec![],
    };
    config.slots.insert(slot.id.clone(), slot);
    let mut root = Group::new("Nested reference fixture".into());
    root.columns = "2".into();
    let mut snapshot = Snapshot { topology_revision: 1, now_ms: 1000, ..Snapshot::default() };
    snapshot.displays.insert(
        "monitor".into(),
        Display {
            id: "monitor".into(),
            name: "Fixture".into(),
            work_area: Rect { x: 0, y: 0, width: 2400, height: 1600 },
            dpi: 96,
        },
    );
    for branch in 0..2 {
        let mut nested = Group::new(format!("Branch {branch}"));
        for index in branch * count / 2..(branch + 1) * count / 2 {
            let id = format!("fixture-{index}");
            let mut reference = WindowRef::unbound(id.clone(), id.clone());
            reference.allow_hide = true;
            reference.capabilities.allow_dpi_transfer = true; // Synthetic fixture, not an app profile.
            config.windows.insert(id.clone(), reference);
            let mut placement = Placement::new(id.clone(), "member".into());
            placement.default_preference.client_size = Some([300.0, 200.0]);
            nested.children.push(Node::Placement(placement));
            snapshot.windows.insert(
                id,
                ObservedWindow {
                    binding: Binding {
                        handle: u64::try_from(index + 1).unwrap_or(1),
                        process: 1,
                        process_started: 1,
                        session: 1,
                        generation: 1,
                        token_property: "synthetic-only".into(),
                    },
                    frame: Rect { x: 100, y: 100, width: 250, height: 180 },
                    client: [230, 140],
                    dpi: 96,
                    display: "monitor".into(),
                    visible: true,
                    show_state: ShowState::Normal,
                    can_move: true,
                    can_resize: true,
                    normal_resize_supported: true,
                    can_hide: true,
                    has_owned_dialog: false,
                },
            );
        }
        root.children.push(Node::Group(nested));
    }
    if let Some(view) = config.views.get_mut(&view) {
        view.roots.insert("main".into(), Node::Group(root));
    }
    (config, snapshot, Target { view, roots: BTreeMap::from([("main".into(), "work".into())]) })
}

fn settled(plan: &Plan, snapshot: &Snapshot) -> Snapshot {
    let mut after = snapshot.clone();
    for mutation in &plan.mutations {
        if let Some(window) = after.windows.get_mut(&mutation.window) {
            if let Some(frame) = mutation.geometry {
                window.client[0] += frame.width - window.frame.width;
                window.client[1] += frame.height - window.frame.height;
                window.frame = frame;
            }
            if let Some(visible) = mutation.visible {
                window.visible = visible;
            }
        }
    }
    after
}

fn percentile(sorted: &[u64], percentile: usize) -> u64 {
    sorted[(sorted.len() * percentile).div_ceil(100).saturating_sub(1)]
}

#[allow(clippy::too_many_lines)] // Complete reference scenario setup and measurement remain visible together.
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut cases = Vec::new();
    for count in [4, 8, 16] {
        for mode in [
            "move_only",
            "partial_resize",
            "widespread_resize",
            "semantic_switch",
            "responsive_fold",
            "mixed_dpi_logical",
            "disconnected",
        ] {
            let (mut config, mut snapshot, target) = fixture(count);
            let mut runtime = Runtime::default();
            let mut request = Request::open(&config, target.clone());
            if mode == "move_only" || mode == "partial_resize" {
                request.mode = TransitionMode::KeepSize;
                request.retain = config
                    .windows
                    .keys()
                    .enumerate()
                    .filter(|(index, _)| mode == "move_only" || index % 2 == 0)
                    .map(|(_, window)| window.clone())
                    .collect();
            }
            if mode == "semantic_switch" || mode == "responsive_fold" {
                let mut leaves = Vec::new();
                config.views[&target.view].roots["main"].placements(&mut leaves);
                let mut tabs = Group::new("Fixture tabs".into());
                tabs.children = leaves.into_iter().cloned().map(Node::Placement).collect();
                tabs.strategy =
                    if mode == "semantic_switch" { Strategy::SemanticTabs } else { Strategy::Grid };
                tabs.variants.push(Variant {
                    id: "compact".into(),
                    below_width: Some(700.0),
                    below_height: None,
                    hysteresis: 16.0,
                    strategy: Strategy::ResponsiveTabs,
                    ratios: vec![],
                    condition: None,
                });
                let group = tabs.id.clone();
                let selected = tabs.children[1].id().to_owned();
                config
                    .views
                    .get_mut(&target.view)
                    .ok_or("view missing")?
                    .roots
                    .insert("main".into(), Node::Group(tabs));
                let initial = plan_independent(
                    &config,
                    &runtime,
                    &snapshot,
                    &Request::open(&config, target.clone()),
                )?;
                snapshot = settled(&initial, &snapshot);
                initial.commit(&mut runtime);
                if mode == "semantic_switch" {
                    request.selected_tabs.insert(group, selected);
                } else {
                    snapshot
                        .displays
                        .get_mut("monitor")
                        .ok_or("display missing")?
                        .work_area
                        .width = 600;
                    snapshot.topology_revision += 1;
                }
            }
            if mode == "mixed_dpi_logical" {
                let display = snapshot.displays.get_mut("monitor").ok_or("display missing")?;
                display.dpi = 144;
                display.work_area = Rect { x: -3600, y: 0, width: 3600, height: 2400 };
            }
            if mode == "disconnected" {
                snapshot.displays.clear();
            }
            let original = serde_json::to_vec(&config)?;
            let mut samples = Vec::new();
            let mut failures = BTreeMap::<String, usize>::new();
            let mut impact = None;
            for iteration in 0..550 {
                let start = Instant::now();
                let result = plan_independent(&config, &runtime, &snapshot, &request);
                let elapsed = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
                if iteration < 50 {
                    continue;
                }
                samples.push(elapsed);
                match result {
                    Ok(plan) => {
                        if mode == "move_only" {
                            assert_eq!(plan.impact.resized, 0);
                            assert!(
                                plan.mutations
                                    .iter()
                                    .all(|mutation| mutation.geometry.is_none()
                                        || mutation.move_only)
                            );
                        }
                        let unique: BTreeSet<_> =
                            plan.mutations.iter().map(|mutation| &mutation.window).collect();
                        assert_eq!(unique.len(), plan.mutations.len());
                        impact = Some(plan.impact);
                    }
                    Err(error) => {
                        *failures.entry(format!("{:?}", error.code)).or_default() += 1;
                    }
                }
            }
            assert_eq!(serde_json::to_vec(&config)?, original);
            if mode == "disconnected" {
                assert_eq!(failures.values().sum::<usize>(), 500);
            } else {
                assert!(failures.is_empty(), "{mode}/{count}: {failures:?}");
            }
            samples.sort_unstable();
            cases.push(serde_json::json!({"application_class":"synthetic_metadata","windows":count,"mode":mode,"samples":500,"planning_ns":{"median":percentile(&samples,50),"p95":percentile(&samples,95),"p99":percentile(&samples,99),"worst":samples.last()},"impact":impact,"failures":failures,"submission":null,"geometry_settlement":null,"rendering_readiness":"unknown"}));
        }
    }
    let report = serde_json::json!({"fixture":"pure nested layout; no native operations","formula_limits":{"bytes":4096,"tokens":512,"depth":32,"membership_passes":4},"cases":cases});
    let output = serde_json::to_vec_pretty(&report)?;
    if let Some(path) = std::env::args_os().nth(1) {
        std::fs::write(path, output)?;
    } else {
        println!("{}", String::from_utf8(output)?);
    }
    Ok(())
}
