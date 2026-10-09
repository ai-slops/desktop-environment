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
        Self { path, _lock: lock, watchdog: watchdog.ok(), draft: config.clone(), config, safe_mode, worker, snapshot: Snapshot::default(), runtime: Runtime::default(), inventory: Vec::new(), selected_view, selected_root, control_position: None, settled_control: None, selected_slot, selected_group, selected_window: None, retained: BTreeSet::new(), mode: TransitionMode::Open, focus_target: false, search: String::new(), page: 0, name: String::new(), display_choice: String::new(), region: [0.0, 0.0, 1.0, 1.0], undo: Vec::new(), preview: None, latest_request: None, apply_when_previewed: false, pending_requests: Vec::new(), applying: false, error, notice: "영역을 만들고 창을 추가한 뒤 미리보기로 시작하세요. 저장된 배치는 자동 적용하지 않습니다.".into(), result: None,
            tag_input: String::new(),
            #[cfg(feature = "ui-smoke")]
            screenshot: std::env::var_os("WINDOW_MANAGER_SCREENSHOT").map(|path| (PathBuf::from(path), std::time::Instant::now(), false)),
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
        if !self.config.slots.is_empty()
            && (window_manager_core::control_bounds(&self.config, &self.snapshot)
                .is_none_or(|bounds| self.settled_control != Some(bounds)))
        {
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
                    let settled = result.status == Status::Settled;
                    self.result = Some(result);
                    self.preview = None;
                    if settled && !self.pending_requests.is_empty() {
                        let (request, apply) = self.pending_requests.remove(0);
                        self.request_preview(request, apply);
                    } else if !settled {
                        self.pending_requests.clear();
                    }
                }
                Event::Shortcut(number) => {
                    if let Some(shortcut) =
                        self.config.shortcuts.iter().find(|shortcut| shortcut.number == number)
                    {
                        self.request_preview(
                            Request::open(&self.config, shortcut.target.clone()),
                            true,
                        );
                    }
                }
                Event::Error(error) => {
                    self.error = Some(error.to_string());
                    self.applying = false;
                    self.apply_when_previewed = false;
                }
                Event::Notice(notice) => self.notice = notice,
                Event::Barrier(_) => {}
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
            self.pending_requests.push((request, apply));
            return;
        }
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
    fn editor(&mut self, ui: &mut egui::Ui) {
        ui.heading("배치 편집");
        ui.label("구조 저장은 실제 창을 이동하지 않습니다. 미리보기와 적용으로 반영하세요.");
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
        ui.label("고정 단축키: 현재 배치 ID와 영역에 연결 (저장 후 앱 재시작)");
        ui.horizontal(|ui| {
            for number in 1..=9 {
                if ui.button(format!("{number}")).clicked()
                    && let Some(target) = self.target()
                {
                    let mut draft = self.config.clone();
                    draft.shortcuts.retain(|shortcut| shortcut.number != number);
                    draft.shortcuts.push(Shortcut { number, target });
                    self.commit(draft);
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
                            else { group.membership = Some(Membership { collection: id.clone(), role: "member".into(), generated: BTreeMap::new(), retired: BTreeMap::new() }); }
                        }
                    }
                });
                if let Some(membership) = &mut group.membership { ui.horizontal(|ui| { ui.label("배치 로컬 역할"); ui.text_edit_singleline(&mut membership.role); }); }
            }
            if ui.button("구성원 변경 계산 → 초안에 반영").clicked() {
                match self.draft.stage_memberships() {
                    Ok((draft, delta)) => { self.draft = draft; self.notice = format!("구성원 초안: 추가 {} / 제외 {}. 구조 저장 후 전환 미리보기로 확인하세요.", delta.added.len(), delta.removed.len()); }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
        });
    }
    fn transition_bar(&mut self, ui: &mut egui::Ui) {
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
                    ui.horizontal(|ui| {
                        ui.label(
                            self.config
                                .windows
                                .get(&desired.window)
                                .map_or(desired.window.as_str(), |window| window.alias.as_str()),
                        );
                        if !desired.carried && ui.button("현재 크기만 저장").clicked() {
                            self.save_here(&plan, desired, &observed, false);
                        }
                        if !desired.carried && ui.button("현재 위치만 저장").clicked() {
                            self.save_here(&plan, desired, &observed, true);
                        }
                    });
                }
            }
        }
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
        plan: &Plan,
        desired: &Desired,
        observed: &ObservedWindow,
        position: bool,
    ) {
        let Some(presentation) = plan.presentations.get(&desired.slot) else {
            return;
        };
        let mut draft = self.config.clone();
        let scale = f64::from(observed.dpi) / 96.0;
        let coordinates = [
            f64::from(observed.frame.x - desired.allocated.x) / scale,
            f64::from(observed.frame.y - desired.allocated.y) / scale,
        ];
        let size = [f64::from(observed.client[0]) / scale, f64::from(observed.client[1]) / scale];
        match draft.save_properties(
            &presentation.view,
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
}

impl eframe::App for Manager {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll();
        if !self.guard_control(ctx) {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(200));
        #[cfg(feature = "ui-smoke")]
        self.capture_smoke(ctx);
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
            self.show_attention(ui);
        });
        egui::SidePanel::left("workspace")
            .resizable(true)
            .default_width(245.0)
            .show(ctx, |ui| self.sidebar(ui));
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (index, label) in ["창 목록", "배치 / 그룹", "화면 / 영역"].iter().enumerate()
                {
                    ui.selectable_value(&mut self.page, index, *label);
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                match self.page {
                    1 => self.editor(ui),
                    2 => self.slots(ui),
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
    match query {
        Query::Tag(value) | Query::Application(value) => {
            ui.text_edit_singleline(value);
        }
        Query::Not(inner) => {
            ui.label("NOT (unknown은 그대로 unknown)");
            query_editor(ui, inner);
        }
        Query::And(queries) | Query::Or(queries) => {
            for query in queries {
                query_editor(ui, query);
            }
        }
        Query::All => {
            ui.label("등록된 모든 창");
        }
        Query::Private => {
            ui.label("공급자 증거 없음: unknown");
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
                            let preference = placement.preferences.entry(key).or_default();
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
                        let removed = group.children.remove(index);
                        if let Some(membership) = &mut group.membership {
                            membership.generated.retain(|_, id| id != removed.id());
                        }
                    } else if let Some(index) = unwrap {
                        if let Node::Group(child) = group.children.remove(index) {
                            group.children.splice(index..index, child.children);
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
    let (response, painter) = ui
        .allocate_painter(egui::vec2(ui.available_width().min(760.0), 180.0), egui::Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 6.0, Color32::from_rgb(12, 16, 23));
    let bounds: Vec<_> = plan
        .scope
        .iter()
        .filter_map(|id| config.slots.get(id).and_then(|slot| slot_bounds(slot, snapshot).ok()))
        .collect();
    if bounds.is_empty() {
        return;
    }
    let min_x = bounds.iter().map(|rect| rect.x).min().unwrap_or_default();
    let min_y = bounds.iter().map(|rect| rect.y).min().unwrap_or_default();
    let max_x = bounds.iter().map(|rect| rect.x + rect.width).max().unwrap_or(1);
    let max_y = bounds.iter().map(|rect| rect.y + rect.height).max().unwrap_or(1);
    let scale = ((rect.width() - 20.0) / (max_x - min_x).max(1) as f32)
        .min((rect.height() - 20.0) / (max_y - min_y).max(1) as f32);
    for desired in plan.desired.values() {
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
        painter.text(
            tile.center(),
            egui::Align2::CENTER_CENTER,
            config.windows.get(&desired.window).map_or("창", |window| window.alias.as_str()),
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
    }
}
