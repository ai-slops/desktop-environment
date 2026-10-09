use crate::service::{Command, Event, Worker};
use eframe::egui::{self, Color32, RichText};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;
use window_manager_core::{
    Collection, Composition, Configuration, Desired, DisplaySlot, Group, Id, Membership, Node,
    ObservedWindow, Placement, Plan, Protection, Query, Rect, Request, Runtime, Shortcut, Snapshot,
    Status, Strategy, Tag, Target, TransitionMode, TransitionResult, Variant, View, WindowRef,
    Workspace, context_key, new_id, slot_bounds,
};
use windows_window_manager::Candidate;

pub fn run(path: PathBuf) -> anyhow::Result<()> {
    std::fs::create_dir_all(
        path.parent().ok_or_else(|| anyhow::anyhow!("Configuration directory missing"))?,
    )?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let lock = options.open(path.with_extension("lock")).map_err(|error| {
        anyhow::anyhow!("이미 실행 중이거나 설정 잠금을 얻을 수 없습니다: {error}")
    })?;
    let watchdog = start_watchdog(&path);
    eframe::run_native(
        "Window Manager",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1280.0, 820.0])
                .with_min_inner_size([1000.0, 640.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            configure_style(&cc.egui_ctx);
            Ok(Box::new(Manager::new(path, lock, watchdog)))
        }),
    )
    .map_err(|error| anyhow::anyhow!("창 관리 앱을 열 수 없습니다: {error}"))
}

pub fn start_watchdog(path: &std::path::Path) -> anyhow::Result<std::process::Child> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .arg("--watch-parent")
        .arg(std::process::id().to_string())
        .arg(windows_window_manager::current_process_started()?.to_string())
        .arg("--config")
        .arg(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    Ok(command.spawn()?)
}

fn configure_style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Some(windows) = std::env::var_os("WINDIR")
        && let Ok(bytes) = std::fs::read(PathBuf::from(windows).join("Fonts/malgun.ttf"))
    {
        fonts.font_data.insert("korean".into(), egui::FontData::from_owned(bytes).into());
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().insert(0, "korean".into());
        }
    }
    ctx.set_fonts(fonts);
    ctx.set_visuals(egui::Visuals::dark());
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.visuals.panel_fill = Color32::from_rgb(20, 25, 34);
    style.visuals.selection.bg_fill = Color32::from_rgb(48, 91, 143);
    style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
    ctx.set_style(style);
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum DragMode {
    #[default]
    Move,
    Copy,
}

#[derive(Clone, Default, PartialEq, Eq)]
enum ShortcutSelection {
    #[default]
    View,
    Workspace,
    Composition(Id),
}

struct Manager {
    path: PathBuf,
    _lock: std::fs::File,
    watchdog: Option<std::process::Child>,
    config: Configuration,
    draft: Configuration,
    safe_mode: Option<String>,
    worker: Worker,
    snapshot: Snapshot,
    runtime: Runtime,
    inventory: Vec<Candidate>,
    selected_view: Id,
    selected_root: String,
    control_position: Option<Rect>,
    settled_control: Option<Rect>,
    selected_slot: Id,
    selected_group: Id,
    selected_window: Option<Id>,
    tag_input: String,
    retained: BTreeSet<Id>,
    mode: TransitionMode,
    focus_target: bool,
    search: String,
    page: usize,
    name: String,
    display_choice: Id,
    region: [f64; 4],
    undo: Vec<Configuration>,
    preview: Option<Plan>,
    latest_request: Option<Id>,
    apply_when_previewed: bool,
    pending_requests: Vec<(Request, bool)>,
    applying: bool,
    structure_source: Option<window_manager_core::NodeAddress>,
    size_context: String,
    size_destinations: BTreeSet<Id>,
    transient_filter: Option<Query>,
    shortcut_selection: ShortcutSelection,
    last_transition: Option<Request>,
    expansion_node: Id,
    expansion_slots: BTreeSet<Id>,
    parameter_group: Id,
    parameter_text: String,
    simulation: Option<window_manager_core::Simulation>,
    simulation_report: Option<window_manager_core::SimulationReport>,
    package: Option<window_manager_core::LayoutPackage>,
    package_path: String,
    package_mappings: BTreeMap<String, Id>,
    package_placeholders: BTreeSet<String>,
    drag_mode: DragMode,
    spatial_selection: BTreeSet<Id>,
    structure_preview: Option<(String, Configuration, String)>,
    monitor_epoch: Id,
    monitor_inventory: Vec<windows_window_manager::MonitorIdentity>,
    monitor_alias: Id,
    error: Option<String>,
    notice: String,
    result: Option<TransitionResult>,
    #[cfg(feature = "ui-smoke")]
    screenshot: Option<(PathBuf, std::time::Instant, bool)>,
}

impl Manager {
    #[cfg(feature = "ui-smoke")]
    #[allow(clippy::cast_possible_truncation)] // Dimensions are bounded by the GUI viewport.
    fn capture_smoke(&mut self, ctx: &egui::Context) {
        let Some((path, started, requested)) = self.screenshot.as_mut() else {
            return;
        };
        if !*requested && started.elapsed() > Duration::from_secs(2) {
            *requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let image = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = image {
            let pixels: Vec<u8> = image.pixels.iter().flat_map(Color32::to_array).collect();
            if let Err(error) = image::save_buffer(
                path,
                &pixels,
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
            ) {
                self.error = Some(error.to_string());
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    fn new(
        path: PathBuf,
        lock: std::fs::File,
        watchdog: anyhow::Result<std::process::Child>,
    ) -> Self {
        let (config, safe_mode) = match Configuration::load(&path) {
            Ok(config) => (config, None),
            Err(error) => (Configuration::default(), Some(error.to_string())),
        };
        let selected_view = config.views.keys().next().cloned().unwrap_or_default();
        let selected_slot = config.slots.keys().next().cloned().unwrap_or_default();
        let selected_group = config
            .views
            .get(&selected_view)
            .and_then(|view| view.roots.values().next())
            .map_or_else(String::new, |root| root.id().into());
        let error = watchdog
            .as_ref()
            .err()
            .map(|error| format!("독립 복구 도우미 오류; 창 숨김 차단: {error}"));
        let worker = Worker::start(path.clone(), config.clone());
        let selected_root = config
            .views
            .get(&selected_view)
            .and_then(|view| view.roots.keys().next())
            .cloned()
            .unwrap_or_default();
        #[cfg(feature = "ui-smoke")]
        let page = std::env::var("WINDOW_MANAGER_SMOKE_PAGE")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|page| *page <= 3)
            .unwrap_or_default();
        #[cfg(not(feature = "ui-smoke"))]
        let page = 0;
        let package_path =
            path.with_file_name("layout-package.json").to_string_lossy().into_owned();
        #[allow(unused_mut)] // Smoke fixtures prepare a pure preview after normal initialization.
        let mut manager = Self { path, _lock: lock, watchdog: watchdog.ok(), draft: config.clone(), config, safe_mode, worker, snapshot: Snapshot::default(), runtime: Runtime::default(), inventory: Vec::new(), selected_view, selected_root, control_position: None, settled_control: None, selected_slot, selected_group, selected_window: None, retained: BTreeSet::new(), mode: TransitionMode::Open, focus_target: false, search: String::new(), page, name: String::new(), display_choice: String::new(), region: [0.0, 0.0, 1.0, 1.0], undo: Vec::new(), preview: None, latest_request: None, apply_when_previewed: false, pending_requests: Vec::new(), applying: false, structure_source: None, size_context: String::new(), size_destinations: BTreeSet::new(), transient_filter: None, shortcut_selection: ShortcutSelection::View, last_transition: None, monitor_epoch: String::new(), monitor_inventory: Vec::new(), monitor_alias: String::new(), error, notice: "영역을 만들고 창을 추가한 뒤 미리보기로 시작하세요. 저장된 배치는 자동 적용하지 않습니다.".into(), result: None,
            tag_input: String::new(),
            expansion_node: String::new(),
            expansion_slots: BTreeSet::new(),
            parameter_group: String::new(), parameter_text: String::new(), simulation: None, simulation_report: None, package: None, package_path, package_mappings: BTreeMap::new(), package_placeholders: BTreeSet::new(),
            drag_mode: DragMode::Move, spatial_selection: BTreeSet::new(), structure_preview: None,
            #[cfg(feature = "ui-smoke")]
            screenshot: std::env::var_os("WINDOW_MANAGER_SCREENSHOT").map(|path| (PathBuf::from(path), std::time::Instant::now(), false)),
        };
        #[cfg(feature = "ui-smoke")]
        manager.prepare_smoke();
        manager
    }
    #[cfg(feature = "ui-smoke")]
    fn prepare_smoke(&mut self) {
        if std::env::var("WINDOW_MANAGER_SMOKE_PANEL").is_err() {
            return;
        }
        if self.page == 3 {
            self.package = self
                .config
                .views
                .get(&self.selected_view)
                .map(window_manager_core::LayoutPackage::from_view);
        }
        if self.page == 1
            && std::env::var("WINDOW_MANAGER_SMOKE_PANEL").is_ok_and(|panel| panel == "formula")
            && let Some(target) = self.target()
        {
            let input = window_manager_core::Simulation {
                target,
                width: 1280,
                height: 800,
                dpi: 96,
                group: Some(self.selected_group.clone()),
                count: Some(16),
                minimum: None,
                fixed_children: false,
                missing: BTreeSet::new(),
                display_present: true,
            };
            self.simulation_report = window_manager_core::simulate(&self.draft, &input).ok();
            self.simulation = Some(input);
        }
    }
    fn send(&mut self, command: Command) {
        if self.worker.sender.send(command).is_err() {
            self.error =
                Some("실행기가 종료되었습니다. --recover 명령으로 독립 복구할 수 있습니다.".into());
            self.applying = false;
        }
    }
    fn commit(&mut self, mut draft: Configuration) -> bool {
        if self.safe_mode.is_some() || self.applying {
            return false;
        }
        draft.revision = self.config.revision + 1;
        match draft.save(&self.path) {
            Ok(()) => {
                if self.undo.len() >= 50 {
                    self.undo.remove(0);
                }
                self.undo.push(self.config.clone());
                self.config = draft;
                self.draft = self.config.clone();
                self.preview = None;
                self.latest_request = None;
                self.send(Command::Refresh(self.config.clone()));
                true
            }
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }
    fn show_attention(&mut self, ui: &mut egui::Ui) {
        let attention: Vec<_> = self
            .runtime
            .providers
            .attention(window_manager_core::unix_millis())
            .into_iter()
            .cloned()
            .collect();
        for event in attention {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{} · {:?} · {}",
                    event.provider,
                    event.attention,
                    self.config
                        .windows
                        .get(&event.window)
                        .map_or("누락", |window| window.alias.as_str())
                ));
                if ui.button("해당 작업 영역 보기").clicked() {
                    match window_manager_core::attention_request(
                        &self.config,
                        &self.runtime,
                        &event.window,
                    ) {
                        Ok(request) => self.request_preview(request, true),
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
            });
        }
    }
    fn guard_control(&mut self, ctx: &egui::Context) -> bool {
        if window_manager_core::control_bounds(&self.config, &self.snapshot).is_none_or(|bounds| {
            self.settled_control != Some(bounds)
                || !windows_window_manager::control_fits(bounds)
                || self.config.slots.values().any(|slot| slot.designated_public)
                    && self.monitor_inventory.iter().any(|monitor| {
                        monitor.work_area.overlaps(bounds)
                            && !self
                                .snapshot
                                .displays
                                .values()
                                .any(|display| display.name == monitor.name)
                    })
        }) {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("비공개 제어 영역을 사용할 수 없습니다");
                ui.label("제어 내용은 가려져 있습니다. 사용할 화면을 비공개 제어 영역으로 명시적으로 지정하세요.");
                let displays: Vec<_> = self.snapshot.displays.values().cloned().collect();
                for display in displays {
                    if ui.button(format!("{} → 공개 지정을 해제하고 제어 영역 지정", display.name)).clicked() {
                        let mut draft = self.config.clone();
                        for slot in draft.slots.values_mut() { if slot.display == display.id { slot.designated_public = false; } }
                        let id = new_id("slot");
                        draft.slots.insert(id.clone(), DisplaySlot { id, name: "비공개 제어".into(), display: display.id, region: [0.0, 0.0, 1.0, 1.0], designated_public: false, fallback_displays: Vec::new() });
                        self.control_position = None;
                        self.commit(draft);
                    }
                }
                if ui.button("숨긴 창 복구").clicked() { self.send(Command::Recover); }
                self.monitor_mapping_ui(ui);
                if let Some(bounds) = window_manager_core::control_bounds(&self.config, &self.snapshot)
                    && ui.button("기존 비공개 제어 영역으로 돌아가기").clicked() {
                    self.settled_control = None; self.send(Command::PlaceControl(bounds));
                }
            });
            ctx.request_repaint_after(Duration::from_millis(200));
            return false;
        }
        true
    }
    fn manual_edit(&mut self, edit: window_manager_core::ManualEdit) {
        if self.safe_mode.is_some()
            || self
                .runtime
                .claims
                .get(&edit.window)
                .is_none_or(|claim| claim.slot != edit.slot || claim.placement != edit.placement)
        {
            return;
        }
        let mut saved = self.config.clone();
        match saved.save_properties(
            &edit.view,
            &edit.placement,
            &edit.context,
            edit.position,
            edit.size,
        ) {
            Ok(()) => {
                // Merge addressed properties into the editor draft without discarding unrelated typing.
                let mut pending_draft = self.draft.clone();
                let merged = pending_draft
                    .save_properties(
                        &edit.view,
                        &edit.placement,
                        &edit.context,
                        edit.position,
                        edit.size,
                    )
                    .is_ok();
                if self.commit(saved) {
                    if merged {
                        pending_draft.revision = self.config.revision;
                        self.draft = pending_draft;
                    }
                    self.send(Command::Promote(
                        edit.slot,
                        edit.window,
                        edit.position.is_some(),
                        edit.size.is_some(),
                    ));
                    self.notice =
                        "사용자 드래그에서 바뀐 위치·크기만 이 배치 문맥에 저장했습니다.".into();
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    fn poll(&mut self) {
        while let Ok(event) = self.worker.receiver.try_recv() {
            match event {
                Event::ManualEdit(edit) => self.manual_edit(edit),
                Event::ControlReady(bounds, ready) => {
                    if self.control_position == Some(bounds) {
                        self.settled_control = ready.then_some(bounds);
                    }
                }
                Event::Inventory(inventory) => self.inventory = inventory,
                Event::State(snapshot, runtime) => {
                    self.snapshot = snapshot;
                    self.runtime = runtime;
                    if let Some(bounds) =
                        window_manager_core::control_bounds(&self.config, &self.snapshot)
                    {
                        if self.control_position != Some(bounds) {
                            self.control_position = Some(bounds);
                            self.settled_control = None;
                            self.send(Command::PlaceControl(bounds));
                        }
                    } else {
                        self.settled_control = None;
                    }
                    if self.display_choice.is_empty() {
                        self.display_choice =
                            self.snapshot.displays.keys().next().cloned().unwrap_or_default();
                    }
                }
                Event::Preview(plan) => {
                    if self.latest_request.as_ref() == Some(&plan.id) {
                        self.preview = Some(*plan);
                        if self.apply_when_previewed {
                            self.apply_when_previewed = false;
                            self.apply_preview();
                        }
                    }
                }
                Event::Result(result) => {
                    self.applying = false;
                    self.notice = format!(
                        "전환 결과: {:?} · 렌더링 준비: {}",
                        result.status, result.rendering_readiness
                    );
                    let next_allowed = matches!(
                        result.status,
                        Status::Settled | Status::Superseded | Status::PartiallyApplied
                    );
                    self.result = Some(result);
                    self.preview = None;
                    if next_allowed && !self.pending_requests.is_empty() {
                        let (request, apply) = self.pending_requests.remove(0);
                        self.request_preview(request, apply);
                    } else if !next_allowed {
                        self.pending_requests.retain(|(request, _)| {
                            request.scope.is_disjoint(&self.runtime.suspended)
                        });
                        if !self.pending_requests.is_empty() {
                            let (request, apply) = self.pending_requests.remove(0);
                            self.request_preview(request, apply);
                        }
                    }
                }
                Event::Shortcut(number) => {
                    if let Some(shortcut) =
                        self.config.shortcuts.iter().find(|shortcut| shortcut.number == number)
                    {
                        match shortcut.target.resolve(&self.config) {
                            Ok(request) => self.request_preview(request, true),
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    }
                }
                Event::Error(error) => {
                    self.error = Some(error.to_string());
                    self.applying = false;
                    self.apply_when_previewed = false;
                }
                Event::Notice(notice) => self.notice = notice,
                Event::Barrier(_) => {}
                Event::Monitors(epoch, inventory) => {
                    self.monitor_epoch = epoch;
                    self.monitor_inventory = inventory;
                }
            }
        }
    }
    fn target(&self) -> Option<Target> {
        let view = self.config.views.get(&self.selected_view)?;
        if !self.config.slots.contains_key(&self.selected_slot) {
            return None;
        }
        Some(Target {
            view: view.id.clone(),
            roots: BTreeMap::from([(
                view.roots.get_key_value(&self.selected_root)?.0.clone(),
                self.selected_slot.clone(),
            )]),
        })
    }
    fn request_preview(&mut self, request: Request, apply: bool) {
        if self.safe_mode.is_some() {
            return;
        }
        if self.applying {
            // Keep the latest queued intent for an overlapping scope; retain disjoint work.
            self.pending_requests.retain(|(pending, _)| pending.scope.is_disjoint(&request.scope));
            if apply {
                self.send(Command::Supersede(request.scope.clone()));
            }
            self.pending_requests.push((request, apply));
            return;
        }
        self.last_transition = Some(request.clone());
        self.latest_request = Some(request.id.clone());
        self.apply_when_previewed = apply;
        self.preview = None;
        self.error = None;
        self.send(Command::Preview(self.config.clone(), request));
    }
    fn preview_selected(&mut self) {
        if let Some(target) = self.target() {
            let mut request = Request::open(&self.config, target);
            request.mode = self.mode;
            request.filter.clone_from(&self.transient_filter);
            if matches!(
                self.mode,
                TransitionMode::KeepSize | TransitionMode::KeepHere | TransitionMode::Bring
            ) {
                request.retain.clone_from(&self.retained);
            }
            if self.focus_target {
                request.focus.clone_from(&self.selected_window);
            }
            self.request_preview(request, false);
        } else {
            self.error = Some("배치와 디스플레이 영역을 선택하세요.".into());
        }
    }
    fn apply_preview(&mut self) {
        if let Some(plan) = self.preview.clone() {
            if self.watchdog.is_none() && plan.impact.hidden > 0 {
                self.error = Some("복구 도우미가 없어 숨김 계획을 적용할 수 없습니다.".into());
                return;
            }
            self.applying = true;
            self.send(Command::Apply(self.config.clone(), plan));
        }
    }
    fn select_view(&mut self, id: Id) {
        self.selected_view = id;
        self.selected_root =
            self.config.views[&self.selected_view].roots.keys().next().cloned().unwrap_or_default();
        self.selected_group = self.config.views[&self.selected_view]
            .roots
            .values()
            .next()
            .map_or_else(String::new, |root| root.id().into());
        self.preview = None;
    }
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.heading("작업 공간");
        ui.add_space(8.0);
        let workspaces: Vec<_> = self.config.workspaces.values().cloned().collect();
        for workspace in workspaces {
            ui.label(RichText::new(&workspace.name).color(Color32::from_rgb(140, 166, 203)));
            if let Some(id) = &workspace.remembered_view
                && ui.small_button("기억된 배치 선택").clicked()
            {
                self.select_view(id.clone());
            }
            let views: Vec<_> = self
                .config
                .views
                .values()
                .filter(|view| view.workspace == workspace.id)
                .map(|view| (view.id.clone(), view.name.clone()))
                .collect();
            for (id, name) in views {
                if ui.selectable_label(self.selected_view == id, format!("▦  {name}")).clicked() {
                    self.select_view(id);
                }
            }
            ui.add_space(10.0);
        }
        ui.separator();
        if let Some(view) = self.config.views.get(&self.selected_view) {
            egui::ComboBox::from_id_salt("target-root")
                .selected_text(format!("루트: {}", self.selected_root))
                .show_ui(ui, |ui| {
                    for role in view.roots.keys() {
                        ui.selectable_value(&mut self.selected_root, role.clone(), role);
                    }
                });
            if ui.button("이 작업 공간의 배치로 기억").clicked() {
                let mut draft = self.config.clone();
                if let Some(workspace) = draft.workspaces.get_mut(&view.workspace) {
                    workspace.remembered_view = Some(view.id.clone());
                }
                self.commit(draft);
            }
        }
        ui.text_edit_singleline(&mut self.name);
        if ui.button("+ 작업 공간").clicked() && !self.name.trim().is_empty() {
            let mut draft = self.config.clone();
            let workspace = Workspace {
                id: new_id("workspace"),
                name: self.name.trim().into(),
                remembered_view: None,
            };
            let view = View {
                id: new_id("view"),
                workspace: workspace.id.clone(),
                name: "기본 배치".into(),
                roots: BTreeMap::from([(
                    "main".into(),
                    Node::Group(Group::new("기본 그룹".into())),
                )]),
            };
            let id = view.id.clone();
            draft.workspaces.insert(workspace.id.clone(), workspace);
            draft.views.insert(view.id.clone(), view);
            if self.commit(draft) {
                self.select_view(id);
                self.name.clear();
            }
        }
        if ui.button("+ 독립 배치 복사").clicked() {
            let mut draft = self.config.clone();
            let name = if self.name.trim().is_empty() {
                "새 배치".into()
            } else {
                self.name.trim().into()
            };
            match draft.copy_view(&self.selected_view, name) {
                Ok(id) => {
                    if self.commit(draft) {
                        self.select_view(id);
                        self.name.clear();
                    }
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        ui.separator();
        ui.label("전환 대상 영역");
        for slot in self.config.slots.values() {
            if ui.selectable_label(self.selected_slot == slot.id, &slot.name).clicked() {
                self.selected_slot = slot.id.clone();
                self.preview = None;
            }
        }
        ui.add_space(16.0);
        ui.colored_label(Color32::from_rgb(221, 177, 105), "출력 / 캡처 상태: 미검증");
        ui.small("같은 실제 창은 여러 배치에서도 내용을 공유합니다.");
    }
    #[allow(clippy::too_many_lines)] // One inventory/inspector UI panel.
    fn inventory(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("창 목록");
            if ui.button("다시 검색").clicked() {
                self.send(Command::Refresh(self.config.clone()));
            }
            ui.text_edit_singleline(&mut self.search);
        });
        ui.label("기존 창을 등록합니다. 배치 작업은 앱을 실행하거나 종료하지 않습니다.");
        let candidates = self.inventory.clone();
        let search = self.search.to_lowercase();
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            for candidate in candidates
                .iter()
                .filter(|candidate| candidate.title.to_lowercase().contains(&search))
            {
                ui.horizontal(|ui| {
                    ui.label(&candidate.title);
                    ui.small(&candidate.class);
                    if ui.button("배치에 추가").clicked() {
                        self.add_candidate(candidate.clone(), None);
                    }
                    if self
                        .selected_window
                        .as_ref()
                        .is_some_and(|id| !self.snapshot.windows.contains_key(id))
                        && ui.button("선택한 참조에 연결").clicked()
                    {
                        self.add_candidate(candidate.clone(), self.selected_window.clone());
                    }
                });
                ui.separator();
            }
        });
        ui.heading("관리 중 / 누락된 참조");
        let references: Vec<_> = self.config.windows.values().cloned().collect();
        for reference in references {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(
                        self.selected_window.as_ref() == Some(&reference.id),
                        &reference.alias,
                    )
                    .clicked()
                {
                    self.selected_window = Some(reference.id.clone());
                    self.tag_input = reference
                        .tags
                        .iter()
                        .map(|tag| tag.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ");
                }
                ui.small(self.snapshot.windows.get(&reference.id).map_or_else(
                    || "누락 · 수동 연결 필요".into(),
                    |window| {
                        format!(
                            "{} · {}×{} client · {} DPI",
                            if window.visible { "표시" } else { "숨김" },
                            window.client[0],
                            window.client[1],
                            window.dpi
                        )
                    },
                ));
                if window_manager_core::shared_public_content(
                    &self.config,
                    &self.runtime,
                    &reference.id,
                    self.snapshot.now_ms,
                ) {
                    ui.colored_label(
                        Color32::YELLOW,
                        "공개 배치와 앱 콘텐츠 공유 · 복사해도 같은 창",
                    );
                }
                let mut retain = self.retained.contains(&reference.id);
                if ui.checkbox(&mut retain, "전환 시 유지").changed() {
                    if retain {
                        self.retained.insert(reference.id.clone());
                    } else {
                        self.retained.remove(&reference.id);
                    }
                }
                if let Some(claim) = self.runtime.claims.get(&reference.id) {
                    ui.small(format!(
                        "소유: {}",
                        self.config
                            .slots
                            .get(&claim.slot)
                            .map_or(claim.slot.as_str(), |slot| slot.name.as_str())
                    ));
                }
            });
        }
        if let Some(id) = &self.selected_window
            && let Some(edit) = self.draft.windows.get_mut(id)
        {
            ui.separator();
            ui.text_edit_singleline(&mut edit.alias);
            ui.checkbox(
                &mut edit.public_content,
                "이 창의 콘텐츠는 공개 출력에 사용됨 (공유 경고 유지)",
            );
            ui.horizontal(|ui| {
                ui.checkbox(&mut edit.protection.geometry_lock, "위치·크기 잠금");
                ui.checkbox(&mut edit.protection.maintain_visible, "계속 표시");
                ui.checkbox(&mut edit.protection.keep_monitor, "모니터 유지");
                ui.checkbox(&mut edit.protection.prohibit_focus, "포커스 요청 금지");
            });
            egui::ComboBox::from_id_salt("output-protection")
                .selected_text(format!("출력 보호: {:?}", edit.output_protection))
                .show_ui(ui, |ui| {
                    for policy in [
                        window_manager_core::OutputProtection::None,
                        window_manager_core::OutputProtection::RequireVerifiedPrivate,
                        window_manager_core::OutputProtection::FreezeWhileLinkedOrUnknown,
                    ] {
                        ui.selectable_value(
                            &mut edit.output_protection,
                            policy,
                            format!("{policy:?}"),
                        );
                    }
                });
            ui.small("비공개 지정은 캡처 안전 증명이 아닙니다. 검증 공급자가 없으면 출력 증거는 unknown입니다.");
            ui.horizontal(|ui| {
                ui.checkbox(&mut edit.capabilities.allow_move, "이 앱 이동 허용");
                ui.checkbox(&mut edit.capabilities.allow_resize, "크기 변경 허용");
                ui.checkbox(&mut edit.capabilities.allow_show_state, "상태 복원 호환성 확인됨");
            });
            ui.checkbox(&mut edit.allow_hide, "이 앱의 숨김/복구 호환성을 확인했음 (숨김 허용)");
            ui.horizontal(|ui| {
                ui.label("태그");
                if ui.text_edit_singleline(&mut self.tag_input).changed() {
                    edit.tags = self
                        .tag_input
                        .split(',')
                        .map(str::trim)
                        .filter(|tag| !tag.is_empty())
                        .map(|tag| Tag { name: tag.into(), source: "manual".into() })
                        .collect();
                }
            });
        }
        if ui.button("선택 창의 알림 이동 대상 = 현재 배치·루트·영역").clicked()
            && let Some(window) = &self.selected_window
            && let Some(target) = self.target()
        {
            self.send(Command::AttentionTarget(window.clone(), target));
        }
        ui.horizontal_wrapped(|ui| {
            for (label, kind) in [
                ("오프스크린 창 구조 미리보기", window_manager_core::WindowActionKind::Rescue),
                ("선택 창 표시 미리보기", window_manager_core::WindowActionKind::Reveal),
                (
                    "보통 상태 복원 미리보기",
                    window_manager_core::WindowActionKind::ShowState(
                        window_manager_core::ShowState::Normal,
                    ),
                ),
                (
                    "최소화 미리보기",
                    window_manager_core::WindowActionKind::ShowState(
                        window_manager_core::ShowState::Minimized,
                    ),
                ),
                (
                    "최대화 미리보기 (활성화 가능)",
                    window_manager_core::WindowActionKind::ShowState(
                        window_manager_core::ShowState::Maximized,
                    ),
                ),
            ] {
                if ui.add_enabled(!self.applying, egui::Button::new(label)).clicked()
                    && let Some(window) = &self.selected_window
                {
                    let id = new_id("action");
                    self.latest_request = Some(id.clone());
                    self.preview = None;
                    self.apply_when_previewed = false;
                    self.send(Command::WindowAction(
                        self.config.clone(),
                        window_manager_core::WindowAction {
                            id,
                            expected_revision: self.config.revision,
                            window: window.clone(),
                            slot: self.selected_slot.clone(),
                            action: kind,
                        },
                    ));
                }
            }
        });
        if ui.button("참조 설정 저장").clicked() {
            self.commit(self.draft.clone());
        }
    }
    fn add_candidate(&mut self, candidate: Candidate, rebind: Option<Id>) {
        if let Some(id) = rebind {
            self.send(Command::Bind(self.config.clone(), id, candidate));
            return;
        }
        let id = self
            .snapshot
            .windows
            .iter()
            .find(|(_, observed)| observed.binding.handle == candidate.handle)
            .map_or_else(|| new_id("window"), |(id, _)| id.clone());
        let mut draft = self.config.clone();
        draft.windows.entry(id.clone()).or_insert_with(|| WindowRef {
            id: id.clone(),
            alias: candidate.title.clone(),
            tags: Vec::new(),
            application_hint: Some(candidate.class.clone()),
            allow_hide: false,
            public_content: false,
            protection: Protection::default(),
            output_protection: window_manager_core::OutputProtection::None,
            capabilities: window_manager_core::CapabilityProfile::default(),
        });
        let Some(view) = draft.views.get_mut(&self.selected_view) else {
            return;
        };
        let Some(group) =
            view.roots.values_mut().find_map(|root| find_group_mut(root, &self.selected_group))
        else {
            self.error = Some("배치 편집 화면에서 대상 그룹을 선택하세요.".into());
            return;
        };
        group.children.push(Node::Placement(Placement::new(
            id.clone(),
            format!("window-{}", group.children.len() + 1),
        )));
        if self.commit(draft) {
            self.selected_window = Some(id.clone());
            self.tag_input = self.config.windows[&id]
                .tags
                .iter()
                .map(|tag| tag.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            if !self.snapshot.windows.contains_key(&id) {
                self.send(Command::Bind(self.config.clone(), id, candidate));
            }
        }
    }
    #[allow(clippy::too_many_lines)] // One composition editor UI panel.
    fn slots(&mut self, ui: &mut egui::Ui) {
        self.monitor_mapping_ui(ui);
        ui.heading("디스플레이 영역");
        ui.label(
            "모니터를 명시적으로 선택합니다. 연결 변경 시 다른 모니터로 자동 배정하지 않습니다.",
        );
        egui::ComboBox::from_id_salt("display")
            .selected_text(
                self.snapshot
                    .displays
                    .get(&self.display_choice)
                    .map_or("모니터 선택", |display| display.name.as_str()),
            )
            .show_ui(ui, |ui| {
                for display in self.snapshot.displays.values() {
                    ui.selectable_value(
                        &mut self.display_choice,
                        display.id.clone(),
                        &display.name,
                    );
                }
            });
        ui.horizontal(|ui| {
            ui.label("영역 이름");
            ui.text_edit_singleline(&mut self.name);
        });
        ui.horizontal(|ui| {
            for (label, value) in ["x", "y", "너비", "높이"].into_iter().zip(&mut self.region) {
                ui.label(label);
                ui.add(egui::DragValue::new(value).range(0.0..=1.0).speed(0.01));
            }
        });
        if ui.button("+ 영역 만들기").clicked() {
            let mut draft = self.config.clone();
            let id = new_id("slot");
            draft.slots.insert(
                id.clone(),
                DisplaySlot {
                    id: id.clone(),
                    name: if self.name.is_empty() {
                        "작업 영역".into()
                    } else {
                        self.name.clone()
                    },
                    display: self.display_choice.clone(),
                    region: self.region,
                    designated_public: false,
                    fallback_displays: Vec::new(),
                },
            );
            if self.commit(draft) {
                self.selected_slot = id;
                self.name.clear();
            }
        }
        ui.separator();
        if let Some(slot) = self.draft.slots.get_mut(&self.selected_slot) {
            ui.text_edit_singleline(&mut slot.name);
            ui.checkbox(&mut slot.designated_public, "공개 출력 지정 (실제 캡처는 미검증)");
            ui.label("원본 화면이 없을 때 사용할 명시적 대체 화면 (선택 순서)");
            for display in
                self.snapshot.displays.values().filter(|display| display.id != slot.display)
            {
                let mut enabled = slot.fallback_displays.contains(&display.id);
                if ui.checkbox(&mut enabled, &display.name).changed() {
                    if enabled {
                        slot.fallback_displays.push(display.id.clone());
                    } else {
                        slot.fallback_displays.retain(|id| id != &display.id);
                    }
                }
            }
            ui.small(format!(
                "대체 순서: {} · 원본 재연결 시 자동 이동하지 않고 복원 미리보기를 사용합니다.",
                slot.fallback_displays.join(" → ")
            ));
            ui.horizontal(|ui| {
                for (label, value) in ["x", "y", "너비", "높이"].into_iter().zip(&mut slot.region)
                {
                    ui.label(label);
                    ui.add(egui::DragValue::new(value).range(0.0..=1.0).speed(0.01));
                }
            });
            if ui.button("선택한 모니터에 매핑").clicked() {
                slot.display.clone_from(&self.display_choice);
            }
            ui.label(slot_bounds(slot, &self.snapshot).map_or_else(
                |error| error.to_string(),
                |rect| format!("{}, {} · {}×{} px", rect.x, rect.y, rect.width, rect.height),
            ));
        }
        if ui.button("영역 수정 저장").clicked() {
            self.commit(self.draft.clone());
        }
        ui.heading("현재 화면 구성");
        let presentations: Vec<_> = self
            .runtime
            .presentations
            .iter()
            .map(|(id, value)| (id.clone(), value.clone()))
            .collect();
        for (slot, mut presentation) in presentations {
            ui.label(format!(
                "{} → {} · 임시 유지 {}개",
                self.config.slots.get(&slot).map_or(slot.as_str(), |slot| slot.name.as_str()),
                self.config
                    .views
                    .get(&presentation.view)
                    .map_or(presentation.view.as_str(), |view| view.name.as_str()),
                presentation.overrides.len()
            ));
            protection_editor(ui, &mut presentation.protection);
            if ui.button("현재 방문 동안 보호 적용").clicked() {
                self.send(Command::ProtectPresentation(slot, presentation.protection));
            }
        }
        if ui.button("현재 구성 저장").clicked() {
            let targets = self
                .runtime
                .presentations
                .iter()
                .map(|(slot, presentation)| Target {
                    view: presentation.view.clone(),
                    roots: BTreeMap::from([(presentation.root.clone(), slot.clone())]),
                })
                .collect();
            let mut draft = self.config.clone();
            let id = new_id("composition");
            draft.compositions.insert(
                id.clone(),
                Composition {
                    id,
                    name: if self.name.is_empty() {
                        "화면 구성".into()
                    } else {
                        self.name.clone()
                    },
                    targets,
                    protection: Protection::default(),
                },
            );
            self.commit(draft);
        }
        let compositions: Vec<_> = self.config.compositions.values().cloned().collect();
        for mut composition in compositions {
            ui.horizontal(|ui| {
                ui.label(&composition.name);
                if ui.button("구성 미리보기").clicked() {
                    self.request_preview(
                        Request {
                            id: new_id("request"),
                            expected_revision: self.config.revision,
                            scope: composition
                                .targets
                                .iter()
                                .flat_map(|target| target.roots.values().cloned())
                                .collect(),
                            targets: composition.targets.clone(),
                            mode: TransitionMode::Open,
                            retain: BTreeSet::new(),
                            selected_tabs: BTreeMap::new(),
                            focus: None,
                            filter: None,
                            expansion: None,
                            release: BTreeSet::new(),
                        },
                        false,
                    );
                }
            });
            protection_editor(ui, &mut composition.protection);
            if ui.button("구성 보호 저장").clicked() {
                let mut draft = self.config.clone();
                draft.compositions.insert(composition.id.clone(), composition);
                self.commit(draft);
            }
        }
    }

    fn monitor_mapping_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("현재 모니터의 식별·연결 확인").show(ui, |ui| {
            ui.small("확인은 현재 화면 구성에서만 유효합니다. 화면 연결·위치·DPI가 바뀌면 다시 확인해야 하며 저장된 배치 문맥은 유지됩니다.");
            let aliases: BTreeSet<_> = self.config.slots.values().flat_map(|slot| std::iter::once(slot.display.clone()).chain(slot.fallback_displays.iter().cloned())).collect();
            egui::ComboBox::from_id_salt("monitor-alias").selected_text(if self.monitor_alias.is_empty() { "저장된 화면 ID 선택 또는 입력" } else { &self.monitor_alias }).show_ui(ui, |ui| { for alias in aliases { ui.selectable_value(&mut self.monitor_alias, alias.clone(), alias); } });
            ui.text_edit_singleline(&mut self.monitor_alias);
            for monitor in self.monitor_inventory.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("{} · {} · {:?} · {} DPI · {}", monitor.name, monitor.device, monitor.work_area, monitor.dpi, monitor.hardware.as_deref().unwrap_or("하드웨어 ID 확인 불가")));
                    if ui.add_enabled(!self.monitor_alias.is_empty(), egui::Button::new("이 화면을 선택한 ID로 확인")).clicked() { self.send(Command::ConfirmMonitor { epoch: self.monitor_epoch.clone(), device: monitor.device, alias: self.monitor_alias.clone() }); }
                });
            }
        });
    }
    fn editor(&mut self, ui: &mut egui::Ui) {
        ui.heading("배치 편집");
        #[cfg(feature = "ui-smoke")]
        if let Ok(panel) = std::env::var("WINDOW_MANAGER_SMOKE_PANEL") {
            match panel.as_str() {
                "formula" => {
                    self.formula_tools(ui);
                    return;
                }
                "spatial" => {
                    self.spatial_tools(ui);
                    return;
                }
                _ => {}
            }
        }
        ui.label("구조 저장은 실제 창을 이동하지 않습니다. 미리보기와 적용으로 반영하세요.");
        if let Some(view) = self.config.views.get(&self.selected_view) {
            let mut placements = Vec::new();
            for root in view.roots.values() {
                root.placements(&mut placements);
            }
            let shared =
                placements.iter().map(|placement| &placement.window).collect::<BTreeSet<_>>();
            for window in shared {
                if window_manager_core::shared_public_content(
                    &self.config,
                    &self.runtime,
                    window,
                    self.snapshot.now_ms,
                ) {
                    ui.colored_label(Color32::YELLOW, format!("{} · 공개 출력과 콘텐츠를 공유합니다. 배치 복사는 앱 내용을 복제하지 않습니다.", self.config.windows[window].alias));
                }
            }
        }
        let mut tab_action = None;
        if let Some(view) = self.draft.views.get_mut(&self.selected_view) {
            ui.horizontal(|ui| {
                ui.label("배치 이름");
                ui.text_edit_singleline(&mut view.name);
            });
            for (role, node) in &mut view.roots {
                ui.label(format!("루트 역할: {role}"));
                tree_editor(
                    ui,
                    node,
                    &mut self.selected_group,
                    &mut tab_action,
                    &self.config.slots,
                    &self.selected_slot,
                    0,
                );
            }
        }
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.name);
            if ui.button("+ 루트 역할").clicked()
                && !self.name.trim().is_empty()
                && let Some(view) = self.draft.views.get_mut(&self.selected_view)
            {
                view.roots
                    .entry(self.name.trim().into())
                    .or_insert_with(|| Node::Group(Group::new("새 루트".into())));
                self.name.clear();
            }
        });
        self.collections_editor(ui);
        self.structural_tools(ui);
        self.expansion_tools(ui);
        self.formula_tools(ui);
        self.spatial_tools(ui);
        if ui.button("저장된 구조로 다시 계산 미리보기 · 임시 유지 해제").clicked()
            && let Some(target) = self.target()
        {
            let mut request = Request::open(&self.config, target);
            request.mode = TransitionMode::Restore;
            request.filter.clone_from(&self.transient_filter);
            self.request_preview(request, false);
        }
        if ui.button("배치 구조 저장").clicked() {
            self.commit(self.draft.clone());
        }
        if let Some((group, child)) = tab_action
            && let Some(target) = self.target()
        {
            let mut request = Request::open(&self.config, target);
            request.selected_tabs.insert(group, child);
            self.request_preview(request, false);
        }
        if ui.button("선택한 그룹에 하위 그룹 추가").clicked()
            && let Some(view) = self.draft.views.get_mut(&self.selected_view)
            && let Some(group) =
                view.roots.values_mut().find_map(|root| find_group_mut(root, &self.selected_group))
        {
            group.children.push(Node::Group(Group::new("새 그룹".into())));
        }
        ui.separator();
        self.shortcuts_editor(ui);
    }

    fn shortcuts_editor(&mut self, ui: &mut egui::Ui) {
        ui.label("고정 단축키: 대상 ID와 영역에 연결 (등록 변경 후 앱 재시작)");
        egui::ComboBox::from_id_salt("shortcut-destination")
            .selected_text(match &self.shortcut_selection {
                ShortcutSelection::View => "현재 배치 · 선택한 고정 영역",
                ShortcutSelection::Workspace => "현재 작업 공간 · 기억한 배치 · 고정 영역",
                ShortcutSelection::Composition(id) => self
                    .config
                    .compositions
                    .get(id)
                    .map_or("구성 없음", |composition| composition.name.as_str()),
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.shortcut_selection,
                    ShortcutSelection::View,
                    "현재 배치 · 선택한 고정 영역",
                );
                ui.selectable_value(
                    &mut self.shortcut_selection,
                    ShortcutSelection::Workspace,
                    "현재 작업 공간 · 기억한 배치 · 고정 영역",
                );
                for composition in self.config.compositions.values() {
                    ui.selectable_value(
                        &mut self.shortcut_selection,
                        ShortcutSelection::Composition(composition.id.clone()),
                        format!("전체 화면 구성: {}", composition.name),
                    );
                }
            });
        ui.horizontal(|ui| {
            for number in 1..=9 {
                if ui.button(format!("{number}")).clicked()
                    && let Some(target) = self.target()
                {
                    let destination = match &self.shortcut_selection {
                        ShortcutSelection::View => {
                            window_manager_core::CommandTarget::View { target }
                        }
                        ShortcutSelection::Workspace => {
                            window_manager_core::CommandTarget::Workspace {
                                workspace: self.config.views[&target.view].workspace.clone(),
                                roots: target.roots,
                            }
                        }
                        ShortcutSelection::Composition(id) => {
                            window_manager_core::CommandTarget::Composition {
                                composition: id.clone(),
                            }
                        }
                    };
                    let mut draft = self.config.clone();
                    draft.shortcuts.retain(|shortcut| shortcut.number != number);
                    draft.shortcuts.push(Shortcut {
                        number,
                        target: window_manager_core::ShortcutTarget::Command(destination),
                    });
                    self.commit(draft);
                }
            }
        });
    }

    fn expansion_tools(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("현재 방문에서 확장·축소").show(ui, |ui| {
            let mut nodes = Vec::new();
            if let Some(root) = self.config.views.get(&self.selected_view).and_then(|view| view.roots.get(&self.selected_root)) {
                collect_nodes(root, &self.selected_view, &self.selected_root, &mut nodes);
            }
            egui::ComboBox::from_id_salt("expansion-node").selected_text(nodes.iter().find(|(address, _, _)| address.node == self.expansion_node).map_or("확장할 항목 선택", |(_, path, _)| path.as_str())).show_ui(ui, |ui| {
                for (address, path, _) in &nodes { ui.selectable_value(&mut self.expansion_node, address.node.clone(), path); }
            });
            ui.label("상위 그룹·현재 영역은 다른 영역을 빌리지 않습니다. 모니터·여러 영역은 아래에서 빌릴 영역을 직접 선택하세요.");
            for slot in self.config.slots.values().filter(|slot| slot.id != self.selected_slot) {
                let mut selected = self.expansion_slots.contains(&slot.id);
                if ui.checkbox(&mut selected, &slot.name).changed() { if selected { self.expansion_slots.insert(slot.id.clone()); } else { self.expansion_slots.remove(&slot.id); } }
            }
            ui.horizontal_wrapped(|ui| {
                for (label, area) in [("상위 그룹으로 확장", window_manager_core::ExpansionArea::Group), ("현재 영역으로 확장", window_manager_core::ExpansionArea::Slot), ("모니터로 확장", window_manager_core::ExpansionArea::Monitor), ("선택한 여러 영역으로 확장", window_manager_core::ExpansionArea::Slots)] {
                    if ui.add_enabled(!self.expansion_node.is_empty(), egui::Button::new(label)).clicked() && let Some(target) = self.target() {
                        let borrow_slots = if matches!(area, window_manager_core::ExpansionArea::Group | window_manager_core::ExpansionArea::Slot) { BTreeSet::new() } else { self.expansion_slots.clone() };
                        let mut request = Request::open(&self.config, target);
                        request.scope.extend(borrow_slots.iter().cloned());
                        request.expansion = Some(window_manager_core::Expansion { node: self.expansion_node.clone(), area, borrow_slots });
                        request.filter.clone_from(&self.transient_filter);
                        self.request_preview(request, false);
                    }
                }
                if ui.add_enabled(self.runtime.presentations.get(&self.selected_slot).is_some_and(|active| active.expansion.is_some()), egui::Button::new("축소·빌린 영역 복원 미리보기")).clicked() {
                    match self.runtime.collapse_request(&self.config, &self.selected_slot) { Ok(request) => self.request_preview(request, false), Err(error) => self.error = Some(error.to_string()) }
                }
            });
            ui.label("확장은 저장된 구조를 바꾸지 않습니다. 미리보기의 범위와 숨김 영향을 확인한 뒤 적용하세요.");
        });
    }

    #[allow(clippy::too_many_lines)] // Drag previews expose addressed source/target paths before changing a draft.
    fn spatial_tools(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("창·그룹·탭 드래그 / 선택 묶기").default_open(cfg!(feature = "ui-smoke") && std::env::var("WINDOW_MANAGER_SMOKE_PANEL").is_ok()).show(ui, |ui| {
            ui.label("아래 항목 전체를 대상 그룹에 드래그합니다. 드롭은 검토할 구조 초안만 준비합니다.");
            let mut copy = self.drag_mode == DragMode::Copy; if ui.checkbox(&mut copy, "드롭으로 독립 복사 (기본은 이동)").changed() { self.drag_mode = if copy { DragMode::Copy } else { DragMode::Move }; }
            let mut nodes = Vec::new();
            for view in self.draft.views.values() { for (role, root) in &view.roots { collect_nodes(root, &view.id, &format!("{} / {role}", view.name), &mut nodes); } }
            let mut proposed = None;
            for (address, path, is_placement) in &nodes {
                ui.dnd_drag_source(egui::Id::new(("subtree-drag", &address.node)), address.clone(), |ui| { ui.label(format!("↕ {path} · {}", if *is_placement { "이 창 배치" } else { "전체 하위 트리" })); });
                if !is_placement {
                    let (_, dropped) = ui.dnd_drop_zone::<window_manager_core::NodeAddress, _>(egui::Frame::group(ui.style()), |ui| { ui.small(format!("드롭 대상: {path} / 마지막 자식")); });
                    if let Some(source) = dropped {
                        let destination = window_manager_core::NodeDestination { view: address.view.clone(), group: address.node.clone(), index: None };
                        let source_path = nodes.iter().find(|(address, _, _)| address.node == source.node).map_or(source.node.as_str(), |(_, path, _)| path.as_str());
                        let action = if self.drag_mode == DragMode::Copy { window_manager_core::StructureAction::Copy { source: (*source).clone(), destination } } else { window_manager_core::StructureAction::Move { source: (*source).clone(), destination } };
                        proposed = Some((action, format!("{}: {source_path} → {path} / 마지막 자식", if self.drag_mode == DragMode::Copy { "독립 복사" } else { "전체 하위 트리 이동" })));
                    }
                    if ui.small_button(format!("조상 그룹 선택: {path}")).clicked() { self.select_view(address.view.clone()); self.selected_group.clone_from(&address.node); }
                }
            }
            let parent = self.draft.views.get(&self.selected_view).and_then(|view| view.roots.values().find_map(|root| root.find(&self.selected_group))).cloned();
            if let Some(Node::Group(group)) = parent {
                ui.label(format!("선택 묶기의 부모: {} ({})", group.name, group.id));
                for child in &group.children { let mut selected = self.spatial_selection.contains(child.id()); if ui.checkbox(&mut selected, format!("자식 {}", child.id())).changed() { if selected { self.spatial_selection.insert(child.id().into()); } else { self.spatial_selection.remove(child.id()); } } }
                if ui.button("선택한 직접 자식을 새 그룹으로 묶기 검토").clicked() { let children = self.spatial_selection.intersection(&group.children.iter().map(|child| child.id().to_owned()).collect()).cloned().collect(); proposed = Some((window_manager_core::StructureAction::Wrap { view: self.selected_view.clone(), parent: group.id.clone(), children, name: "묶은 그룹".into(), strategy: Strategy::Horizontal }, format!("그룹 {}의 선택한 직접 자식을 묶습니다. ID와 각 자식의 설정을 유지합니다.", group.name))); }
            }
            if let Some(source) = self.structure_source.clone() {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("선택 원본 그룹 풀기 검토").clicked() { proposed = Some((window_manager_core::StructureAction::Unwrap { source: source.clone() }, format!("원본 {} 그룹을 풀어 자식을 부모에 올립니다.", source.node))); }
                    if ui.button("선택 원본의 배치 참조 제거 검토").clicked() { proposed = Some((window_manager_core::StructureAction::Remove { source: source.clone() }, format!("원본 {} 배치 참조만 제거합니다. 앱 실행 상태는 유지됩니다.", source.node))); }
                });
            }
            if let Some((action, description)) = proposed {
                match self.draft.edit_structure(&window_manager_core::StructureEdit { expected_revision: self.draft.revision, action }) { Ok(after) => { self.structure_preview = Some((serde_json::to_string(&self.draft).unwrap_or_default(), after, description)); }, Err(error) => { self.structure_preview = None; self.error = Some(error.to_string()); } }
            }
            let mut apply = false; let mut cancel = false;
            if let Some((_, _, description)) = &self.structure_preview {
                ui.separator(); ui.label(description); ui.label("표시한 원본·대상에 구조 변경을 적용합니다. 실제 창 이동은 구조 저장 후 전환 미리보기에서 별도로 적용합니다.");
                ui.horizontal(|ui| { apply = ui.button("검토한 구조를 초안에 반영").clicked(); cancel = ui.button("취소").clicked(); });
            }
            if cancel { self.structure_preview = None; }
            if apply && let Some((before, after, _)) = self.structure_preview.take() {
                if serde_json::to_string(&self.draft).unwrap_or_default() == before { self.draft = after; self.spatial_selection.clear(); self.notice = "검토한 구조를 초안에 반영했습니다. 구조 저장으로 확정하세요.".into(); } else { self.error = Some("검토 후 초안이 변경되었습니다. 드롭/묶기 검토를 다시 실행하세요.".into()); }
            }
        });
    }

    #[allow(clippy::too_many_lines)] // Scoped declarations and synthetic inputs share an explicit no-native commit boundary.
    fn formula_tools(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("수식 문맥·매개변수·합성 미리보기").default_open(cfg!(feature = "ui-smoke") && std::env::var("WINDOW_MANAGER_SMOKE_PANEL").is_ok()).show(ui, |ui| {
            let mut nodes = Vec::new();
            if let Some(view) = self.draft.views.get(&self.selected_view) { for (role, root) in &view.roots { collect_nodes(root, &view.id, role, &mut nodes); } }
            ui.label(format!("편집 범위: {}", nodes.iter().find(|(address, _, _)| address.node == self.selected_group).map_or("그룹을 선택하세요", |(_, path, _)| path.as_str())));
            let group = self.draft.views.get(&self.selected_view).and_then(|view| view.roots.values().find_map(|root| root.find(&self.selected_group))).and_then(|node| if let Node::Group(group) = node { Some(group) } else { None });
            if let Some(group) = group {
                if self.parameter_group != self.selected_group { self.parameter_group.clone_from(&self.selected_group); self.parameter_text = serde_json::to_string_pretty(&group.parameters).unwrap_or_default(); }
                let mut leaves = Vec::new(); for child in &group.children { child.placements(&mut leaves); }
                ui.label(format!("직접 자식 {}개 · 후보 {}개 · logical 단위 (96 DPI), 위치는 상위 그룹 기준", group.children.len(), leaves.len()));
                ui.small(leaves.iter().map(|leaf| format!("{} ({})", self.draft.windows.get(&leaf.window).map_or("미연결", |window| window.alias.as_str()), leaf.id)).collect::<Vec<_>>().join(", "));
                let active = self.preview.as_ref().and_then(|plan| plan.presentations.get(&self.selected_slot)).or_else(|| self.runtime.presentations.get(&self.selected_slot));
                if let Some(active) = active.filter(|active| active.view == self.selected_view) {
                    ui.label(format!("현재 문맥: {} DPI · 그룹 할당 {:?}", active.context_dpi, active.group_bounds.get(&self.selected_group)));
                    if let Some(inputs) = active.group_inputs.get(&self.selected_group) { ui.monospace(serde_json::to_string_pretty(inputs).unwrap_or_default()); }
                }
                ui.label("이름 → 수식으로 매개변수를 선언합니다. 예: {\"gutter\": \"8\", \"compact_limit\": \"gutter * 100\"}");
                ui.add(egui::TextEdit::multiline(&mut self.parameter_text).code_editor().desired_rows(4));
                if ui.button("이 그룹의 매개변수를 초안에 반영").clicked() {
                    let parsed = serde_json::from_str::<BTreeMap<String, String>>(&self.parameter_text);
                    match parsed { Ok(parameters) => { let mut draft = self.draft.clone(); if let Some(group) = draft.views.get_mut(&self.selected_view).and_then(|view| view.roots.values_mut().find_map(|root| root.group_mut(&self.selected_group))) { group.parameters = parameters; } match draft.validate() { Ok(()) => { self.draft = draft; self.notice = "매개변수를 초안에 반영했습니다. 구조 저장 후 실제 배치를 미리볼 수 있습니다.".into(); }, Err(error) => self.error = Some(error.to_string()) } }, Err(error) => self.error = Some(error.to_string()) }
                }
            }
            ui.separator();
            ui.label("합성 미리보기 — 실제 창에 명령을 보내지 않습니다.");
            if self.simulation.is_none() && let Some(target) = self.target() { self.simulation = Some(window_manager_core::Simulation { target, width: 1600, height: 900, dpi: 96, group: None, count: None, minimum: None, fixed_children: false, missing: BTreeSet::new(), display_present: true }); }
            if let Some(input) = self.simulation.as_mut() {
                ui.horizontal_wrapped(|ui| {
                    ui.label("물리 너비 / 높이 / DPI"); ui.add(egui::DragValue::new(&mut input.width).range(1..=65_536)); ui.add(egui::DragValue::new(&mut input.height).range(1..=65_536)); ui.add(egui::DragValue::new(&mut input.dpi).range(48..=768));
                    if ui.button("가로 화면").clicked() { input.width = 1600; input.height = 900; }
                    if ui.button("세로 화면").clicked() { input.width = 900; input.height = 1600; }
                });
                let mut count_enabled = input.count.is_some(); if ui.checkbox(&mut count_enabled, "선택 그룹의 후보 수 변경").changed() { input.count = count_enabled.then_some(4); }
                if let Some(count) = input.count.as_mut() { ui.add(egui::DragValue::new(count).range(0..=256)); }
                optional_pair(ui, "전체 최소 클라이언트 크기", &mut input.minimum, [640.0, 480.0]);
                ui.checkbox(&mut input.fixed_children, "자식 크기 고정"); ui.checkbox(&mut input.display_present, "디스플레이 있음");
                if let Some(window) = &self.selected_window { let mut missing = input.missing.contains(window); if ui.checkbox(&mut missing, "선택한 창의 바인딩 없음").changed() { if missing { input.missing.insert(window.clone()); } else { input.missing.remove(window); } } }
            }
            if ui.button("초안을 합성 평가").clicked() && let Some(target) = self.target() && let Some(input) = self.simulation.as_mut() {
                input.target = target; input.group = Some(self.selected_group.clone());
                match window_manager_core::simulate(&self.draft, input) { Ok(report) => self.simulation_report = Some(report), Err(error) => { self.simulation_report = None; self.error = Some(error.to_string()); } }
            }
            if let Some(report) = &self.simulation_report {
                draw_rectangles(ui, &[report.area], report.desired.values(), &self.draft);
                if let Some(error) = &report.error { ui.colored_label(Color32::LIGHT_RED, error.to_string()); }
                ui.label(format!("이동 {} · 크기 변경 {} · 숨김 {} · 그대로 {}", report.impact.moved, report.impact.resized, report.impact.hidden, report.impact.unchanged));
                ui.monospace(serde_json::to_string_pretty(&report.group_inputs).unwrap_or_default());
                for diagnostic in &report.diagnostics { ui.label(diagnostic); }
            }
        });
    }

    #[allow(clippy::too_many_lines)] // File import, explicit mapping, and independent installation remain distinct user actions.
    fn library(&mut self, ui: &mut egui::Ui) {
        ui.heading("레이아웃 라이브러리");
        ui.label("내보내기는 창 핸들·제목·태그·화면 ID·개인 크기 문맥을 제외합니다. 가져오기는 독립 복사이며 창을 이동하지 않습니다.");
        ui.horizontal(|ui| {
            ui.label("패키지 JSON 경로");
            ui.text_edit_singleline(&mut self.package_path);
        });
        ui.horizontal(|ui| {
            if ui.button("현재 배치의 내보내기 준비").clicked()
                && let Some(view) = self.config.views.get(&self.selected_view)
            {
                self.package = Some(window_manager_core::LayoutPackage::from_view(view));
                self.package_mappings.clear();
                self.package_placeholders.clear();
            }
            if ui.button("파일 읽기").clicked() {
                self.package = None;
                self.package_mappings.clear();
                self.package_placeholders.clear();
                let loaded = (|| -> anyhow::Result<window_manager_core::LayoutPackage> {
                    let path = std::path::Path::new(&self.package_path);
                    anyhow::ensure!(
                        std::fs::metadata(path)?.len()
                            <= window_manager_core::MAX_CONFIGURATION_BYTES,
                        "패키지 크기 한도를 초과했습니다."
                    );
                    let package: window_manager_core::LayoutPackage =
                        serde_json::from_reader(std::fs::File::open(path)?)?;
                    package.validate()?;
                    Ok(package)
                })();
                match loaded {
                    Ok(package) => self.package = Some(package),
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
        });
        if let Some(package) = self.package.as_mut() {
            ui.horizontal(|ui| {
                ui.label("패키지 이름");
                ui.text_edit_singleline(&mut package.name);
            });
            ui.small(format!(
                "ID {} · schema {} · expression {} · 전략 {:?} · 의존성 {:?}",
                package.id,
                package.version,
                package.language_version,
                package.supported_strategies,
                package.dependencies
            ));
            for (key, expression) in &mut package.required_parameters {
                ui.horizontal_wrapped(|ui| {
                    let label = key
                        .rsplit_once(':')
                        .and_then(|(id, parameter)| {
                            package.roots.values().find_map(|root| root.find(id)).and_then(|node| {
                                if let Node::Group(group) = node {
                                    Some(format!("{} / {parameter}", group.name))
                                } else {
                                    None
                                }
                            })
                        })
                        .unwrap_or_else(|| key.clone());
                    ui.label(label).on_hover_text(key);
                    if ui.text_edit_singleline(expression).changed()
                        && let Some((group_id, parameter)) = key.rsplit_once(':')
                    {
                        for root in package.roots.values_mut() {
                            if let Some(group) = root.group_mut(group_id) {
                                group.parameters.insert(parameter.into(), expression.clone());
                            }
                        }
                    }
                });
            }
            for role in &package.required_roles {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("역할: {role}"));
                    let mapping = self.package_mappings.entry(role.clone()).or_default();
                    egui::ComboBox::from_id_salt(("package-role", role))
                        .selected_text(
                            self.config
                                .windows
                                .get(mapping)
                                .map_or("명시적으로 창 선택", |window| {
                                    window.alias.as_str()
                                }),
                        )
                        .show_ui(ui, |ui| {
                            for window in self.config.windows.values() {
                                ui.selectable_value(mapping, window.id.clone(), &window.alias);
                            }
                        });
                    let mut placeholder = self.package_placeholders.contains(role);
                    if ui.checkbox(&mut placeholder, "미연결 자리표시자로 설치").changed()
                    {
                        if placeholder {
                            self.package_placeholders.insert(role.clone());
                        } else {
                            self.package_placeholders.remove(role);
                        }
                    }
                });
            }
            if ui.button("검증된 패키지를 파일에 저장").clicked() {
                let validation = package.validate();
                match validation {
                    Ok(()) => match serde_json::to_vec_pretty(package)
                        .map_err(anyhow::Error::from)
                        .and_then(|bytes| {
                            anyhow::ensure!(
                                bytes.len() as u64 <= window_manager_core::MAX_CONFIGURATION_BYTES,
                                "패키지 크기 한도를 초과했습니다."
                            );
                            window_manager_core::atomic_write(
                                std::path::Path::new(&self.package_path),
                                &bytes,
                            )
                            .map_err(anyhow::Error::from)
                        }) {
                        Ok(()) => {
                            self.notice =
                                "패키지를 저장했습니다. 실제 창은 변경되지 않았습니다.".into();
                        }
                        Err(error) => self.error = Some(error.to_string()),
                    },
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            let package = package.clone();
            if ui.button("역할 매핑을 검증하고 현재 작업공간에 독립 복사 설치").clicked()
            {
                let mut draft = self.config.clone();
                let mut mappings = self.package_mappings.clone();
                mappings.retain(|_, window| !window.is_empty());
                for role in &self.package_placeholders {
                    let id = new_id("unbound-window");
                    draft.windows.insert(
                        id.clone(),
                        WindowRef {
                            id: id.clone(),
                            alias: format!("미연결 · {role}"),
                            tags: Vec::new(),
                            application_hint: None,
                            allow_hide: false,
                            public_content: false,
                            protection: Protection::default(),
                            output_protection: window_manager_core::OutputProtection::None,
                            capabilities: window_manager_core::CapabilityProfile::default(),
                        },
                    );
                    mappings.insert(role.clone(), id);
                }
                if let Some(workspace) =
                    self.config.views.get(&self.selected_view).map(|view| view.workspace.clone())
                {
                    match package.install(&mut draft, &workspace, &mappings) {
                        Ok(view) => {
                            if self.commit(draft) {
                                self.select_view(view);
                                self.notice =
                                    "독립 복사를 설치했습니다. 영역 매핑 후 미리보기로 적용하세요."
                                        .into();
                            }
                        }
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)] // Source selection and addressed draft operations share one inspectable editor panel.
    fn structural_tools(&mut self, ui: &mut egui::Ui) {
        let mut nodes = Vec::new();
        for view in self.draft.views.values() {
            for (role, root) in &view.roots {
                collect_nodes(root, &view.id, &format!("{} / {role}", view.name), &mut nodes);
            }
        }
        egui::CollapsingHeader::new("하위 트리 이동·독립 복사 / 크기 복사").show(ui, |ui| {
            egui::ComboBox::from_id_salt("structure-source")
                .selected_text(
                    self.structure_source
                        .as_ref()
                        .and_then(|source| {
                            nodes.iter().find(|(address, _, _)| address.node == source.node)
                        })
                        .map_or("원본 선택", |(_, label, _)| label.as_str()),
                )
                .show_ui(ui, |ui| {
                    for (address, label, _) in &nodes {
                        if ui
                            .selectable_label(
                                self.structure_source
                                    .as_ref()
                                    .is_some_and(|source| source.node == address.node),
                                label,
                            )
                            .clicked()
                        {
                            self.structure_source = Some(address.clone());
                            self.size_context.clear();
                        }
                    }
                });
            ui.label(format!("삽입 대상: 현재 배치의 선택한 그룹 {}", self.selected_group));
            let mut action = None;
            if let Some(source) = self.structure_source.clone() {
                let destination = window_manager_core::NodeDestination {
                    view: self.selected_view.clone(),
                    group: self.selected_group.clone(),
                    index: None,
                };
                ui.horizontal(|ui| {
                    if ui.button("하위 트리 이동 → 초안").clicked() {
                        action = Some(window_manager_core::StructureAction::Move {
                            source: source.clone(),
                            destination: destination.clone(),
                        });
                    }
                    if ui.button("하위 트리 독립 복사 → 초안").clicked() {
                        action = Some(window_manager_core::StructureAction::Copy {
                            source: source.clone(),
                            destination,
                        });
                    }
                });
                if let Ok(Node::Placement(placement)) = self.draft.node(&source) {
                    egui::ComboBox::from_id_salt("size-source-context")
                        .selected_text(if self.size_context.is_empty() {
                            "크기 원본 문맥"
                        } else {
                            &self.size_context
                        })
                        .show_ui(ui, |ui| {
                            for key in placement.preferences.keys() {
                                ui.selectable_value(&mut self.size_context, key.clone(), key);
                            }
                        });
                    for (address, label, is_placement) in &nodes {
                        if *is_placement {
                            let mut selected = self.size_destinations.contains(&address.node);
                            if ui.checkbox(&mut selected, label).changed() {
                                if selected {
                                    self.size_destinations.insert(address.node.clone());
                                } else {
                                    self.size_destinations.remove(&address.node);
                                }
                            }
                        }
                    }
                    if let Some(slot) = self.config.slots.get(&self.selected_slot) {
                        let context = context_key(slot, "base");
                        ui.small(format!("크기만 복사할 대상 문맥: {context}. 수식은 유지됩니다."));
                        if ui.button("선택한 배치들에 크기 복사 → 초안").clicked() {
                            action = Some(window_manager_core::StructureAction::CopySize {
                                source,
                                source_context: self.size_context.clone(),
                                destinations: nodes
                                    .iter()
                                    .filter(|(address, _, is_placement)| {
                                        *is_placement
                                            && self.size_destinations.contains(&address.node)
                                    })
                                    .map(|(address, _, _)| (address.clone(), context.clone()))
                                    .collect(),
                            });
                        }
                    }
                }
            }
            if let Some(action) = action {
                match self.draft.edit_structure(&window_manager_core::StructureEdit {
                    expected_revision: self.draft.revision,
                    action,
                }) {
                    Ok(draft) => {
                        self.draft = draft;
                        self.notice =
                            "구조 초안에 반영했습니다. 구조 저장 후 전환을 미리보세요.".into();
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
        });
    }
    fn collections_editor(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("컬렉션 / 동적 구성원").show(ui, |ui| {
            ui.label("조건·포함·제외는 초안입니다. 구성원 계산 후 구조 저장, 전환 미리보기로 반영하세요.");
            if ui.button("+ 태그 컬렉션").clicked() {
                let id = new_id("collection");
                self.draft.collections.insert(id.clone(), Collection { id, name: "새 컬렉션".into(), query: Query::Tag("work".into()), include: BTreeSet::new(), exclude: BTreeSet::new() });
            }
            for collection in self.draft.collections.values_mut() {
                egui::CollapsingHeader::new(&collection.name).id_salt(&collection.id).show(ui, |ui| {
                    ui.text_edit_singleline(&mut collection.name);
                    ui.horizontal(|ui| {
                        if ui.button("전체").clicked() { collection.query = Query::All; }
                        if ui.button("태그").clicked() { collection.query = Query::Tag(String::new()); }
                        if ui.button("앱 힌트").clicked() { collection.query = Query::Application(String::new()); }
                        if ui.button("조건 부정").clicked() { collection.query = Query::Not(Box::new(collection.query.clone())); }
                    });
                    query_editor(ui, &mut collection.query);
                    for window in self.config.windows.values() {
                        ui.horizontal(|ui| {
                            ui.label(&window.alias);
                            let mut include = collection.include.contains(&window.id);
                            let mut exclude = collection.exclude.contains(&window.id);
                            if ui.checkbox(&mut include, "포함").changed() { if include { collection.include.insert(window.id.clone()); } else { collection.include.remove(&window.id); } }
                            if ui.checkbox(&mut exclude, "제외").changed() { if exclude { collection.exclude.insert(window.id.clone()); } else { collection.exclude.remove(&window.id); } }
                            ui.small(if collection.selects(window) { "선택됨" } else { "선택 안 됨" });
                        });
                    }
                });
            }
            let choices: Vec<_> = self.draft.collections.values().map(|collection| (collection.id.clone(), collection.name.clone())).collect();
            if let Some(view) = self.draft.views.get_mut(&self.selected_view)
                && let Some(group) = view.roots.values_mut().find_map(|root| find_group_mut(root, &self.selected_group)) {
                ui.label(format!("연결 대상: {}", group.name));
                egui::ComboBox::from_id_salt("membership-source").selected_text(group.membership.as_ref().map_or("수동 구성원", |membership| membership.collection.as_str())).show_ui(ui, |ui| {
                    if ui.selectable_label(group.membership.is_none(), "수동 구성원").clicked() { group.membership = None; }
                    for (id, name) in &choices {
                        if ui.selectable_label(group.membership.as_ref().is_some_and(|membership| &membership.collection == id), name).clicked() {
                            if let Some(membership) = &mut group.membership { membership.collection.clone_from(id); }
                            else { group.membership = Some(Box::new(Membership { collection: id.clone(), role: "member".into(), generated: BTreeMap::new(), retired: BTreeMap::new(), weights: BTreeMap::new(), include: BTreeSet::new(), exclude: BTreeSet::new() })); }
                        }
                    }
                });
                if let Some(membership) = &mut group.membership {
                    ui.horizontal(|ui| { ui.label("배치 로컬 역할"); ui.text_edit_singleline(&mut membership.role); });
                    for window in self.draft.windows.values() { ui.horizontal(|ui| {
                        ui.label(&window.alias);
                        let mut include = membership.include.contains(&window.id); let mut exclude = membership.exclude.contains(&window.id);
                        if ui.checkbox(&mut include, "이 그룹에 포함").changed() { if include { membership.include.insert(window.id.clone()); } else { membership.include.remove(&window.id); } }
                        if ui.checkbox(&mut exclude, "이 그룹에서 제외").changed() { if exclude { membership.exclude.insert(window.id.clone()); } else { membership.exclude.remove(&window.id); } }
                        if exclude { ui.small("로컬 제외가 우선 · 해제 후 구성원 계산으로 복원"); }
                    }); }
                }
            }
            if ui.button("구성원 변경 계산 → 초안에 반영").clicked() {
                match self.draft.stage_memberships() {
                    Ok((draft, delta)) => { self.draft = draft; self.notice = format!("구성원 초안: 추가 {} / 제외 {}. 구조 저장 후 전환 미리보기로 확인하세요.", delta.added.len(), delta.removed.len()); }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
        });
    }
    fn temporary_filter_tools(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("임시 필터 · 저장된 배치에 영향 없음").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("태그 필터").clicked() { self.transient_filter = Some(Query::Tag(String::new())); }
                if ui.button("앱 힌트 필터").clicked() { self.transient_filter = Some(Query::Application(String::new())); }
                if ui.button("임시 필터 해제 미리보기").clicked() { self.transient_filter = None; if let Some(target) = self.target() { self.request_preview(Request::open(&self.config, target), false); } }
            });
            if let Some(query) = &mut self.transient_filter { query_editor(ui, query); }
            if let Some(query) = self.transient_filter.clone() { ui.horizontal(|ui| {
                ui.label("새 배치 이름"); ui.text_edit_singleline(&mut self.name);
                if ui.button("필터 결과를 독립 배치로 저장").clicked() { let mut draft = self.config.clone(); match draft.save_filtered_view(&self.selected_view, self.name.clone(), &query) {
                    Ok(id) => { if self.commit(draft) { self.select_view(id); self.transient_filter = None; self.notice = "필터 결과를 독립 배치로 저장했습니다. 창 콘텐츠는 공유하며 실제 창은 변경되지 않았습니다.".into(); } },
                    Err(error) => self.error = Some(error.to_string()),
                } }
            }); }
            ui.small("필터 편집은 창을 바꾸지 않습니다. 전환 미리보기 → 적용으로 확인하세요. unknown은 제외됩니다.");
        });
    }

    fn fit_actions(&mut self, ui: &mut egui::Ui) {
        if self.error.is_some()
            || self.preview.as_ref().is_some_and(|plan| {
                !plan.blocked.is_empty()
                    || plan.diagnostics.iter().any(|message| message.contains("retained"))
            })
        {
            ui.horizontal_wrapped(|ui| {
                if ui.button("임시 유지 해제·저장 규칙으로 다시 미리보기").clicked()
                    && let Some(mut request) = self.last_transition.clone()
                {
                    request.id = new_id("restore-preview");
                    request.mode = TransitionMode::Restore;
                    request.retain.clear();
                    request.focus = None;
                    self.request_preview(request, false);
                }
                if ui.button("접기·최소 크기·그룹 규칙 편집").clicked() {
                    self.page = 1;
                }
            });
            ui.small("다시 미리보기는 현재 범위를 유지합니다. 잠금·최소 크기·출력 보호는 그대로 검사합니다. 실패 영역은 관리 재개 후 다시 미리보세요.");
        }
    }

    fn transition_bar(&mut self, ui: &mut egui::Ui) {
        self.temporary_filter_tools(ui);
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("mode").selected_text(mode_label(self.mode)).show_ui(
                ui,
                |ui| {
                    for mode in [
                        TransitionMode::Open,
                        TransitionMode::KeepSize,
                        TransitionMode::KeepHere,
                        TransitionMode::Bring,
                        TransitionMode::Restore,
                    ] {
                        ui.selectable_value(&mut self.mode, mode, mode_label(mode));
                    }
                },
            );
            ui.checkbox(&mut self.focus_target, "선택한 창에 포커스 요청");
            if ui
                .add_enabled(
                    !self.applying && !self.runtime.paused && self.safe_mode.is_none(),
                    egui::Button::new("전환 미리보기"),
                )
                .clicked()
            {
                self.preview_selected();
            }
            if ui
                .add_enabled(
                    !self.applying && self.preview.is_some(),
                    egui::Button::new("계획 적용").fill(Color32::from_rgb(41, 91, 145)),
                )
                .clicked()
            {
                self.apply_preview();
            }
            if self.applying {
                ui.spinner();
                ui.label("적용 / 관찰 중");
            }
        });
        if let Some(plan) = self.preview.clone() {
            ui.label(format!(
                "그대로 {} · 이동만 {} · 크기 변경 {} · 표시 {} · 숨김 {} · 포커스 {}",
                plan.impact.unchanged,
                plan.impact.moved,
                plan.impact.resized,
                plan.impact.shown,
                plan.impact.hidden,
                plan.impact.focus_requests
            ));
            ui.small(format!(
                "범위: {}",
                plan.scope
                    .iter()
                    .map(|id| self
                        .config
                        .slots
                        .get(id)
                        .map_or(id.as_str(), |slot| slot.name.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            for diagnostic in &plan.diagnostics {
                ui.colored_label(Color32::YELLOW, diagnostic);
            }
            draw_preview(ui, &plan, &self.config, &self.snapshot);
            for desired in plan.desired.values() {
                if let Some(observed) = self.snapshot.windows.get(&desired.window).cloned() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            self.config
                                .windows
                                .get(&desired.window)
                                .map_or(desired.window.as_str(), |window| window.alias.as_str()),
                        );
                        ui.small(format!(
                            "관찰된 클라이언트 {}×{} / {:?}",
                            observed.client[0], observed.client[1], observed.show_state
                        ));
                    });
                }
            }
        }
        self.fit_actions(ui);
        self.active_property_tools(ui);
        if let Some(result) = &self.result {
            for outcome in &result.windows {
                if let Some(error) = &outcome.error {
                    ui.colored_label(Color32::YELLOW, error.to_string());
                }
            }
        }
    }
    fn save_here(
        &mut self,
        view: &str,
        desired: &Desired,
        observed: &ObservedWindow,
        position: bool,
    ) {
        let mut draft = self.config.clone();
        let scale = f64::from(observed.dpi) / 96.0;
        let coordinates = [
            f64::from(observed.frame.x - desired.allocated.x) / scale,
            f64::from(observed.frame.y - desired.allocated.y) / scale,
        ];
        let size = [f64::from(observed.client[0]) / scale, f64::from(observed.client[1]) / scale];
        match draft.save_properties(
            view,
            &desired.placement,
            &desired.context,
            position.then_some(coordinates),
            (!position).then_some(size),
        ) {
            Ok(()) => {
                if self.commit(draft) {
                    self.send(Command::Promote(
                        desired.slot.clone(),
                        desired.window.clone(),
                        position,
                        !position,
                    ));
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn active_property_tools(&mut self, ui: &mut egui::Ui) {
        let active: Vec<_> =
            self.runtime.geometry.values().filter(|desired| !desired.carried).cloned().collect();
        egui::CollapsingHeader::new("활성 배치의 현재 속성 저장").show(ui, |ui| {
            for desired in active {
                if let Some(observed) = self.snapshot.windows.get(&desired.window).cloned()
                    && let Some(presentation) =
                        self.runtime.presentations.get(&desired.slot).cloned()
                {
                    ui.horizontal(|ui| {
                        ui.label(
                            self.config
                                .windows
                                .get(&desired.window)
                                .map_or(desired.window.as_str(), |window| window.alias.as_str()),
                        );
                        let compatible = observed.dpi == presentation.context_dpi
                            && observed.display == presentation.context_display
                            && observed.show_state == window_manager_core::ShowState::Normal;
                        if ui
                            .add_enabled(compatible, egui::Button::new("현재 크기만 저장"))
                            .clicked()
                        {
                            self.save_here(&presentation.view, &desired, &observed, false);
                        }
                        if ui
                            .add_enabled(compatible, egui::Button::new("현재 위치만 저장"))
                            .clicked()
                        {
                            self.save_here(&presentation.view, &desired, &observed, true);
                        }
                        if ui
                            .add_enabled(
                                compatible,
                                egui::Button::new("크기 수식을 현재 값으로 대체"),
                            )
                            .clicked()
                        {
                            let scale = f64::from(observed.dpi) / 96.0;
                            let mut draft = self.config.clone();
                            match draft.replace_size_formulas(
                                &presentation.view,
                                &desired.placement,
                                &desired.context,
                                observed.client.map(|value| f64::from(value) / scale),
                            ) {
                                Ok(()) => {
                                    self.commit(draft);
                                }
                                Err(error) => self.error = Some(error.to_string()),
                            }
                        }
                        if ui.button("저장된 크기 규칙을 기본 프리셋으로 복사").clicked()
                        {
                            let mut draft = self.config.clone();
                            match draft.promote_size_rule(
                                &presentation.view,
                                &desired.placement,
                                &desired.context,
                            ) {
                                Ok(()) => {
                                    self.commit(draft);
                                }
                                Err(error) => self.error = Some(error.to_string()),
                            }
                        }
                        if !compatible {
                            ui.small("화면·DPI·일반 상태 확인 후 저장");
                        }
                    });
                }
            }
        });
    }
}

impl eframe::App for Manager {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll();
        #[cfg(feature = "ui-smoke")]
        self.capture_smoke(ctx);
        if !self.guard_control(ctx) {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(200));
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Window Manager");
                ui.label(RichText::new("독립 배치 · 선택 영역 전환").weak());
                if ui
                    .button(if self.runtime.paused {
                        "관리 재개"
                    } else {
                        "관리 일시 정지"
                    })
                    .clicked()
                {
                    self.send(Command::Pause(!self.runtime.paused));
                }
                if ui.button("숨긴 창 복구").clicked() {
                    self.send(Command::Recover);
                }
                if ui
                    .add_enabled(!self.applying, egui::Button::new("창 배치 되돌리기 미리보기"))
                    .clicked()
                {
                    let id = new_id("undo");
                    self.latest_request = Some(id.clone());
                    self.preview = None;
                    self.apply_when_previewed = false;
                    self.send(Command::Undo(id));
                }
                if !self.runtime.suspended.is_empty() && ui.button("실패 영역 관리 재개").clicked()
                {
                    self.send(Command::Pause(false));
                }
                if ui
                    .add_enabled(!self.undo.is_empty(), egui::Button::new("설정 편집 되돌리기"))
                    .clicked()
                    && let Some(mut previous) = self.undo.pop()
                {
                    previous.revision = self.config.revision + 1;
                    match previous.save(&self.path) {
                        Ok(()) => {
                            self.config = previous;
                            self.draft = self.config.clone();
                            self.preview = None;
                            self.send(Command::Refresh(self.config.clone()));
                        }
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
            });
            if let Some(error) = &self.safe_mode {
                ui.colored_label(Color32::LIGHT_RED, format!("안전 모드: {error}"));
            }
            if let Some(error) = &self.error {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            ui.small(&self.notice);
            for (slot, reason) in &self.runtime.pending_reflow {
                ui.small(format!(
                    "규칙 재배치 대기: {} · {reason}",
                    self.config.slots.get(slot).map_or(slot.as_str(), |slot| slot.name.as_str())
                ));
            }
            self.show_attention(ui);
        });
        egui::SidePanel::left("workspace")
            .resizable(true)
            .default_width(245.0)
            .show(ctx, |ui| self.sidebar(ui));
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (index, label) in
                    ["창 목록", "배치 / 그룹", "화면 / 영역", "라이브러리"].iter().enumerate()
                {
                    ui.selectable_value(&mut self.page, index, *label);
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                match self.page {
                    1 => self.editor(ui),
                    2 => self.slots(ui),
                    3 => self.library(ui),
                    _ => self.inventory(ui),
                }
                ui.separator();
                self.transition_bar(ui);
            });
        });
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.worker.shutdown();
    }
}

const fn mode_label(mode: TransitionMode) -> &'static str {
    match mode {
        TransitionMode::Open => "저장된 배치 열기",
        TransitionMode::KeepSize => "선택한 창 크기 유지",
        TransitionMode::KeepHere => "여기 유지 · 주변만 전환",
        TransitionMode::Bring => "현재 크기로 잠시 가져오기",
        TransitionMode::Reflow => "연속 규칙 재계산",
        TransitionMode::Restore => "저장된 배치로 복원",
    }
}
fn optional_pair(ui: &mut egui::Ui, label: &str, value: &mut Option<[f64; 2]>, default: [f64; 2]) {
    ui.horizontal(|ui| {
        let mut enabled = value.is_some();
        if ui.checkbox(&mut enabled, label).changed() {
            *value = enabled.then_some(default);
        }
        if let Some(pair) = value {
            for axis in pair {
                ui.add(egui::DragValue::new(axis).speed(1.0));
            }
        }
    });
}
fn protection_editor(ui: &mut egui::Ui, protection: &mut Protection) {
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut protection.geometry_lock, "위치·크기 잠금");
        ui.checkbox(&mut protection.maintain_visible, "계속 표시");
        ui.checkbox(&mut protection.keep_monitor, "모니터 유지");
        ui.checkbox(&mut protection.prohibit_focus, "포커스 금지");
    });
}
fn optional_number(ui: &mut egui::Ui, label: &str, value: &mut Option<f64>, default: f64) {
    ui.horizontal(|ui| {
        let mut enabled = value.is_some();
        if ui.checkbox(&mut enabled, label).changed() {
            *value = enabled.then_some(default);
        }
        if let Some(number) = value {
            ui.add(egui::DragValue::new(number).range(1.0..=65_536.0));
        }
    });
}
fn optional_formula(ui: &mut egui::Ui, label: &str, value: &mut Option<String>) {
    ui.horizontal(|ui| {
        let mut enabled = value.is_some();
        if ui.checkbox(&mut enabled, label).changed() {
            *value = enabled.then(|| "available_width".into());
        }
        if let Some(formula) = value {
            ui.text_edit_singleline(formula);
        }
    });
}
fn query_editor(ui: &mut egui::Ui, query: &mut Query) {
    query_node(ui, query, 0);
}
fn query_node(ui: &mut egui::Ui, query: &mut Query, depth: usize) {
    if depth >= 32 {
        ui.colored_label(Color32::LIGHT_RED, "조건 깊이 한도: 하위 조건을 더 추가할 수 없습니다.");
        return;
    }
    let original = match query {
        Query::All => 0,
        Query::Tag(_) => 1,
        Query::Application(_) => 2,
        Query::Alias(_) => 3,
        Query::Private => 4,
        Query::Not(_) => 5,
        Query::And(_) => 6,
        Query::Or(_) => 7,
    };
    let mut kind = original;
    egui::ComboBox::from_id_salt("query-kind")
        .selected_text(
            ["전체", "태그", "앱 힌트", "별칭 포함", "개인 분류", "NOT", "AND", "OR"][kind],
        )
        .show_ui(ui, |ui| {
            for (index, label) in
                ["전체", "태그", "앱 힌트", "별칭 포함", "개인 분류", "NOT", "AND", "OR"]
                    .iter()
                    .enumerate()
            {
                ui.selectable_value(&mut kind, index, *label);
            }
        });
    if kind != original {
        *query = match kind {
            1 => Query::Tag(String::new()),
            2 => Query::Application(String::new()),
            3 => Query::Alias(String::new()),
            4 => Query::Private,
            5 => Query::Not(Box::new(query.clone())),
            6 => Query::And(vec![query.clone()]),
            7 => Query::Or(vec![query.clone()]),
            _ => Query::All,
        };
    }
    match query {
        Query::Tag(value) | Query::Application(value) | Query::Alias(value) => {
            ui.text_edit_singleline(value);
        }
        Query::Not(inner) => {
            ui.small("NOT: unknown은 unknown으로 유지");
            ui.indent("query-not", |ui| query_node(ui, inner, depth + 1));
        }
        Query::And(queries) | Query::Or(queries) => {
            let mut remove = None;
            for (index, child) in queries.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        query_node(ui, child, depth + 1);
                        if ui.small_button("이 하위 조건 제거").clicked() {
                            remove = Some(index);
                        }
                    });
                });
            }
            if let Some(index) = remove {
                queries.remove(index);
            }
            if ui.add_enabled(queries.len() < 256, egui::Button::new("+ 하위 태그 조건")).clicked()
            {
                queries.push(Query::Tag(String::new()));
            }
            if queries.is_empty() {
                ui.small(if kind == 6 {
                    "빈 AND는 전체를 선택합니다."
                } else {
                    "빈 OR는 아무 창도 선택하지 않습니다."
                });
            }
        }
        Query::All => {
            ui.label("등록된 모든 창");
        }
        Query::Private => {
            ui.label("공급자 분류 증거 없음: unknown");
        }
    }
}
fn find_group_mut<'a>(node: &'a mut Node, id: &str) -> Option<&'a mut Group> {
    match node {
        Node::Placement(_) => None,
        Node::Group(group) => {
            if group.id == id {
                Some(group)
            } else {
                group.children.iter_mut().find_map(|child| find_group_mut(child, id))
            }
        }
    }
}
#[allow(clippy::too_many_lines)] // Recursive structural editing is displayed with its exact local scope.
fn tree_editor(
    ui: &mut egui::Ui,
    node: &mut Node,
    selected: &mut Id,
    tab_action: &mut Option<(Id, Id)>,
    slots: &BTreeMap<Id, DisplaySlot>,
    selected_slot: &str,
    depth: usize,
) {
    match node {
        Node::Placement(placement) => {
            protection_editor(ui, &mut placement.protection);
            ui.horizontal(|ui| {
                ui.label("창 역할");
                ui.text_edit_singleline(&mut placement.role);
                ui.small(&placement.window);
            });
            egui::CollapsingHeader::new("이 배치의 기본 크기 수식 · 새 화면 문맥에 사용")
                .id_salt((&placement.id, "default-size"))
                .show(ui, |ui| {
                    optional_pair(
                        ui,
                        "기본 클라이언트 크기",
                        &mut placement.default_preference.client_size,
                        [600.0, 400.0],
                    );
                    optional_formula(
                        ui,
                        "기본 너비 수식",
                        &mut placement.default_preference.width_formula,
                    );
                    optional_formula(
                        ui,
                        "기본 높이 수식",
                        &mut placement.default_preference.height_formula,
                    );
                    ui.small("이미 저장된 화면 문맥의 규칙은 독립적으로 유지됩니다.");
                });
            if let Some(slot) = slots.get(selected_slot) {
                let keys: Vec<_> = std::iter::once(context_key(slot, "base"))
                    .chain(placement.preferences.keys().cloned())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                for key in keys {
                    egui::CollapsingHeader::new(format!("크기·위치 규칙 {key}"))
                        .id_salt((&placement.id, &key))
                        .show(ui, |ui| {
                            let preference = placement
                                .preferences
                                .entry(key)
                                .or_insert_with(|| placement.default_preference.clone());
                            optional_pair(
                                ui,
                                "클라이언트 크기",
                                &mut preference.client_size,
                                [600.0, 400.0],
                            );
                            optional_pair(ui, "그룹 내 위치", &mut preference.position, [0.0, 0.0]);
                            optional_formula(ui, "너비 수식", &mut preference.width_formula);
                            optional_formula(ui, "높이 수식", &mut preference.height_formula);
                            optional_pair(
                                ui,
                                "최소 크기",
                                &mut placement.minimum_client,
                                [100.0, 80.0],
                            );
                            if ui.button("이 문맥의 수동 덮어쓰기 해제").clicked() {
                                preference.position_override = None;
                                preference.size_override = None;
                            }
                        });
                }
            }
        }
        Node::Group(group) => {
            egui::CollapsingHeader::new(&group.name)
                .id_salt(&group.id)
                .default_open(depth < 3)
                .show(ui, |ui| {
                    protection_editor(ui, &mut group.protection);
                    ui.horizontal(|ui| {
                        if ui.selectable_label(selected == &group.id, "이 그룹 선택").clicked()
                        {
                            selected.clone_from(&group.id);
                        }
                        ui.text_edit_singleline(&mut group.name);
                    });
                    egui::ComboBox::from_id_salt(&group.id)
                        .selected_text(format!("{:?}", group.strategy))
                        .show_ui(ui, |ui| {
                            for strategy in [
                                Strategy::Horizontal,
                                Strategy::Vertical,
                                Strategy::Grid,
                                Strategy::Flow,
                                Strategy::Free,
                                Strategy::SemanticTabs,
                                Strategy::ResponsiveTabs,
                            ] {
                                ui.selectable_value(
                                    &mut group.strategy,
                                    strategy,
                                    format!("{strategy:?}"),
                                );
                            }
                        });
                    egui::ComboBox::from_id_salt((&group.id, "reflow-policy")).selected_text(format!("재배치 정책: {:?}", group.reflow)).show_ui(ui, |ui| {
                        for (policy, label) in [(window_manager_core::ReflowPolicy::ExistingPositionFirst, "기존 위치 우선 (기본)"), (window_manager_core::ReflowPolicy::OnEntryOrRequest, "진입·명시 요청 시 재배치"), (window_manager_core::ReflowPolicy::ContinuousRule, "연속 규칙 · 감지된 상호작용 중 보류")] { ui.selectable_value(&mut group.reflow, policy, label); }
                    });
                    ui.small("구성원 계산·저장은 별도 단계입니다. 연속 규칙은 공개/확장/보호 영역과 전경 관리 창·드래그·모달 중 보류합니다. 앱 내부 입력 상태는 완전히 감지할 수 없습니다.");
                    ui.horizontal(|ui| { ui.label("자동 선택 규칙 우선순위 (큰 값 우선)"); ui.add(egui::DragValue::new(&mut group.rule_priority).range(-1_000_000..=1_000_000)); });
                    ui.horizontal(|ui| {
                        ui.label("간격 수식");
                        ui.text_edit_singleline(&mut group.gap);
                        ui.checkbox(&mut group.preserve_child_sizes, "자식 크기 유지");
                    });
                    egui::ComboBox::from_id_salt((&group.id, "preservation"))
                        .selected_text(format!("그룹 보존: {:?}", group.preservation))
                        .show_ui(ui, |ui| {
                            for mode in [
                                window_manager_core::GroupPreservation::Fill,
                                window_manager_core::GroupPreservation::Outer,
                                window_manager_core::GroupPreservation::Children,
                                window_manager_core::GroupPreservation::Arrangement,
                            ] {
                                ui.selectable_value(
                                    &mut group.preservation,
                                    mode,
                                    format!("{mode:?}"),
                                );
                            }
                        });
                    egui::ComboBox::from_id_salt((&group.id, "alignment"))
                        .selected_text(format!("여백 정렬: {:?}", group.alignment))
                        .show_ui(ui, |ui| {
                            for alignment in [
                                window_manager_core::Alignment::Start,
                                window_manager_core::Alignment::Center,
                                window_manager_core::Alignment::End,
                            ] {
                                ui.selectable_value(
                                    &mut group.alignment,
                                    alignment,
                                    format!("{alignment:?}"),
                                );
                            }
                        });
                    if group.strategy != Strategy::SemanticTabs {
                        let mut fallback = !group.allowed_fallbacks.is_empty();
                        if ui
                            .checkbox(&mut fallback, "맞지 않으면 줄 바꿈 → 반응형 탭 순서로 시도")
                            .changed()
                        {
                            group.allowed_fallbacks = if fallback {
                                vec![Strategy::Flow, Strategy::ResponsiveTabs]
                            } else {
                                Vec::new()
                            };
                        }
                    }
                    optional_formula(
                        ui,
                        "자식 정렬 우선순위 (작은 값 먼저)",
                        &mut group.sort_formula,
                    );
                    if group.strategy == Strategy::Grid {
                        ui.horizontal(|ui| {
                            ui.label("열 수식");
                            ui.text_edit_singleline(&mut group.columns);
                        });
                    }
                    if ui.button("좁을 때 탭으로 접기 (720 logical)").clicked()
                        && group.variants.is_empty()
                    {
                        group.variants.push(Variant {
                            id: "compact".into(),
                            condition: None,
                            below_width: Some(720.0),
                            below_height: None,
                            hysteresis: 24.0,
                            strategy: Strategy::ResponsiveTabs,
                            ratios: Vec::new(),
                        });
                    }
                    let mut remove_variant = None;
                    for (index, variant) in group.variants.iter_mut().enumerate() {
                        ui.push_id((&group.id, &variant.id), |ui| {
                            ui.label(format!("반응형 문맥: {}", variant.id));
                            optional_number(ui, "너비 미만", &mut variant.below_width, 720.0);
                            optional_number(ui, "높이 미만", &mut variant.below_height, 500.0);
                            ui.horizontal(|ui| {
                                let mut enabled = variant.condition.is_some();
                                if ui.checkbox(&mut enabled, "조건 수식").changed() {
                                    variant.condition =
                                        enabled.then(|| "available_width < 720".into());
                                }
                                if let Some(condition) = variant.condition.as_mut() {
                                    ui.text_edit_singleline(condition);
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("복귀 여유");
                                ui.add(
                                    egui::DragValue::new(&mut variant.hysteresis)
                                        .range(0.0..=4096.0),
                                );
                                for strategy in [
                                    Strategy::ResponsiveTabs,
                                    Strategy::Flow,
                                    Strategy::Grid,
                                    Strategy::Horizontal,
                                    Strategy::Vertical,
                                ] {
                                    ui.selectable_value(
                                        &mut variant.strategy,
                                        strategy,
                                        format!("{strategy:?}"),
                                    );
                                }
                                if ui.button("문맥 삭제").clicked() {
                                    remove_variant = Some(index);
                                }
                            });
                        });
                    }
                    if let Some(index) = remove_variant {
                        group.variants.remove(index);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("분할 비율 편집").clicked()
                            && group.ratios.len() != group.children.len()
                        {
                            group.ratios.resize(group.children.len(), 1.0);
                        }
                        for ratio in &mut group.ratios {
                            ui.add(egui::DragValue::new(ratio).speed(0.1).range(0.01..=1000.0));
                        }
                    });
                    let mut remove = None;
                    let mut reorder = None;
                    let mut unwrap = None;
                    for (index, child) in group.children.iter_mut().enumerate() {
                        ui.push_id(child.id().to_owned(), |ui| {
                            ui.horizontal(|ui| {
                                if ui.add_enabled(index > 0, egui::Button::new("↑")).clicked() {
                                    reorder = Some(index);
                                }
                                if ui.button("참조 제거").clicked() {
                                    remove = Some(index);
                                }
                                if matches!(child, Node::Group(_))
                                    && ui.button("그룹 풀기").clicked()
                                {
                                    unwrap = Some(index);
                                }
                                if matches!(
                                    group.strategy,
                                    Strategy::SemanticTabs | Strategy::ResponsiveTabs
                                ) && ui.button("이 탭 미리보기").clicked()
                                {
                                    *tab_action = Some((group.id.clone(), child.id().into()));
                                }
                            });
                            tree_editor(
                                ui,
                                child,
                                selected,
                                tab_action,
                                slots,
                                selected_slot,
                                depth + 1,
                            );
                        });
                        ui.separator();
                    }
                    if let Some(index) = remove {
                        let (removed, _) = group.take_child(index);
                        if let Some(membership) = &mut group.membership {
                            membership.generated.retain(|_, id| id != removed.id());
                            if let Node::Placement(placement) = removed {
                                membership.retired.remove(&placement.window);
                                membership.weights.remove(&placement.window);
                                membership.include.remove(&placement.window);
                                membership.exclude.insert(placement.window);
                            }
                        }
                    } else if let Some(index) = unwrap {
                        let (child, weights) = group.take_child(index);
                        if let Node::Group(child) = child {
                            let weights = weights.divided(child.children.len());
                            for (offset, node) in child.children.into_iter().enumerate() {
                                group.insert_child(index + offset, node, &weights);
                            }
                        }
                    } else if let Some(index) = reorder {
                        group.children.swap(index, index - 1);
                        if group.ratios.len() > index {
                            group.ratios.swap(index, index - 1);
                        }
                        for variant in &mut group.variants {
                            if variant.ratios.len() > index {
                                variant.ratios.swap(index, index - 1);
                            }
                        }
                    }
                });
        }
    }
}
#[allow(clippy::cast_precision_loss)] // The miniature preview is approximate; planning uses exact integer pixels.
fn draw_preview(ui: &mut egui::Ui, plan: &Plan, config: &Configuration, snapshot: &Snapshot) {
    let bounds: Vec<_> = plan
        .scope
        .iter()
        .filter_map(|id| config.slots.get(id).and_then(|slot| slot_bounds(slot, snapshot).ok()))
        .collect();
    draw_rectangles(ui, &bounds, plan.desired.values(), config);
}

#[allow(clippy::cast_precision_loss)] // Geometry is an approximate UI miniature; the report retains exact pixels.
fn draw_rectangles<'a>(
    ui: &mut egui::Ui,
    bounds: &[Rect],
    desired: impl Iterator<Item = &'a Desired>,
    config: &Configuration,
) {
    let (response, painter) = ui
        .allocate_painter(egui::vec2(ui.available_width().min(760.0), 180.0), egui::Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 6.0, Color32::from_rgb(12, 16, 23));
    if bounds.is_empty() {
        return;
    }
    let min_x = bounds.iter().map(|rect| rect.x).min().unwrap_or_default();
    let min_y = bounds.iter().map(|rect| rect.y).min().unwrap_or_default();
    let max_x = bounds.iter().map(|rect| rect.x + rect.width).max().unwrap_or(1);
    let max_y = bounds.iter().map(|rect| rect.y + rect.height).max().unwrap_or(1);
    let scale = ((rect.width() - 20.0) / (max_x - min_x).max(1) as f32)
        .min((rect.height() - 20.0) / (max_y - min_y).max(1) as f32);
    for desired in desired {
        let frame = desired.frame;
        let min = rect.min
            + egui::vec2(
                ((frame.x - min_x) as f32).mul_add(scale, 10.0),
                ((frame.y - min_y) as f32).mul_add(scale, 10.0),
            );
        let tile = egui::Rect::from_min_size(
            min,
            egui::vec2(frame.width as f32 * scale, frame.height as f32 * scale),
        );
        painter.rect_filled(
            tile.shrink(1.0),
            3.0,
            if desired.strict_size {
                Color32::from_rgb(86, 78, 47)
            } else {
                Color32::from_rgb(37, 64, 96)
            },
        );
        let label =
            config.windows.get(&desired.window).map_or("창", |window| window.alias.as_str());
        let mut job = egui::text::LayoutJob::simple_singleline(
            label.into(),
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
        job.wrap.max_width = (tile.width() - 4.0).max(0.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let text = painter.layout_job(job);
        painter.with_clip_rect(painter.clip_rect().intersect(tile.shrink(2.0))).galley(
            tile.center() - text.size() * 0.5,
            text,
            Color32::WHITE,
        );
        ui.interact(
            tile,
            egui::Id::new(("preview-window", &desired.placement)),
            egui::Sense::hover(),
        )
        .on_hover_text(label);
    }
}

fn collect_nodes(
    node: &Node,
    view: &str,
    prefix: &str,
    output: &mut Vec<(window_manager_core::NodeAddress, String, bool)>,
) {
    let (name, placement) = match node {
        Node::Group(group) => (&group.name, false),
        Node::Placement(placement) => (&placement.role, true),
    };
    let label = format!("{prefix} / {name}");
    output.push((
        window_manager_core::NodeAddress { view: view.into(), node: node.id().into() },
        label.clone(),
        placement,
    ));
    if let Node::Group(group) = node {
        for child in &group.children {
            collect_nodes(child, view, &label, output);
        }
    }
}
