use crate::process::{ManagedProcess, sibling_binary};
use anyhow::{Context, Result, bail};
use desktop_presets::{DisplaySettings, PresetStore, Settings, WindowStateFile, window_state_path};
use eframe::egui::{self, Color32, RichText};
use raw_window_handle::HasWindowHandle;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use windows_audio_router::{AudioOutputDevice, list_output_devices};
use windows_desktop_duplication::{DisplayInfo, enumerate_displays};

struct Devices {
    audio: Result<Vec<AudioOutputDevice>, String>,
    displays: Result<Vec<DisplayInfo>, String>,
}

// A new layout key applies the smaller default once to existing installations.
// Subsequent manual sizes continue to be restored, including larger windows.
const CONTROL_WINDOW_KEY: &str = "control-compact-v1";

pub struct ControlPanel {
    path: PathBuf,
    store: PresetStore,
    draft: Settings,
    selected: Option<usize>,
    name: String,
    confirm_delete: bool,
    storage_error: Option<String>,
    error: Option<String>,
    notice: String,
    has_saved_settings: bool,
    window_state: Option<WindowStateFile>,
    skip_initial_placement_save: bool,
    devices_pending: Option<Receiver<Devices>>,
    audio_devices: Vec<AudioOutputDevice>,
    displays: Vec<DisplayInfo>,
    device_errors: Vec<String>,
    audio: ManagedProcess,
    mirrors: Vec<ManagedProcess>,
    #[cfg(feature = "ui-smoke")]
    screenshot: Option<(PathBuf, std::time::Instant, bool)>,
}

impl ControlPanel {
    pub fn new(path: PathBuf) -> Self {
        let (store, storage_error) = match PresetStore::load(&path) {
            Ok(store) => (store, None),
            Err(error) => (PresetStore::default(), Some(format!("{error:#}"))),
        };
        let selected = store.presets.iter().position(|preset| preset.settings == store.last_used);
        let has_saved_settings = path.is_file() && storage_error.is_none();
        let notice = if has_saved_settings {
            "마지막으로 저장한 설정을 불러왔습니다. 시작 버튼으로 실행하세요."
        } else {
            "설정을 선택하고 저장하세요. 저장한 설정은 다음 실행 때 복원됩니다."
        };
        let (window_state, window_error) =
            match WindowStateFile::load(window_state_path(&path, "control")) {
                Ok(state) => (Some(state), None),
                Err(error) => (None, Some(format!("창 배치를 읽지 못했습니다: {error:#}"))),
            };
        let mirrors = store.last_used.displays().map(|_| ManagedProcess::default()).collect();
        let mut panel = Self {
            path,
            draft: store.last_used.clone(),
            store,
            selected,
            name: String::new(),
            confirm_delete: false,
            storage_error,
            error: window_error,
            notice: notice.into(),
            has_saved_settings,
            window_state,
            skip_initial_placement_save: false,
            devices_pending: None,
            audio_devices: Vec::new(),
            displays: Vec::new(),
            device_errors: Vec::new(),
            audio: ManagedProcess::default(),
            mirrors,
            #[cfg(feature = "ui-smoke")]
            screenshot: std::env::var_os("DESKTOP_CONTROL_SCREENSHOT")
                .map(|path| (PathBuf::from(path), std::time::Instant::now(), false)),
        };
        panel.refresh_devices();
        panel
    }

    pub fn restore_window(&mut self, window: &eframe::CreationContext<'_>) {
        let Some(state) = self.window_state.as_ref() else {
            return;
        };
        let legacy = state.get(CONTROL_WINDOW_KEY).is_none();
        let Some(mut placement) = state.get(CONTROL_WINDOW_KEY).or_else(|| state.get("control"))
        else {
            return;
        };
        if legacy {
            placement.maximized = false;
        }
        let result = (|| -> Result<()> {
            windows_window_placement::restore(window.window_handle()?, placement)
        })();
        if let Err(error) = result {
            self.error = Some(format!("창 배치를 복원하지 못했습니다: {error:#}"));
        }
        if legacy {
            window.egui_ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                crate::COMPACT_WINDOW_SIZE.into(),
            ));
            self.skip_initial_placement_save = true;
        }
    }

    fn remember_window(&mut self, window: &impl HasWindowHandle) -> Result<()> {
        if self.skip_initial_placement_save {
            self.skip_initial_placement_save = false;
            return Ok(());
        }
        let Some(state) = self.window_state.as_mut() else { return Ok(()) };
        if let Some(placement) = windows_window_placement::capture(window.window_handle()?)? {
            state.remember(CONTROL_WINDOW_KEY, placement)?;
        }
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.audio.is_running() || self.mirrors.iter().any(ManagedProcess::is_running)
    }

    fn refresh_devices(&mut self) {
        if self.devices_pending.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.devices_pending = Some(receiver);
        std::thread::spawn(move || {
            let devices = Devices {
                audio: list_output_devices().map_err(|error| format!("오디오 장치: {error:#}")),
                displays: enumerate_displays().map_err(|error| format!("디스플레이: {error:#}")),
            };
            let _ = sender.send(devices);
        });
    }

    fn poll(&mut self) {
        if let Some(receiver) = self.devices_pending.as_ref() {
            match receiver.try_recv() {
                Ok(devices) => {
                    self.device_errors.clear();
                    match devices.audio {
                        Ok(devices) => self.audio_devices = devices,
                        Err(error) => {
                            self.audio_devices.clear();
                            self.device_errors.push(error);
                        }
                    }
                    match devices.displays {
                        Ok(displays) => self.displays = displays,
                        Err(error) => {
                            self.displays.clear();
                            self.device_errors.push(error);
                        }
                    }
                    self.devices_pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.devices_pending = None;
                    self.device_errors
                        .push("장치 목록을 가져오지 못했습니다. 다시 검색하세요.".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Err(error) = self.audio.poll() {
            self.error = Some(format!("오디오: {error:#}"));
        }
        for (index, mirror) in self.mirrors.iter_mut().enumerate() {
            if let Err(error) = mirror.poll() {
                self.error = Some(format!("미러링 {}: {error:#}", index + 1));
            }
        }
    }

    fn commit(&mut self, store: PresetStore) -> Result<()> {
        if let Some(error) = self.storage_error.as_ref() {
            bail!(
                "기존 설정 파일을 보호하기 위해 저장을 막았습니다: {error}. 파일을 복구한 후 다시 실행하세요."
            );
        }
        store.save(&self.path)?;
        self.store = store;
        self.has_saved_settings = true;
        Ok(())
    }

    fn save_current(&mut self) -> Result<()> {
        self.draft.validate()?;
        let mut store = self.store.clone();
        store.last_used = self.draft.clone();
        self.commit(store)?;
        self.notice = "현재 설정을 저장했습니다. 다음 실행 때 그대로 복원됩니다.".into();
        Ok(())
    }

    fn add_preset(&mut self) -> Result<()> {
        let mut store = self.store.clone();
        store.add(&self.name, self.draft.clone())?;
        store.last_used = self.draft.clone();
        self.commit(store)?;
        self.selected = Some(self.store.presets.len() - 1);
        self.notice = format!("‘{}’ 프리셋을 저장했습니다.", self.name.trim());
        self.name.clear();
        Ok(())
    }

    fn load_preset(&mut self, index: usize) -> Result<()> {
        if self.is_running() {
            bail!("프리셋을 불러오려면 실행 중인 도구를 모두 중지하세요.");
        }
        let mut store = self.store.clone();
        let preset =
            store.presets.get(index).ok_or_else(|| anyhow::anyhow!("프리셋이 없습니다."))?;
        let settings = preset.settings.clone();
        let name = preset.name.clone();
        store.last_used = settings.clone();
        self.commit(store)?;
        self.draft = settings;
        self.mirrors = self.draft.displays().map(|_| ManagedProcess::default()).collect();
        self.notice = format!("‘{name}’ 설정을 불러왔습니다. 시작 버튼으로 실행하세요.");
        Ok(())
    }

    fn run_preset(&mut self, index: usize) -> Result<()> {
        if self.is_running() {
            bail!("먼저 실행 중인 도구를 모두 중지하세요.");
        }
        self.load_preset(index)?;
        self.start()
    }

    fn update_preset(&mut self, index: usize) -> Result<()> {
        self.draft.validate()?;
        let mut store = self.store.clone();
        let preset =
            store.presets.get_mut(index).ok_or_else(|| anyhow::anyhow!("프리셋이 없습니다."))?;
        let name = preset.name.clone();
        preset.settings = self.draft.clone();
        store.last_used = self.draft.clone();
        self.commit(store)?;
        self.notice = format!("‘{name}’ 프리셋을 현재 설정으로 덮어썼습니다.");
        Ok(())
    }

    fn delete_preset(&mut self, index: usize) -> Result<()> {
        let mut store = self.store.clone();
        if index >= store.presets.len() {
            bail!("프리셋이 없습니다.");
        }
        let preset = store.presets.remove(index);
        self.commit(store)?;
        self.selected = None;
        self.confirm_delete = false;
        self.notice = format!(
            "‘{}’ 프리셋을 삭제했습니다. 현재 설정은 계속 사용할 수 있습니다.",
            preset.name
        );
        Ok(())
    }

    fn validate_devices(&self) -> Result<()> {
        self.draft.validate()?;
        if self.devices_pending.is_some() {
            bail!("장치 검색이 끝날 때까지 기다려 주세요.");
        }
        if self.draft.audio.enabled && !self.audio.is_running() {
            let resolve = |selector: &str| {
                self.audio_devices.iter().find(|device| {
                    if selector == "default" { device.is_default } else { device.id == selector }
                })
            };
            let source = resolve(&self.draft.audio.source).ok_or_else(|| {
                anyhow::anyhow!(
                    "저장된 오디오 원본이 연결되어 있지 않습니다. 장치를 다시 검색하세요."
                )
            })?;
            let target = resolve(&self.draft.audio.target).ok_or_else(|| {
                anyhow::anyhow!(
                    "저장된 오디오 대상이 연결되어 있지 않습니다. 장치를 다시 검색하세요."
                )
            })?;
            if source.id == target.id {
                bail!("Windows 기본 출력과 대상이 같은 장치입니다. 다른 출력을 선택하세요.");
            }
        }
        for (index, settings) in self.draft.displays().enumerate() {
            if settings.enabled && !self.mirrors[index].is_running() {
                self.validate_mirror(index)?;
            }
        }
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        self.start_with_resolver(sibling_binary)
    }

    fn start_with_resolver(&mut self, resolve: impl Fn(&str) -> Result<PathBuf>) -> Result<()> {
        self.poll();
        self.validate_devices()?;
        let audio_binary = (self.draft.audio.enabled && !self.audio.is_running())
            .then(|| resolve("audio-output-router"))
            .transpose()?;
        let pending: Vec<usize> = self
            .draft
            .displays()
            .enumerate()
            .filter(|(index, settings)| settings.enabled && !self.mirrors[*index].is_running())
            .map(|(index, _)| index)
            .collect();
        let display_binary = (!pending.is_empty()).then(|| resolve("display-relay")).transpose()?;
        self.save_current()?;
        let started_audio = audio_binary.is_some();
        if let Some(binary) = audio_binary {
            self.audio.start(&binary, &self.draft.audio_args())?;
        }
        if let Some(binary) = display_binary {
            let mut started: Vec<usize> = Vec::new();
            for index in pending {
                if let Err(error) = self.launch_mirror(index, &binary) {
                    // Roll back only the processes started by this click.
                    for index in started {
                        let _ = self.mirrors[index].stop();
                    }
                    if started_audio {
                        let _ = self.audio.stop();
                    }
                    return Err(error);
                }
                started.push(index);
            }
        }
        self.notice =
            "저장한 설정으로 시작했습니다. 설정 앱을 닫으면 실행한 도구도 중지됩니다.".into();
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        // Attempt every owned process even if one stop fails.
        let mut result = self.audio.stop();
        for mirror in &mut self.mirrors {
            let stopped = mirror.stop();
            if result.is_ok() {
                result = stopped;
            }
        }
        result?;
        self.notice = "모든 도구를 중지했습니다.".into();
        Ok(())
    }

    fn validate_mirror(&self, index: usize) -> Result<()> {
        let settings = self.draft.display_at(index).context("미러링 설정이 없습니다.")?;
        if !settings.enabled {
            bail!("미러링을 선택하세요.");
        }
        settings.validate()?;
        if self.devices_pending.is_some() {
            bail!("장치 검색이 끝날 때까지 기다려 주세요.");
        }
        if !self.displays.iter().any(|display| display.name == settings.display) {
            bail!("저장된 디스플레이가 연결되어 있지 않습니다. 장치를 다시 검색하세요.");
        }
        Ok(())
    }

    fn mirror_state_path(&self, index: usize) -> PathBuf {
        // Separate files avoid concurrent children overwriting each other's placements.
        let kind = if index == 0 { "relay".into() } else { format!("relay-{}", index + 1) };
        window_state_path(&self.path, &kind)
    }

    fn launch_mirror(&mut self, index: usize, binary: &std::path::Path) -> Result<()> {
        let settings = self.draft.display_at(index).context("미러링 설정이 없습니다.")?;
        let mut args: Vec<OsString> = settings.args().into_iter().map(OsString::from).collect();
        args.extend([
            OsString::from("--window-state-file"),
            self.mirror_state_path(index).into_os_string(),
        ]);
        self.mirrors.get_mut(index).context("미러링 실행 상태가 없습니다.")?.start(binary, &args)
    }

    fn open_mirror(&mut self, index: usize) -> Result<()> {
        self.open_mirror_with_binary(index, &sibling_binary("display-relay")?)
    }

    fn open_mirror_with_binary(&mut self, index: usize, binary: &std::path::Path) -> Result<()> {
        self.poll();
        self.validate_mirror(index)?;
        self.launch_mirror(index, binary)?;
        self.notice = format!(
            "미러링 {} 창을 열었습니다. 현재 설정 저장으로 이 구성을 보관하세요.",
            index + 1
        );
        Ok(())
    }

    fn add_mirror(&mut self) {
        self.draft
            .additional_displays
            .push(DisplaySettings { enabled: true, ..DisplaySettings::default() });
        self.mirrors.push(ManagedProcess::default());
    }

    fn remove_mirror(&mut self, index: usize) -> Result<()> {
        if index == 0 || index + 1 != self.mirrors.len() {
            bail!("추가한 미러링만 삭제할 수 있습니다.");
        }
        if self.is_running() {
            bail!("미러링 설정을 삭제하려면 실행 중인 도구를 모두 중지하세요.");
        }
        self.draft.additional_displays.remove(index - 1);
        self.mirrors.remove(index);
        Ok(())
    }

    fn report(&mut self, result: Result<()>) {
        self.error = result.err().map(|error| format!("{error:#}"));
    }

    fn create_desktop_shortcut(&mut self) -> Result<()> {
        let executable = std::env::current_exe()?;
        let shortcut = windows_shortcuts::create_desktop_shortcut(&executable, &self.path)?;
        self.notice = format!("바탕화면 바로가기를 만들었습니다: {}", shortcut.display());
        Ok(())
    }

    #[cfg(feature = "ui-smoke")]
    fn screenshot_ui(&mut self, ctx: &egui::Context) {
        let Some((path, started, requested)) = self.screenshot.as_mut() else { return };
        if !*requested
            && started.elapsed() > Duration::from_secs(2)
            && self.devices_pending.is_none()
        {
            *requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let screenshot = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(screenshot) = screenshot {
            let result = (|| -> Result<()> {
                let pixels: Vec<u8> =
                    screenshot.pixels.iter().flat_map(Color32::to_array).collect();
                image::save_buffer(
                    path,
                    &pixels,
                    screenshot.width().try_into()?,
                    screenshot.height().try_into()?,
                    image::ColorType::Rgba8,
                )?;
                Ok(())
            })();
            self.report(result);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn presets_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("저장한 프리셋");
        ui.label("자주 쓰는 조합을 이름으로 저장하세요.");
        ui.add_space(6.0);
        let editable = !self.is_running();
        ui.add_enabled_ui(editable, |ui| {
            egui::ScrollArea::vertical().id_salt("presets").max_height(220.0).show(ui, |ui| {
                if self.store.presets.is_empty() {
                    ui.weak("아직 저장한 프리셋이 없습니다.");
                }
                for (index, preset) in self.store.presets.iter().enumerate() {
                    if ui.selectable_label(self.selected == Some(index), &preset.name).clicked() {
                        self.selected = Some(index);
                        self.confirm_delete = false;
                    }
                }
            });
            if let Some(index) = self.selected {
                if ui.button("선택한 설정 불러오기").clicked() {
                    let result = self.load_preset(index);
                    self.report(result);
                }
                if ui
                    .add_enabled(
                        self.devices_pending.is_none(),
                        egui::Button::new("이 프리셋으로 시작"),
                    )
                    .clicked()
                {
                    let result = self.run_preset(index);
                    self.report(result);
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button("덮어쓰기")
                        .on_hover_text("선택한 프리셋을 현재 설정으로 바꿉니다.")
                        .clicked()
                    {
                        let result = self.update_preset(index);
                        self.report(result);
                    }
                    if ui.button("삭제").clicked() {
                        self.confirm_delete = true;
                    }
                });
                if self.confirm_delete {
                    ui.label("선택한 프리셋을 삭제할까요?");
                    ui.horizontal(|ui| {
                        if ui.button("삭제 확인").clicked() {
                            let result = self.delete_preset(index);
                            self.report(result);
                        }
                        if ui.button("취소").clicked() {
                            self.confirm_delete = false;
                        }
                    });
                }
            }
            ui.separator();
            ui.label("새 프리셋 이름");
            ui.add(egui::TextEdit::singleline(&mut self.name).hint_text("예: 캡처 카드 / 헤드폰"));
            if ui.button("새 프리셋으로 저장").clicked() {
                let result = self.add_preset();
                self.report(result);
            }
        });
        ui.add_space(12.0);
        ui.weak("다음 실행 때 마지막으로 저장한 설정을 복원합니다. 자동으로 실행하지는 않습니다.");
        ui.weak("창 크기와 위치는 자동으로 기억합니다.");
        ui.separator();
        if ui
            .button("바탕화면 바로가기 만들기")
            .on_hover_text("현재 설정 파일을 사용하는 Desktop Control 바로가기를 만듭니다.")
            .clicked()
        {
            let result = self.create_desktop_shortcut();
            self.report(result);
        }
    }

    fn audio_settings_ui(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_enabled_ui(!self.audio.is_running(), |ui| {
                ui.checkbox(
                    &mut self.draft.audio.enabled,
                    RichText::new("오디오 출력 복제").strong(),
                );
                ui.weak("원본 소리를 유지하면서 다른 출력으로 복제합니다.");
                if self.draft.audio.enabled {
                    audio_selector(
                        ui,
                        "audio_source",
                        "원본 출력",
                        &mut self.draft.audio.source,
                        &mut self.draft.audio.source_label,
                        &self.audio_devices,
                    );
                    audio_selector(
                        ui,
                        "audio_target",
                        "대상 출력",
                        &mut self.draft.audio.target,
                        &mut self.draft.audio.target_label,
                        &self.audio_devices,
                    );
                }
            });
        });
        ui.add_space(8.0);
    }

    fn mirror_settings_ui(&mut self, ui: &mut egui::Ui, index: usize) {
        let running = self.mirrors[index].is_running();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                if let Some(settings) = self.draft.display_at_mut(index) {
                    ui.add_enabled(
                        !running,
                        egui::Checkbox::new(
                            &mut settings.enabled,
                            RichText::new(format!("디스플레이 미러링 {}", index + 1)).strong(),
                        ),
                    );
                }
                ui.label(&self.mirrors[index].status);
                let enabled = self.draft.display_at(index).is_some_and(|settings| settings.enabled);
                let label = if self.mirrors[index].status == "중지됨"
                    && self.mirrors[index].logs.is_empty()
                {
                    "창 열기"
                } else {
                    "다시 열기"
                };
                if ui
                    .add_enabled(
                        !running && enabled && self.devices_pending.is_none(),
                        egui::Button::new(label),
                    )
                    .clicked()
                {
                    let result = self.open_mirror(index);
                    self.report(result);
                }
                if ui.add_enabled(running, egui::Button::new("중지")).clicked() {
                    let result = self.mirrors[index].stop();
                    self.report(result);
                }
            });
            ui.add_enabled_ui(!running, |ui| {
                let Some(settings) = self.draft.display_at_mut(index) else { return };
                ui.add_enabled_ui(settings.enabled, |ui| {
                    ui.label("원본 디스플레이");
                    let selected =
                        self.displays.iter().find(|display| display.name == settings.display);
                    let label = selected.map_or_else(
                        || missing_label(&settings.display, &settings.display_label),
                        display_label,
                    );
                    egui::ComboBox::from_id_salt(("display", index))
                        .width(ui.available_width() - 20.0)
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            for display in &self.displays {
                                if ui
                                    .selectable_value(
                                        &mut settings.display,
                                        display.name.clone(),
                                        display_label(display),
                                    )
                                    .clicked()
                                {
                                    settings.display_label = display_label(display);
                                }
                            }
                        });
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut settings.fullscreen, "전체 화면");
                        ui.label("FPS");
                        ui.add(egui::DragValue::new(&mut settings.fps).range(1..=240));
                        ui.label("캡처 대기");
                        ui.add(
                            egui::DragValue::new(&mut settings.timeout_ms)
                                .range(1..=1000)
                                .suffix(" ms"),
                        );
                    });
                });
            });
        });
        ui.add_space(8.0);
    }

    fn device_settings_ui(&mut self, ui: &mut egui::Ui) {
        self.audio_settings_ui(ui);
        ui.horizontal_wrapped(|ui| {
            if ui.button("미러링 추가").clicked() {
                self.add_mirror();
            }
            if self.mirrors.len() > 1
                && ui
                    .add_enabled(!self.is_running(), egui::Button::new("마지막 미러링 설정 삭제"))
                    .clicked()
            {
                let result = self.remove_mirror(self.mirrors.len() - 1);
                self.report(result);
            }
        });
        ui.weak("각 창은 따로 열고 닫을 수 있습니다. 창을 닫으면 여기서 다시 열어 주세요.");
        for index in 0..self.mirrors.len() {
            self.mirror_settings_ui(ui, index);
        }
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("현재 설정");
            let saved = self.has_saved_settings && self.draft == self.store.last_used;
            ui.weak(if saved { "저장됨" } else { "저장하지 않은 변경" });
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("현재 설정 저장").clicked() {
                let result = self.save_current();
                self.report(result);
            }
            if ui
                .add_enabled(
                    self.devices_pending.is_none(),
                    egui::Button::new(RichText::new("저장하고 시작").color(Color32::WHITE))
                        .fill(Color32::from_rgb(36, 96, 180)),
                )
                .clicked()
            {
                let result = self.start();
                self.report(result);
            }
            if ui.add_enabled(self.is_running(), egui::Button::new("모두 중지")).clicked() {
                let result = self.stop();
                self.report(result);
            }
        });
        ui.weak("실행 중인 도구의 설정은 중지한 후 바꿀 수 있습니다.");
        ui.add_space(8.0);
        self.device_settings_ui(ui);
        ui.separator();
        ui.heading("실행 상태");
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("오디오: {}", self.audio.status));
            if ui.add_enabled(self.audio.is_running(), egui::Button::new("오디오 중지")).clicked()
            {
                let result = self.audio.stop();
                self.report(result);
            }
            let running = self.mirrors.iter().filter(|mirror| mirror.is_running()).count();
            ui.label(format!("미러링: {running}/{}개 실행 중", self.mirrors.len()));
        });
        egui::CollapsingHeader::new("실행 로그").show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("logs")
                .max_height(150.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    let processes = std::iter::once(("오디오".into(), &self.audio)).chain(
                        self.mirrors
                            .iter()
                            .enumerate()
                            .map(|(index, mirror)| (format!("미러링 {}", index + 1), mirror)),
                    );
                    for (name, process) in processes {
                        ui.strong(name);
                        if process.logs.is_empty() {
                            ui.weak("아직 로그가 없습니다.");
                        }
                        for line in &process.logs {
                            ui.add(egui::Label::new(line).wrap());
                        }
                    }
                });
        });
    }
}

impl eframe::App for ControlPanel {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll();
        if let Err(error) = self.remember_window(frame) {
            self.error = Some(format!("창 배치를 저장하지 못했습니다: {error:#}"));
        }
        #[cfg(feature = "ui-smoke")]
        self.screenshot_ui(ctx);
        ctx.request_repaint_after(Duration::from_millis(200));
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.heading("Desktop Control");
                    ui.weak("저장한 설정으로 간편하게 실행하세요.");
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("창 작게")
                        .on_hover_text("설정 창을 기본 크기로 줄입니다.")
                        .clicked()
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                            crate::COMPACT_WINDOW_SIZE.into(),
                        ));
                    }
                    if ui
                        .add_enabled(
                            self.devices_pending.is_none(),
                            egui::Button::new("장치 다시 검색"),
                        )
                        .clicked()
                    {
                        self.refresh_devices();
                    }
                    if self.devices_pending.is_some() {
                        ui.spinner();
                    }
                });
            });
            ui.add_space(6.0);
        });
        egui::TopBottomPanel::bottom("messages").show(ctx, |ui| {
            if let Some(error) = &self.storage_error {
                ui.colored_label(Color32::DARK_RED, format!("설정을 읽지 못했습니다: {error}"));
                ui.label("기존 파일을 보호하기 위해 저장과 실행을 막았습니다. 파일을 복구한 후 다시 실행하세요.");
            }
            if let Some(error) = &self.error {
                let dismissed = ui.horizontal_wrapped(|ui| {
                    ui.colored_label(Color32::DARK_RED, error);
                    ui.small_button("닫기").clicked()
                }).inner;
                if dismissed {
                    self.error = None;
                }
            } else {
                ui.label(&self.notice);
            }
            for error in &self.device_errors {
                ui.colored_label(Color32::DARK_RED, error);
            }
            ui.add(egui::Label::new(RichText::new(format!("설정 파일: {}", self.path.display())).small()).wrap());
        });
        egui::SidePanel::left("sidebar").exact_width(220.0).resizable(false).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.presets_ui(ui));
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.settings_ui(ui));
        });
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        let _ = self.stop();
    }
}

fn missing_label(id: &str, label: &str) -> String {
    if id.is_empty() {
        "장치를 선택하세요".into()
    } else {
        format!("{} (연결 안 됨)", if label.is_empty() { id } else { label })
    }
}

fn display_label(display: &DisplayInfo) -> String {
    format!(
        "{} · {} · {}×{}",
        display.friendly_name, display.name, display.area.width, display.area.height
    )
}

fn audio_selector(
    ui: &mut egui::Ui,
    id: &str,
    title: &str,
    value: &mut String,
    saved_label: &mut String,
    devices: &[AudioOutputDevice],
) {
    ui.label(title);
    let label = if value == "default" {
        devices.iter().find(|device| device.is_default).map_or_else(
            || "Windows 기본 출력 (연결 안 됨)".into(),
            |device| format!("Windows 기본 출력 · {}", device.friendly_name),
        )
    } else {
        devices.iter().find(|device| device.id == *value).map_or_else(
            || missing_label(value, saved_label),
            |device| device.friendly_name.clone(),
        )
    };
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width() - 20.0)
        .selected_text(label)
        .show_ui(ui, |ui| {
            if ui.selectable_value(value, "default".into(), "Windows 기본 출력").clicked() {
                *saved_label = "Windows 기본 출력".into();
            }
            for device in devices {
                if ui
                    .selectable_value(value, device.id.clone(), &device.friendly_name)
                    .on_hover_text(&device.id)
                    .clicked()
                {
                    saved_label.clone_from(&device.friendly_name);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(directory: &std::path::Path) -> ControlPanel {
        let mut panel = ControlPanel::new(directory.join("presets.json"));
        // Use deterministic devices instead of depending on the host's hardware.
        panel.devices_pending = None;
        panel.audio_devices = vec![
            AudioOutputDevice {
                id: "speakers".into(),
                friendly_name: "Speakers".into(),
                is_default: true,
            },
            AudioOutputDevice {
                id: "headphones".into(),
                friendly_name: "Headphones".into(),
                is_default: false,
            },
        ];
        panel.draft.audio.target = "headphones".into();
        panel.draft.audio.target_label = "Headphones".into();
        panel
    }

    #[test]
    fn save_reload_update_and_delete_presets() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut panel = panel(directory.path());
        panel.name = "방송".into();
        panel.add_preset()?;
        let saved = panel.draft.clone();
        panel.draft.audio.source = "speakers".into();
        panel.load_preset(0)?;
        assert_eq!(panel.draft, saved);
        panel.draft.audio.source = "speakers".into();
        panel.update_preset(0)?;
        let restarted = ControlPanel::new(panel.path.clone());
        assert_eq!(restarted.draft, panel.draft);
        assert!(!restarted.is_running());
        panel.delete_preset(0)?;
        assert!(PresetStore::load(&panel.path)?.presets.is_empty());
        assert_eq!(PresetStore::load(&panel.path)?.last_used, panel.draft);
        Ok(())
    }

    #[test]
    fn unavailable_devices_and_resolved_default_feedback_are_rejected() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut panel = panel(directory.path());
        panel.validate_devices()?;
        panel.draft.audio.target = "speakers".into();
        assert!(panel.validate_devices().is_err());
        panel.draft.audio.target = "unplugged".into();
        assert!(panel.validate_devices().is_err());
        assert_eq!(panel.draft.audio.target, "unplugged");
        panel.draft.audio.enabled = false;
        panel.draft.display.enabled = true;
        panel.draft.display.display = r"\\.\DISPLAY99".into();
        assert!(panel.validate_devices().is_err());
        Ok(())
    }

    #[test]
    fn corrupted_storage_cannot_be_overwritten_by_gui() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("presets.json");
        std::fs::write(&path, "broken JSON")?;
        let mut panel = panel(directory.path());
        assert!(panel.save_current().is_err());
        assert_eq!(std::fs::read_to_string(path)?, "broken JSON");
        Ok(())
    }

    #[test]
    fn persistence_failure_does_not_change_memory() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut panel = panel(directory.path());
        std::fs::create_dir(&panel.path)?;
        panel.name = "save failure".into();
        let old = panel.store.clone();
        assert!(panel.add_preset().is_err());
        assert_eq!(panel.store, old);
        assert_eq!(panel.selected, None);
        Ok(())
    }

    #[test]
    fn broken_window_state_does_not_block_presets_or_get_overwritten() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = window_state_path(&directory.path().join("presets.json"), "control");
        std::fs::write(&path, "broken placement")?;
        let mut panel = panel(directory.path());
        assert!(panel.window_state.is_none());
        assert!(panel.error.is_some());
        panel.save_current()?;
        assert_eq!(std::fs::read_to_string(path)?, "broken placement");
        assert_eq!(PresetStore::load(&panel.path)?.last_used, panel.draft);
        Ok(())
    }

    #[test]
    fn additional_mirrors_save_reload_and_use_independent_geometry_files() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut panel = panel(directory.path());
        panel.draft.display = DisplaySettings {
            enabled: true,
            display: "DISPLAY1".into(),
            ..DisplaySettings::default()
        };
        panel.add_mirror();
        panel.draft.additional_displays[0] = panel.draft.display.clone();
        assert_ne!(panel.mirror_state_path(0), panel.mirror_state_path(1));
        panel.name = "여러 화면".into();
        panel.add_preset()?;
        panel.remove_mirror(1)?;
        panel.load_preset(0)?;
        assert_eq!(panel.mirrors.len(), 2);
        assert_eq!(panel.draft.additional_displays[0], panel.draft.display);
        let restarted = ControlPanel::new(panel.path.clone());
        assert_eq!(restarted.mirrors.len(), 2);
        assert_eq!(restarted.draft, panel.draft);
        assert!(!restarted.is_running());
        Ok(())
    }

    #[test]
    fn independent_mirror_validation_ignores_incomplete_audio_and_other_rows() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut panel = panel(directory.path());
        panel.draft.audio.target.clear();
        let area = display_relay_core::DisplayArea { left: 0, top: 0, width: 1920, height: 1080 };
        panel.displays = vec![DisplayInfo {
            name: "DISPLAY1".into(),
            friendly_name: "Test display".into(),
            area,
            virtual_desktop: display_relay_core::VirtualDesktop { bounds: area },
        }];
        let display = panel.displays.first().context("No connected display")?;
        panel.draft.display = DisplaySettings {
            enabled: true,
            display: display.name.clone(),
            ..DisplaySettings::default()
        };
        panel.add_mirror();
        assert!(panel.validate_devices().is_err());
        panel.validate_mirror(0)?;
        assert!(panel.validate_mirror(1).is_err());
        Ok(())
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires a built display-relay and connected Windows display; set DESKTOP_CONTROL_RELAY_SMOKE_BINARY"]
    fn real_mirror_close_reopen_keeps_other_windows_running() -> Result<()> {
        use std::time::{Duration, Instant};
        let binary = std::env::var_os("DESKTOP_CONTROL_RELAY_SMOKE_BINARY")
            .map(PathBuf::from)
            .context("Set DESKTOP_CONTROL_RELAY_SMOKE_BINARY")?;
        let directory = tempfile::tempdir()?;
        let mut panel = panel(directory.path());
        panel.displays = enumerate_displays()?;
        let display = panel.displays.last().context("No connected display")?;
        panel.draft.display = DisplaySettings {
            enabled: true,
            display: display.name.clone(),
            fps: 15,
            ..DisplaySettings::default()
        };
        panel.add_mirror();
        panel.draft.additional_displays[0] = panel.draft.display.clone();
        // No audio output is selected; per-window open must remain independent.
        panel.draft.audio.target.clear();
        panel.open_mirror_with_binary(0, &binary)?;
        let first_pid = panel.mirrors[0].child_id().context("First mirror missing")?;
        panel.draft.audio.enabled = false;
        panel.start_with_resolver(|name| {
            assert_eq!(name, "display-relay");
            Ok(binary.clone())
        })?;
        assert_eq!(panel.mirrors[0].child_id(), Some(first_pid));
        let other_pid = panel.mirrors[1].child_id().context("Second mirror missing")?;
        close_owned_mirror_window(first_pid)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while panel.mirrors[0].is_running() {
            panel.poll();
            if Instant::now() >= deadline {
                bail!("Closed mirror did not exit");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(panel.mirrors[0].status, "종료됨");
        assert_eq!(panel.mirrors[1].child_id(), Some(other_pid));
        assert!(panel.mirrors[1].is_running());
        panel.open_mirror_with_binary(0, &binary)?;
        assert_ne!(panel.mirrors[0].child_id(), Some(first_pid));
        assert_eq!(panel.mirrors[1].child_id(), Some(other_pid));
        panel
            .start_with_resolver(|_| bail!("Already running mirrors must not resolve binaries"))?;
        assert!(panel.open_mirror_with_binary(1, &binary).is_err());
        assert!(panel.remove_mirror(1).is_err());
        let deadline = Instant::now() + Duration::from_secs(10);
        while !panel.mirror_state_path(0).exists() || !panel.mirror_state_path(1).exists() {
            panel.poll();
            assert!(panel.mirrors.iter().all(ManagedProcess::is_running));
            if Instant::now() >= deadline {
                bail!("Mirrors did not save their separate placements");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panel.stop()?;
        assert!(!panel.is_running());
        println!(
            "Two real mirror windows opened; closing/reopening one preserved the other; separate placements saved; all stopped."
        );
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn close_owned_mirror_window(pid: u32) -> Result<()> {
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetWindowTextW, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
        };
        use windows::core::BOOL;
        struct Search {
            pid: u32,
            window: HWND,
        }
        unsafe extern "system" fn find(window: HWND, data: LPARAM) -> BOOL {
            // EnumWindows invokes this synchronously with the live stack search below.
            let search = unsafe { &mut *(data.0 as *mut Search) };
            let mut pid = 0;
            unsafe {
                GetWindowThreadProcessId(window, Some(&raw mut pid));
            }
            let mut title = [0u16; 200];
            let length = unsafe { GetWindowTextW(window, &mut title) };
            if pid == search.pid
                && length > 0
                && String::from_utf16_lossy(&title[..usize::try_from(length).unwrap_or(0)])
                    .starts_with("Relay ")
            {
                search.window = window;
            }
            BOOL(1)
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let mut search = Search { pid, window: HWND::default() };
            unsafe {
                EnumWindows(
                    Some(find),
                    LPARAM((&raw mut search).cast::<std::ffi::c_void>() as isize),
                )?;
            }
            if search.window != HWND::default() {
                // Only a title-bearing window of the owned test child PID is closed.
                unsafe {
                    PostMessageW(Some(search.window), WM_CLOSE, WPARAM(0), LPARAM(0))?;
                }
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                bail!("Owned mirror did not create a window");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
