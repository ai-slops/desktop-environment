use crate::process::{ManagedProcess, sibling_binary};
use anyhow::{Result, bail};
use desktop_presets::{PresetStore, Settings, WindowStateFile, window_state_path};
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
    devices_pending: Option<Receiver<Devices>>,
    audio_devices: Vec<AudioOutputDevice>,
    displays: Vec<DisplayInfo>,
    device_errors: Vec<String>,
    audio: ManagedProcess,
    display: ManagedProcess,
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
            devices_pending: None,
            audio_devices: Vec::new(),
            displays: Vec::new(),
            device_errors: Vec::new(),
            audio: ManagedProcess::default(),
            display: ManagedProcess::default(),
            #[cfg(feature = "ui-smoke")]
            screenshot: std::env::var_os("DESKTOP_CONTROL_SCREENSHOT")
                .map(|path| (PathBuf::from(path), std::time::Instant::now(), false)),
        };
        panel.refresh_devices();
        panel
    }

    pub fn restore_window(&mut self, window: &impl HasWindowHandle) {
        let Some(placement) = self.window_state.as_ref().and_then(|state| state.get("control"))
        else {
            return;
        };
        let result = (|| -> Result<()> {
            windows_window_placement::restore(window.window_handle()?, placement)
        })();
        if let Err(error) = result {
            self.error = Some(format!("창 배치를 복원하지 못했습니다: {error:#}"));
        }
    }

    fn remember_window(&mut self, window: &impl HasWindowHandle) -> Result<()> {
        let Some(state) = self.window_state.as_mut() else { return Ok(()) };
        if let Some(placement) = windows_window_placement::capture(window.window_handle()?)? {
            state.remember("control", placement)?;
        }
        Ok(())
    }

    const fn is_running(&self) -> bool {
        self.audio.is_running() || self.display.is_running()
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
        if let Err(error) = self.display.poll() {
            self.error = Some(format!("디스플레이: {error:#}"));
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
        let mut store = self.store.clone();
        let preset =
            store.presets.get(index).ok_or_else(|| anyhow::anyhow!("프리셋이 없습니다."))?;
        let settings = preset.settings.clone();
        let name = preset.name.clone();
        store.last_used = settings.clone();
        self.commit(store)?;
        self.draft = settings;
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
        if self.draft.audio.enabled {
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
        if self.draft.display.enabled
            && !self.displays.iter().any(|display| display.name == self.draft.display.display)
        {
            bail!("저장된 디스플레이가 연결되어 있지 않습니다. 장치를 다시 검색하세요.");
        }
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        if self.is_running() {
            bail!("설정을 바꾸기 전에 실행 중인 도구를 모두 중지하세요.");
        }
        self.validate_devices()?;
        let audio_binary =
            self.draft.audio.enabled.then(|| sibling_binary("audio-output-router")).transpose()?;
        let display_binary =
            self.draft.display.enabled.then(|| sibling_binary("display-relay")).transpose()?;
        self.save_current()?;
        if let Some(binary) = audio_binary {
            self.audio.start(&binary, &self.draft.audio_args())?;
        }
        if let Some(binary) = display_binary {
            let mut args: Vec<OsString> =
                self.draft.display_args().into_iter().map(OsString::from).collect();
            args.extend([
                OsString::from("--window-state-file"),
                window_state_path(&self.path, "relay").into_os_string(),
            ]);
            if let Err(error) = self.display.start(&binary, &args) {
                self.audio.stop()?;
                return Err(error);
            }
        }
        self.notice = "저장한 설정으로 시작했습니다. 창을 닫으면 실행한 도구도 중지됩니다.".into();
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        // Attempt both stops even if one fails.
        let audio_result = self.audio.stop();
        let display_result = self.display.stop();
        audio_result?;
        display_result?;
        self.notice = "모든 도구를 중지했습니다.".into();
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

    fn device_settings_ui(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.checkbox(&mut self.draft.audio.enabled, RichText::new("오디오 출력 복제").strong());
            ui.weak("원본 소리를 유지하면서 다른 출력으로 복제합니다.");
            ui.add_enabled_ui(self.draft.audio.enabled, |ui| {
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
            });
        });
        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.checkbox(
                &mut self.draft.display.enabled,
                RichText::new("디스플레이 미러링").strong(),
            );
            ui.weak("선택한 화면을 별도 창에 표시합니다. 미러링 창에서 Esc로 종료할 수 있습니다.");
            ui.add_enabled_ui(self.draft.display.enabled, |ui| {
                ui.label("원본 디스플레이");
                let selected =
                    self.displays.iter().find(|display| display.name == self.draft.display.display);
                let label = selected.map_or_else(
                    || {
                        missing_label(
                            &self.draft.display.display,
                            &self.draft.display.display_label,
                        )
                    },
                    display_label,
                );
                egui::ComboBox::from_id_salt("display")
                    .width(ui.available_width() - 20.0)
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for display in &self.displays {
                            if ui
                                .selectable_value(
                                    &mut self.draft.display.display,
                                    display.name.clone(),
                                    display_label(display),
                                )
                                .clicked()
                            {
                                self.draft.display.display_label = display_label(display);
                            }
                        }
                    });
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.draft.display.fullscreen, "전체 화면");
                    ui.label("FPS");
                    ui.add(egui::DragValue::new(&mut self.draft.display.fps).range(1..=240));
                    ui.label("캡처 대기");
                    ui.add(
                        egui::DragValue::new(&mut self.draft.display.timeout_ms)
                            .range(1..=1000)
                            .suffix(" ms"),
                    );
                });
            });
        });
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("현재 설정");
            let saved = self.has_saved_settings && self.draft == self.store.last_used;
            ui.weak(if saved { "저장됨" } else { "저장하지 않은 변경" });
        });
        let editable = !self.is_running();
        ui.add_enabled_ui(editable, |ui| {
            self.device_settings_ui(ui);
        });
        if !editable {
            ui.weak("설정을 바꾸려면 실행 중인 도구를 모두 중지하세요.");
        }
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(editable, egui::Button::new("현재 설정 저장")).clicked() {
                let result = self.save_current();
                self.report(result);
            }
            if ui
                .add_enabled(
                    editable && self.devices_pending.is_none(),
                    egui::Button::new(RichText::new("저장하고 시작").color(Color32::WHITE))
                        .fill(Color32::from_rgb(36, 96, 180)),
                )
                .clicked()
            {
                let result = self.start();
                self.report(result);
            }
            if ui.add_enabled(!editable, egui::Button::new("모두 중지")).clicked() {
                let result = self.stop();
                self.report(result);
            }
        });
        ui.separator();
        ui.heading("실행 상태");
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("오디오: {}", self.audio.status));
            if ui.add_enabled(self.audio.is_running(), egui::Button::new("오디오 중지")).clicked()
            {
                let result = self.audio.stop();
                self.report(result);
            }
            ui.label(format!("미러링: {}", self.display.status));
            if ui.add_enabled(self.display.is_running(), egui::Button::new("미러링 중지")).clicked()
            {
                let result = self.display.stop();
                self.report(result);
            }
        });
        egui::CollapsingHeader::new("실행 로그").show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("logs")
                .max_height(150.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for (name, process) in [("오디오", &self.audio), ("미러링", &self.display)]
                    {
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
                    ui.weak("오디오와 디스플레이, 한 번 저장하고 다시 사용하세요.");
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
        egui::SidePanel::left("sidebar").exact_width(245.0).resizable(false).show(ctx, |ui| {
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
}
