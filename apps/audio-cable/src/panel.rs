use audio_cable_core::{
    AudioBackend, Cable, Endpoint, EndpointFlow, Preset, RouteControl, RouteState, Settings,
};
use eframe::egui::{self, Color32};
use std::path::PathBuf;
use std::sync::{
    Arc,
    mpsc::{self, Receiver},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

struct Worker {
    control: Arc<RouteControl>,
    thread: Option<JoinHandle<Result<(), String>>>,
    cable: Cable,
    output_id: String,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.control.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub struct CablePanel {
    path: PathBuf,
    settings: Settings,
    backend: Option<Arc<dyn AudioBackend>>,
    endpoints: Vec<Endpoint>,
    cables: Vec<Cable>,
    pending: Option<Receiver<Result<Vec<Endpoint>, String>>>,
    next_refresh: Instant,
    worker: Option<Worker>,
    error: Option<String>,
    notice: String,
    manual_playback: String,
    manual_capture: String,
    preset_name: String,
    #[cfg(feature = "ui-smoke")]
    screenshot: Option<(PathBuf, Instant, bool)>,
}
impl CablePanel {
    pub fn new(path: PathBuf, backend: Option<Arc<dyn AudioBackend>>) -> Self {
        let (settings, error) = match Settings::load(&path) {
            Ok(settings) => (settings, None),
            Err(error) => (Settings::default(), Some(format!("{error:#}"))),
        };
        let mut panel = Self {
            path,
            settings,
            backend,
            endpoints: Vec::new(),
            cables: Vec::new(),
            pending: None,
            next_refresh: Instant::now(),
            worker: None,
            error,
            notice: "시작 버튼을 누르기 전에는 소리를 재생하지 않습니다.".into(),
            manual_playback: String::new(),
            manual_capture: String::new(),
            preset_name: String::new(),
            #[cfg(feature = "ui-smoke")]
            screenshot: std::env::var_os("AUDIO_CABLE_SCREENSHOT")
                .map(|path| (PathBuf::from(path), Instant::now(), false)),
        };
        panel.refresh();
        panel
    }
    fn refresh(&mut self) {
        if self.pending.is_some() {
            return;
        }
        if let Some(backend) = self.backend.clone() {
            let (send, receive) = mpsc::channel();
            self.pending = Some(receive);
            std::thread::spawn(move || {
                let _ = send.send(backend.endpoints().map_err(|e| format!("{e:#}")));
            });
        }
        self.next_refresh = Instant::now() + Duration::from_secs(2);
    }
    fn poll(&mut self) {
        if let Some(receive) = &self.pending {
            match receive.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    match result {
                        Ok(endpoints) => {
                            self.cables = self
                                .backend
                                .as_ref()
                                .map_or_else(Vec::new, |b| b.installed_cables(&endpoints));
                            if self.settings.cable.is_none() && self.cables.len() == 1 {
                                self.settings.cable = self.cables.first().cloned();
                            }
                            if self.settings.output_id.is_empty() {
                                let source =
                                    self.settings.cable.as_ref().map(|c| c.playback_id.as_str());
                                if let Some(output) = endpoints.iter().find(|e| {
                                    e.flow == EndpointFlow::Playback
                                        && e.is_default
                                        && Some(e.id.as_str()) != source
                                }) {
                                    self.settings.output_id.clone_from(&output.id);
                                }
                            }
                            self.endpoints = endpoints;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.error = Some("장치 목록을 읽는 작업이 종료됐습니다.".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if Instant::now() >= self.next_refresh {
            self.refresh();
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|w| w.thread.as_ref().is_some_and(JoinHandle::is_finished))
        {
            let Some(mut worker) = self.worker.take() else { return };
            if let Some(thread) = worker.thread.take() {
                match thread.join() {
                    Ok(Ok(())) => {
                        self.notice =
                            "모니터링을 중지했습니다. 케이블 자체는 계속 사용할 수 있습니다."
                                .into();
                    }
                    Ok(Err(error)) => self.error = Some(error),
                    Err(_) => self.error = Some("오디오 작업이 예기치 않게 종료됐습니다.".into()),
                }
            }
        }
    }
    fn start(&mut self) -> anyhow::Result<()> {
        self.settings.validate_route(&self.endpoints)?;
        let backend = self.backend.clone().ok_or_else(|| {
            anyhow::anyhow!("이 운영체제의 오디오 백엔드는 아직 제공되지 않습니다.")
        })?;
        let cable =
            self.settings.cable.clone().ok_or_else(|| anyhow::anyhow!("케이블을 선택하세요."))?;
        let output = self.settings.output_id.clone();
        if self
            .worker
            .as_ref()
            .is_some_and(|w| w.cable == cable && w.output_id == output && !w.control.is_stopped())
        {
            self.apply_volume();
            return Ok(());
        }
        // Validate the destination before stopping the current route. Join its
        // owned worker before starting another, so outputs never overlap.
        drop(self.worker.take());
        let control = Arc::new(RouteControl::new(self.settings.gain_db, self.settings.muted)?);
        let route = control.clone();
        let worker_cable = cable.clone();
        let worker_output = output.clone();
        let thread = std::thread::spawn(move || {
            backend.monitor(&cable, &output, route).map_err(|e| format!("{e:#}"))
        });
        self.worker = Some(Worker {
            control,
            thread: Some(thread),
            cable: worker_cable,
            output_id: worker_output,
        });
        self.error = None;
        self.notice = "지정한 볼륨으로 모니터링을 시작합니다.".into();
        Ok(())
    }
    fn apply_volume(&mut self) {
        if let Some(worker) = &self.worker {
            if self.settings.cable.as_ref() != Some(&worker.cable)
                || self.settings.output_id != worker.output_id
            {
                return;
            }
            if let Err(error) = worker.control.set_gain(self.settings.gain_db) {
                self.error = Some(error.to_string());
            }
            worker.control.set_mute(self.settings.muted);
        }
    }
    fn use_preset(&mut self, index: usize) -> anyhow::Result<()> {
        let preset = self
            .settings
            .presets
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("프리셋이 없습니다."))?
            .clone();
        preset.settings().validate_route(&self.endpoints)?;
        self.settings.cable = preset.cable;
        self.settings.output_id = preset.output_id;
        self.settings.gain_db = preset.gain_db;
        self.settings.muted = preset.muted;
        self.preset_name = preset.name;
        self.start()
    }
    fn save_preset(&mut self) -> anyhow::Result<()> {
        let name = self.preset_name.trim().to_owned();
        if name.is_empty() {
            anyhow::bail!("프리셋 이름을 입력하세요.");
        }
        self.settings.validate_route(&self.endpoints)?;
        let preset = Preset {
            name: name.clone(),
            cable: self.settings.cable.clone(),
            output_id: self.settings.output_id.clone(),
            gain_db: self.settings.gain_db,
            muted: self.settings.muted,
        };
        let mut saved = self.settings.clone();
        if let Some(existing) = saved.presets.iter_mut().find(|p| p.name == name) {
            *existing = preset;
        } else {
            saved.presets.push(preset);
        }
        saved.save(&self.path)?;
        self.settings = saved;
        self.notice = "프리셋을 저장했습니다. 위 버튼을 누르면 바로 적용됩니다.".into();
        Ok(())
    }
    fn ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Audio Cable");
                ui.label("가상 케이블의 소리를 원하는 볼륨으로 들으세요.");
                ui.heading("프리셋 — 클릭하면 바로 듣기");
                let mut activate = None;
                let mut delete = None;
                for (index, preset) in self.settings.presets.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let volume = preset.settings().volume_percent();
                        let output = self.endpoints.iter().find(|e| e.id == preset.output_id).map_or("연결되지 않은 장치", |e| e.name.as_str());
                        if ui.button(format!("{} · {volume:.4}%", preset.name)).on_hover_text(output).clicked() { activate = Some(index); }
                        if ui.small_button("삭제").clicked() { delete = Some(index); }
                    });
                }
                if self.settings.presets.is_empty() { ui.small("아래에서 장치와 볼륨을 정하고 이름을 붙여 저장하세요."); }
                if let Some(index) = activate && let Err(error) = self.use_preset(index) { self.error = Some(error.to_string()); }
                if let Some(index) = delete {
                    let mut saved = self.settings.clone();
                    saved.presets.remove(index);
                    match saved.save(&self.path) { Ok(()) => self.settings = saved, Err(error) => self.error = Some(error.to_string()) }
                }
                ui.separator();
                ui.heading("가상 오디오 케이블");
                ui.add_enabled_ui(self.backend.is_some(), |ui| {
                    let selected = self.settings.cable.as_ref().map_or("케이블을 선택하세요", |c| c.name.as_str());
                    egui::ComboBox::from_id_salt("cable").width(ui.available_width()).selected_text(selected).show_ui(ui, |ui| {
                        for cable in &self.cables {
                            let selected = self.settings.cable.as_ref() == Some(cable);
                            if ui.selectable_label(selected, &cable.name).clicked() { self.settings.cable = Some(cable.clone()); }
                        }
                    });
                    if self.cables.is_empty() && self.pending.is_none() {
                        ui.label("자동으로 찾은 케이블이 없습니다. 설치된 장치를 아래에서 직접 지정할 수도 있습니다.");
                        if let Some(backend) = &self.backend {
                            for link in backend.setup_links() { ui.hyperlink_to(link.label, link.url); }
                        }
                    }
                    egui::CollapsingHeader::new("다른 드라이버 / 케이블 장치 직접 지정").show(ui, |ui| {
                        ui.label("가상 드라이버가 만든 재생·녹음 장치 쌍을 지정하세요.");
                        selector(ui, "manual-render", "앱이 소리를 보내는 재생 장치", &mut self.manual_playback, &self.endpoints, EndpointFlow::Playback, None);
                        selector(ui, "manual-capture", "다른 앱이 소리를 받는 녹음 장치", &mut self.manual_capture, &self.endpoints, EndpointFlow::Capture, None);
                        if ui.button("이 장치 쌍 사용").clicked() {
                            let cable = Cable { provider: "installed.manual".into(), name: "직접 지정한 케이블".into(), playback_id: self.manual_playback.clone(), capture_id: self.manual_capture.clone() };
                            match cable.validate(&self.endpoints) { Ok(()) => self.settings.cable = Some(cable), Err(e) => self.error = Some(e.to_string()) }
                        }
                    });
                    ui.label("들을 출력 장치");
                    selector(ui, "output", "출력 장치를 선택하세요", &mut self.settings.output_id, &self.endpoints, EndpointFlow::Playback, self.settings.cable.as_ref().map(|c| c.playback_id.as_str()));
                });
                if let Some(cable) = &self.settings.cable {
                    let capture = self.endpoints.iter().find(|e| e.id == cable.capture_id).map_or("연결되지 않음", |e| e.name.as_str());
                    ui.small(format!("OBS / 통화 앱에서 받을 장치: {capture}"));
                }
                ui.separator();
                ui.heading("들을 볼륨");
                let mut percent = self.settings.volume_percent();
                if ui.add(egui::Slider::new(&mut percent, 0.0..=100.0).suffix("%").max_decimals(4)).changed() {
                    match self.settings.set_volume_percent(percent) { Ok(()) => self.apply_volume(), Err(e) => self.error = Some(e.to_string()) }
                }
                ui.small("100% = 원래 볼륨 · 10% = 진폭 1/10 · 0% = 무음. 숫자를 직접 입력할 수 있습니다.");
                ui.small("현재 듣는 장치의 볼륨은 바로 반영됩니다. Windows 장치 볼륨은 바꾸지 않습니다.");
                ui.horizontal(|ui| {
                    ui.label("프리셋 이름");
                    ui.add(egui::TextEdit::singleline(&mut self.preset_name).desired_width(180.0));
                    if ui.button("프리셋 저장").clicked() && let Err(e) = self.save_preset() { self.error = Some(e.to_string()); }
                });
                ui.horizontal(|ui| {
                        if ui.add_enabled(self.backend.is_some() && self.pending.is_none(), egui::Button::new("지금 적용하고 듣기")).clicked()
                            && let Err(error) = self.start() { self.error = Some(error.to_string());
                        }
                    if self.worker.is_some() && ui.button("듣기 중지").clicked()
                        && let Some(worker) = &self.worker { worker.control.stop();
                    }
                });
                let status = self.worker.as_ref().map_or("중지됨", |w| match w.control.state() { RouteState::Starting => "시작 중", RouteState::Running => "모니터링 중", RouteState::Recovering => "장치 재연결 대기", RouteState::Stopped => "중지 중" });
                ui.label(format!("상태: {status}"));
                ui.small(&self.notice);
                if self.backend.is_none() { ui.colored_label(Color32::DARK_RED, "이 빌드는 Windows 우선 버전입니다. Linux PipeWire 백엔드는 별도 구현 예정입니다."); }
                if let Some(error) = &self.error { ui.colored_label(Color32::DARK_RED, error); }
                ui.separator();
                ui.small("사용할 앱의 출력 → 케이블 재생 장치 → 이 앱 → 들을 장치");
                ui.small("케이블 녹음 장치는 원래 볼륨을 전달합니다. 감쇠는 이 앱으로 듣는 소리에 적용됩니다.");
            });
        });
    }
    #[cfg(feature = "ui-smoke")]
    fn capture_ui(&mut self, ctx: &egui::Context) {
        let Some((path, started, requested)) = self.screenshot.as_mut() else { return };
        if !*requested && started.elapsed() > Duration::from_secs(2) && self.pending.is_none() {
            *requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot { image, .. } = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(screenshot) = screenshot {
            let pixels: Vec<u8> = screenshot.pixels.iter().flat_map(Color32::to_array).collect();
            let result = (|| -> anyhow::Result<()> {
                image::save_buffer(
                    path,
                    &pixels,
                    screenshot.width().try_into()?,
                    screenshot.height().try_into()?,
                    image::ColorType::Rgba8,
                )?;
                Ok(())
            })();
            if let Err(e) = result {
                self.error = Some(e.to_string());
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
impl eframe::App for CablePanel {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll();
        self.ui(ctx);
        #[cfg(feature = "ui-smoke")]
        self.capture_ui(ctx);
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}

fn selector(
    ui: &mut egui::Ui,
    id: &str,
    prompt: &str,
    selected: &mut String,
    endpoints: &[Endpoint],
    flow: EndpointFlow,
    exclude: Option<&str>,
) {
    let label = endpoints
        .iter()
        .find(|e| e.id == *selected && e.flow == flow)
        .map_or(prompt, |e| e.name.as_str());
    egui::ComboBox::from_id_salt(id).width(ui.available_width()).selected_text(label).show_ui(
        ui,
        |ui| {
            for endpoint in
                endpoints.iter().filter(|e| e.flow == flow && Some(e.id.as_str()) != exclude)
            {
                ui.selectable_value(selected, endpoint.id.clone(), &endpoint.name);
            }
        },
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;
    struct FakeBackend;
    impl AudioBackend for FakeBackend {
        fn endpoints(&self) -> anyhow::Result<Vec<Endpoint>> {
            Ok(vec![
                Endpoint {
                    id: "in".into(),
                    name: "Custom playback".into(),
                    flow: EndpointFlow::Playback,
                    is_default: false,
                },
                Endpoint {
                    id: "out".into(),
                    name: "Custom capture".into(),
                    flow: EndpointFlow::Capture,
                    is_default: false,
                },
                Endpoint {
                    id: "speaker".into(),
                    name: "Speakers".into(),
                    flow: EndpointFlow::Playback,
                    is_default: true,
                },
                Endpoint {
                    id: "headphones".into(),
                    name: "Headphones".into(),
                    flow: EndpointFlow::Playback,
                    is_default: false,
                },
            ])
        }
        fn installed_cables(&self, _: &[Endpoint]) -> Vec<Cable> {
            vec![Cable {
                provider: "test.own-driver".into(),
                name: "Custom cable".into(),
                playback_id: "in".into(),
                capture_id: "out".into(),
            }]
        }
        fn monitor(&self, _: &Cable, _: &str, control: Arc<RouteControl>) -> anyhow::Result<()> {
            control.set_state(RouteState::Running);
            while !control.is_stopped() {
                std::thread::sleep(Duration::from_millis(1));
            }
            control.set_state(RouteState::Stopped);
            Ok(())
        }
    }
    #[test]
    fn provider_neutral_gui_updates_gain_without_restarting_and_stops_owned_route() {
        let root = tempfile::tempdir().unwrap();
        let mut panel =
            CablePanel::new(root.path().join("settings.json"), Some(Arc::new(FakeBackend)));
        for _ in 0..100 {
            panel.poll();
            if panel.pending.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(panel.settings.cable.as_ref().unwrap().provider, "test.own-driver");
        assert!(panel.worker.is_none());
        panel.start().unwrap();
        let control = panel.worker.as_ref().unwrap().control.clone();
        panel.settings.gain_db = -60.0;
        panel.apply_volume();
        assert!((control.target_gain() - 0.001).abs() < 1e-6);
        assert!(Arc::ptr_eq(&control, &panel.worker.as_ref().unwrap().control));
        panel.settings.muted = true;
        panel.apply_volume();
        assert_eq!(control.target_gain(), 0.0);
        let context = egui::Context::default();
        let _ = context.run(egui::RawInput::default(), |ctx| panel.ui(ctx));
        drop(panel);
        assert!(control.is_stopped());
    }
    #[test]
    fn saved_presets_start_and_switch_output_in_one_action_and_survive_reload() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let mut panel = CablePanel::new(path.clone(), Some(Arc::new(FakeBackend)));
        for _ in 0..100 {
            panel.poll();
            if panel.pending.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panel.preset_name = "스피커".into();
        panel.settings.set_volume_percent(10.0).unwrap();
        panel.save_preset().unwrap();
        panel.preset_name = "헤드폰".into();
        panel.settings.output_id = "headphones".into();
        panel.settings.set_volume_percent(1.0).unwrap();
        panel.save_preset().unwrap();
        assert!(panel.worker.is_none()); // Saving never starts audio.
        panel.use_preset(0).unwrap();
        let first = panel.worker.as_ref().unwrap().control.clone();
        assert_eq!(panel.worker.as_ref().unwrap().output_id, "speaker");
        assert!((first.target_gain() - 0.1).abs() < 1e-6);
        panel.use_preset(1).unwrap();
        assert!(first.is_stopped());
        assert_eq!(panel.worker.as_ref().unwrap().output_id, "headphones");
        let second = panel.worker.as_ref().unwrap().control.clone();
        assert!((second.target_gain() - 0.01).abs() < 1e-6);
        panel.use_preset(1).unwrap();
        assert!(Arc::ptr_eq(&second, &panel.worker.as_ref().unwrap().control));
        panel.endpoints.retain(|e| e.id != "speaker");
        assert!(panel.use_preset(0).is_err());
        assert!(!second.is_stopped()); // An unavailable preset leaves playback alone.
        drop(panel);
        let loaded = CablePanel::new(path, Some(Arc::new(FakeBackend)));
        assert_eq!(loaded.settings.presets.len(), 2);
        assert!(loaded.worker.is_none());
        assert!((loaded.settings.presets[1].settings().volume_percent() - 1.0).abs() < 1e-6);
    }
}
