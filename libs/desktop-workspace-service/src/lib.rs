//! Desktop job and configuration ownership independent of any web or native UI.
#![allow(clippy::multiple_crate_versions)]

mod process;
mod worker;
use anyhow::{Context, Result, bail};
use audio_cable_core::{AudioBackend, Cable, Endpoint, EndpointFlow, Settings as CableSettings};
use desktop_presets::{DisplaySettings, PresetStore, Settings};
use process::{OwnedProcess, WindowSession};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use window_manager_core::{Configuration, Node, Placement, WindowRef, new_id};
pub use worker::run_audio_worker;

#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Command {
    #[serde(rename = "snapshot")]
    Snapshot {},
    #[serde(rename = "refresh")]
    Refresh {},
    #[serde(rename = "audio.select")]
    AudioSelect { role: String, id: String },
    #[serde(rename = "audio.start")]
    AudioStart {},
    #[serde(rename = "audio.stop")]
    AudioStop {},
    #[serde(rename = "settings.save")]
    Save {},
    #[serde(rename = "settings.reload")]
    Reload {},
    #[serde(rename = "preset.save")]
    PresetSave {
        name: String,
        #[serde(default)]
        overwrite: bool,
    },
    #[serde(rename = "preset.load")]
    PresetLoad { name: String },
    #[serde(rename = "preset.delete")]
    PresetDelete { name: String },
    #[serde(rename = "cable.select")]
    CableSelect { id: String },
    #[serde(rename = "cable.manual")]
    CableManual { name: String, playback: String, capture: String },
    #[serde(rename = "cable.output")]
    CableOutput { id: String },
    #[serde(rename = "cable.volume")]
    CableVolume { value: f32 },
    #[serde(rename = "cable.mute")]
    CableMute { muted: bool },
    #[serde(rename = "cable.start")]
    CableStart {},
    #[serde(rename = "cable.stop")]
    CableStop {},
    #[serde(rename = "cable.save")]
    CableSave {},
    #[serde(rename = "cable.preset.save")]
    CablePresetSave { name: String },
    #[serde(rename = "cable.preset.load")]
    CablePresetLoad { name: String },
    #[serde(rename = "cable.preset.delete")]
    CablePresetDelete { name: String },
    #[serde(rename = "mirror.settings")]
    MirrorSettings { index: usize, settings: DisplaySettings },
    #[serde(rename = "mirror.add")]
    MirrorAdd {},
    #[serde(rename = "mirror.remove")]
    MirrorRemove { index: usize },
    #[serde(rename = "mirror.start")]
    MirrorStart { index: usize },
    #[serde(rename = "mirror.stop")]
    MirrorStop { index: usize },
    #[serde(rename = "windows.connect")]
    WindowsConnect {},
    #[serde(rename = "windows.disconnect")]
    WindowsDisconnect {},
    #[serde(rename = "windows.session")]
    WindowsSession { command: Value },
    #[serde(rename = "windows.add")]
    WindowsAdd { candidates: Vec<String>, view: String, root: String },
    #[serde(rename = "windows.view.add")]
    WindowsViewAdd { name: String },
    #[serde(rename = "windows.slot.add")]
    WindowsSlotAdd { name: String, device: String },
    #[serde(rename = "stop_all")]
    StopAll {},
    #[serde(rename = "shutdown")]
    Shutdown {},
}

pub struct Controller {
    root: PathBuf,
    binaries: PathBuf,
    store: PresetStore,
    draft: Settings,
    cable: CableSettings,
    cable_disk: Value,
    traces: Vec<Value>,
    endpoints: Vec<Endpoint>,
    cables: Vec<Cable>,
    displays: Value,
    errors: Vec<String>,
    storage_error: Option<String>,
    cable_storage_error: Option<String>,
    audio: Option<OwnedProcess>,
    monitor: Option<OwnedProcess>,
    mirrors: BTreeMap<usize, OwnedProcess>,
    session: Option<WindowSession>,
    wm: Value,
    sequence: u64,
    last_wm_poll: std::time::Instant,
    pub shutdown: bool,
}

fn name_valid(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        bail!("이름은 1~80자의 한 줄로 입력하세요.");
    }
    Ok(name)
}
fn stopped() -> Value {
    json!({"phase":"stopped","error":null,"logs":[]})
}

impl Controller {
    #[must_use]
    pub fn new(root: PathBuf, binaries: PathBuf) -> Self {
        let (store, storage_error) = match PresetStore::load(&root.join("presets.json")) {
            Ok(store) => (store, None),
            Err(error) => (PresetStore::default(), Some(format!("{error:#}"))),
        };
        let (cable, cable_storage_error) =
            match CableSettings::load(&root.join("audio-cable/settings.json")) {
                Ok(settings) => (settings, None),
                Err(error) => (CableSettings::default(), Some(format!("{error:#}"))),
            };
        Self {
            draft: store.last_used.clone(),
            root,
            binaries,
            store,
            cable_disk: serde_json::to_value(&cable).unwrap_or(Value::Null),
            traces: vec![],
            cable,
            endpoints: vec![],
            cables: vec![],
            displays: json!([]),
            errors: vec![],
            storage_error,
            cable_storage_error,
            audio: None,
            monitor: None,
            mirrors: BTreeMap::new(),
            session: None,
            wm: json!({"connected":false,"configuration":null,"inventory":[],"snapshot":null,"monitors":null,"error":null,"lastResult":null}),
            sequence: 0,
            last_wm_poll: std::time::Instant::now(),
            shutdown: false,
        }
    }
    fn executable(&self, name: &str) -> Result<PathBuf> {
        let path = self.binaries.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        if !path.is_file() {
            bail!("실행 파일이 없습니다: {}", path.display());
        }
        Ok(path)
    }
    pub fn refresh(&mut self) -> Result<()> {
        self.errors.clear();
        match native_endpoints() {
            Ok(endpoints) => {
                self.cables = discover_cables(&endpoints);
                self.endpoints = endpoints;
            }
            Err(error) => self.errors.push(format!("오디오 장치 조회: {error:#}")),
        }
        match windows_desktop_duplication::enumerate_displays() {
            Ok(displays) => self.displays = json!(displays.into_iter().map(|display| json!({"id":display.name,"name":display.friendly_name,"width":display.area.width,"height":display.area.height})).collect::<Vec<_>>()),
            Err(error) => self.errors.push(format!("디스플레이 조회: {error:#}")),
        }
        Ok(())
    }
    fn playback(&self, id: &str) -> Result<&Endpoint> {
        self.endpoints
            .iter()
            .find(|endpoint| {
                endpoint.flow == EndpointFlow::Playback
                    && (endpoint.id == id || id == "default" && endpoint.is_default)
            })
            .context("선택한 출력 장치가 연결되어 있지 않습니다. 장치를 다시 검색하세요.")
    }
    fn stop_audio(&mut self) -> Result<()> {
        if let Some(worker) = self.audio.as_mut() {
            worker.stop()?;
        }
        Ok(())
    }
    fn stop_monitor(&mut self) -> Result<()> {
        if let Some(worker) = self.monitor.as_mut() {
            worker.stop()?;
        }
        Ok(())
    }
    fn save(&mut self) -> Result<()> {
        if let Some(error) = &self.storage_error {
            bail!("기존 설정 파일 보호: {error}");
        }
        self.draft.validate()?;
        self.check_store_disk()?;
        let mut store = self.store.clone();
        store.last_used = self.draft.clone();
        store.save(&self.root.join("presets.json"))?;
        self.store = store;
        Ok(())
    }
    fn save_cable(&mut self) -> Result<()> {
        if let Some(error) = &self.cable_storage_error {
            bail!("기존 케이블 설정 보호: {error}");
        }
        self.check_cable_disk()?;
        self.cable.save(&self.root.join("audio-cable/settings.json"))?;
        self.cable_disk = serde_json::to_value(&self.cable)?;
        Ok(())
    }
    fn check_store_disk(&self) -> Result<()> {
        if PresetStore::load(&self.root.join("presets.json"))? != self.store {
            bail!("다른 앱이 프리셋 파일을 변경했습니다. 출력들을 중지하고 설정을 다시 읽으세요.");
        }
        Ok(())
    }
    fn check_cable_disk(&self) -> Result<()> {
        if serde_json::to_value(CableSettings::load(&self.root.join("audio-cable/settings.json"))?)?
            != self.cable_disk
        {
            bail!("다른 앱이 케이블 설정을 변경했습니다. 출력을 중지하고 설정을 다시 읽으세요.");
        }
        Ok(())
    }
    pub fn record_trace(&mut self, id: &str, operation: &str, ok: bool, message: &str) {
        self.sequence += 1;
        if operation == "snapshot" {
            return;
        }
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        self.traces.push(json!({"id":id,"timestamp":timestamp,"operation":operation,"status":if ok {"ok"} else {"error"},"message":message.chars().take(3000).collect::<String>()}));
        if self.traces.len() > 100 {
            self.traces.remove(0);
        }
    }
    fn audio_worker(
        &self,
        source: &str,
        target: &str,
        gain_db: f32,
        muted: bool,
    ) -> Result<OwnedProcess> {
        if self.playback(source)?.id == self.playback(target)?.id {
            bail!("원본과 대상이 같은 장치입니다. 피드백을 방지하려면 다른 출력을 선택하세요.");
        }
        let mut worker = OwnedProcess::spawn(
            &self.executable("desktop-workspace-host")?,
            &["--audio-worker".into()],
            "starting",
        )?;
        worker.write(&json!({"source":source,"target":target,"gain_db":gain_db,"muted":muted}))?;
        Ok(worker)
    }
    fn wm_request(&mut self, command: &Value) -> Result<Value> {
        self.sequence += 1;
        let id = format!("workspace-{}", self.sequence);
        let response =
            self.session.as_mut().context("창 관리자에 먼저 연결하세요.")?.request(&id, command);
        if let Err(error) = &response {
            self.wm["error"] = json!(format!("{error:#}"));
        }
        response
    }
    fn wm_refresh(&mut self, inventory: bool) -> Result<()> {
        self.wm["configuration"] = self.wm_request(&json!({"kind":"configuration"}))?;
        self.wm["snapshot"] = self.wm_request(&json!({"kind":"snapshot"}))?;
        if inventory {
            self.wm["inventory"] = self.wm_request(&json!({"kind":"inventory"}))?;
            self.wm["monitors"] = self.wm_request(&json!({"kind":"monitor_inventory"}))?;
        }
        Ok(())
    }
    #[allow(clippy::too_many_lines)] // One exhaustive dispatch keeps job ownership and save guards together.
    pub fn execute(&mut self, command: Command) -> Result<Value> {
        self.poll()?;
        let mut result = Value::Null;
        match command {
            Command::Snapshot {} => {
                if self.session.is_some()
                    && self.last_wm_poll.elapsed() >= std::time::Duration::from_secs(2)
                {
                    self.last_wm_poll = std::time::Instant::now();
                    if let Ok(snapshot) = self.wm_request(&json!({"kind":"snapshot"})) {
                        self.wm["snapshot"] = snapshot;
                    } else {
                        self.session = None;
                        self.wm["connected"] = json!(false);
                    }
                }
            }
            Command::Refresh {} => {
                self.refresh()?;
                if self.session.is_some() {
                    self.wm_refresh(true)?;
                }
            }
            Command::AudioSelect { role, id } => {
                let label = self.playback(&id)?.name.clone();
                if !matches!(role.as_str(), "source" | "target") {
                    bail!("원본 또는 대상 역할이 필요합니다.");
                }
                self.stop_audio()?;
                if role == "source" {
                    self.draft.audio.source = id;
                    self.draft.audio.source_label = label;
                } else {
                    self.draft.audio.target = id;
                    self.draft.audio.target_label = label;
                }
            }
            Command::AudioStart {} => {
                if self.audio.as_ref().is_some_and(OwnedProcess::alive) {
                    bail!("오디오가 이미 실행 중입니다.");
                }
                self.refresh()?;
                let worker = self.audio_worker(
                    &self.draft.audio.source,
                    &self.draft.audio.target,
                    0.0,
                    false,
                )?;
                self.audio = Some(worker);
            }
            Command::AudioStop {} => self.stop_audio()?,
            Command::Save {} => self.save()?,
            Command::Reload {} => {
                if self.audio.as_ref().is_some_and(OwnedProcess::alive)
                    || self.monitor.as_ref().is_some_and(OwnedProcess::alive)
                    || self.mirrors.values().any(OwnedProcess::alive)
                {
                    bail!("설정을 다시 읽기 전에 출력을 모두 중지하세요.");
                }
                let store = PresetStore::load(&self.root.join("presets.json"))?;
                let cable = CableSettings::load(&self.root.join("audio-cable/settings.json"))?;
                self.cable_disk = serde_json::to_value(&cable)?;
                self.cable = cable;
                self.draft = store.last_used.clone();
                self.store = store;
                self.storage_error = None;
                self.cable_storage_error = None;
            }
            Command::PresetSave { name, overwrite } => {
                self.check_store_disk()?;
                let name = name_valid(&name)?.to_owned();
                let mut store = self.store.clone();
                if let Some(preset) = store.presets.iter_mut().find(|preset| preset.name == name) {
                    if !overwrite {
                        bail!("같은 이름의 프리셋이 있습니다. 덮어쓰기를 선택하세요.");
                    }
                    self.draft.validate()?;
                    preset.settings = self.draft.clone();
                } else {
                    store.add(&name, self.draft.clone())?;
                }
                if let Some(error) = &self.storage_error {
                    bail!("기존 설정 보호: {error}");
                }
                store.last_used = self.draft.clone();
                store.save(&self.root.join("presets.json"))?;
                self.store = store;
            }
            Command::PresetLoad { name } => {
                let settings = self
                    .store
                    .presets
                    .iter()
                    .find(|preset| preset.name == name)
                    .context("프리셋을 찾을 수 없습니다.")?
                    .settings
                    .clone();
                if self.audio.as_ref().is_some_and(OwnedProcess::alive)
                    || self.mirrors.values().any(OwnedProcess::alive)
                {
                    bail!("프리셋을 불러오기 전에 실행 중인 출력을 중지하세요.");
                }
                self.draft = settings;
            }
            Command::PresetDelete { name } => {
                self.check_store_disk()?;
                if let Some(error) = &self.storage_error {
                    bail!("기존 설정 보호: {error}");
                }
                let mut store = self.store.clone();
                store.presets.retain(|preset| preset.name != name);
                store.save(&self.root.join("presets.json"))?;
                self.store = store;
            }
            Command::CableSelect { id } => {
                let cable = self
                    .cables
                    .iter()
                    .find(|cable| cable.playback_id == id)
                    .context("케이블을 찾을 수 없습니다.")?
                    .clone();
                self.stop_monitor()?;
                self.cable.cable = Some(cable);
            }
            Command::CableManual { name, playback, capture } => {
                let cable = Cable {
                    name: name_valid(&name)?.into(),
                    provider: "manual".into(),
                    playback_id: playback,
                    capture_id: capture,
                };
                cable.validate(&self.endpoints)?;
                self.stop_monitor()?;
                self.cable.cable = Some(cable);
            }
            Command::CableOutput { id } => {
                self.playback(&id)?;
                if id == "default" {
                    bail!("케이블 모니터링은 고정 출력 장치를 선택하세요.");
                }
                self.stop_monitor()?;
                self.cable.output_id = id;
            }
            Command::CableVolume { value } => {
                self.cable.set_volume_percent(value)?;
                if let Some(worker) = self.monitor.as_mut().filter(|worker| worker.alive()) {
                    worker
                        .write(&json!({"gain_db":self.cable.gain_db,"muted":self.cable.muted}))?;
                }
            }
            Command::CableMute { muted } => {
                self.cable.muted = muted;
                if let Some(worker) = self.monitor.as_mut().filter(|worker| worker.alive()) {
                    worker.write(&json!({"gain_db":self.cable.gain_db,"muted":muted}))?;
                }
            }
            Command::CableStart {} => {
                if self.monitor.as_ref().is_some_and(OwnedProcess::alive) {
                    bail!("케이블 모니터링이 이미 실행 중입니다.");
                }
                self.refresh()?;
                self.cable.validate_route(&self.endpoints)?;
                let cable = self.cable.cable.as_ref().context("케이블을 선택하세요.")?;
                self.monitor = Some(self.audio_worker(
                    &cable.playback_id,
                    &self.cable.output_id,
                    self.cable.gain_db,
                    self.cable.muted,
                )?);
            }
            Command::CableStop {} => self.stop_monitor()?,
            Command::CableSave {} => self.save_cable()?,
            Command::CablePresetSave { name } => {
                self.check_cable_disk()?;
                let name = name_valid(&name)?.to_owned();
                let mut draft = self.cable.clone();
                draft.presets.retain(|preset| preset.name != name);
                draft.presets.push(audio_cable_core::Preset {
                    name,
                    cable: draft.cable.clone(),
                    output_id: draft.output_id.clone(),
                    gain_db: draft.gain_db,
                    muted: draft.muted,
                });
                if let Some(error) = &self.cable_storage_error {
                    bail!("기존 설정 보호: {error}");
                }
                draft.save(&self.root.join("audio-cable/settings.json"))?;
                self.cable_disk = serde_json::to_value(&draft)?;
                self.cable = draft;
            }
            Command::CablePresetLoad { name } => {
                let settings = self
                    .cable
                    .presets
                    .iter()
                    .find(|preset| preset.name == name)
                    .context("프리셋을 찾을 수 없습니다.")?
                    .settings();
                self.stop_monitor()?;
                self.cable.cable = settings.cable;
                self.cable.output_id = settings.output_id;
                self.cable.gain_db = settings.gain_db;
                self.cable.muted = settings.muted;
            }
            Command::CablePresetDelete { name } => {
                self.check_cable_disk()?;
                if let Some(error) = &self.cable_storage_error {
                    bail!("기존 설정 보호: {error}");
                }
                let mut draft = self.cable.clone();
                draft.presets.retain(|preset| preset.name != name);
                draft.save(&self.root.join("audio-cable/settings.json"))?;
                self.cable_disk = serde_json::to_value(&draft)?;
                self.cable = draft;
            }
            Command::MirrorSettings { index, settings } => {
                if self.mirrors.get(&index).is_some_and(OwnedProcess::alive) {
                    bail!("설정을 바꾸기 전에 해당 미러링을 중지하세요.");
                }
                settings.validate()?;
                *self.draft.display_at_mut(index).context("미러링 행이 없습니다.")? = settings;
            }
            Command::MirrorAdd {} => {
                if self.draft.additional_displays.len() >= 15 {
                    bail!("미러링은 최대 16개입니다.");
                }
                self.draft.additional_displays.push(DisplaySettings::default());
            }
            Command::MirrorRemove { index } => {
                if index == 0 {
                    bail!("첫 미러링 행은 삭제할 수 없습니다.");
                }
                if self.mirrors.values().any(OwnedProcess::alive) {
                    bail!("행을 삭제하기 전에 미러링을 중지하세요.");
                }
                if index > self.draft.additional_displays.len() {
                    bail!("미러링 행이 없습니다.");
                }
                self.draft.additional_displays.remove(index - 1);
                self.mirrors.clear();
            }
            Command::MirrorStart { index } => {
                if self.mirrors.get(&index).is_some_and(OwnedProcess::alive) {
                    bail!("미러링이 이미 실행 중입니다.");
                }
                let settings =
                    self.draft.display_at(index).context("미러링 행이 없습니다.")?.clone();
                if !settings.enabled {
                    bail!("미러링 행을 활성화한 뒤 시작하세요.");
                }
                settings.validate()?;
                if !self.displays.as_array().is_some_and(|displays| {
                    displays.iter().any(|display| display["id"] == settings.display)
                }) {
                    bail!("선택한 디스플레이가 연결되어 있지 않습니다.");
                }
                let state_kind =
                    if index == 0 { "relay".into() } else { format!("relay-{}", index + 1) };
                let mut args = settings.args();
                args.push("--window-state-file".into());
                args.push(
                    desktop_presets::window_state_path(
                        &self.root.join("presets.json"),
                        &state_kind,
                    )
                    .to_string_lossy()
                    .into(),
                );
                let worker =
                    OwnedProcess::spawn(&self.executable("display-relay")?, &args, "running")?;
                self.mirrors.insert(index, worker);
            }
            Command::MirrorStop { index } => {
                if let Some(worker) = self.mirrors.get_mut(&index) {
                    worker.stop()?;
                }
            }
            Command::WindowsConnect {} => {
                if self.session.is_none() {
                    let session = WindowSession::spawn(
                        &self.executable("window-manager")?,
                        &self.root.join("window-manager.json"),
                    )?;
                    self.session = Some(session);
                }
                match self.wm_refresh(true) {
                    Ok(()) => {
                        self.wm["connected"] = json!(true);
                        self.wm["error"] = Value::Null;
                    }
                    Err(error) => {
                        self.session = None;
                        self.wm["connected"] = json!(false);
                        return Err(error);
                    }
                }
            }
            Command::WindowsDisconnect {} => {
                self.session = None;
                self.wm["connected"] = json!(false);
            }
            Command::WindowsSession { command } => {
                let kind = command["kind"].as_str().context("창 명령 종류가 없습니다.")?.to_owned();
                if kind.starts_with("provider") {
                    bail!("알림 제공자 연결은 이 UI 세션에 허용되지 않습니다.");
                }
                if matches!(
                    kind.as_str(),
                    "preview" | "undo" | "navigate" | "recall" | "window_action" | "collapse"
                ) && let Some(plan) = self.wm["lastResult"]["plan"].as_str().map(str::to_owned)
                {
                    self.wm_request(&json!({"kind":"discard_preview","plan":plan}))?;
                }
                result = self.wm_request(&command)?;
                self.wm["lastResult"] = result.clone();
                if kind == "inventory" {
                    self.wm["inventory"] = result.clone();
                } else if kind == "monitor_inventory" {
                    self.wm["monitors"] = result.clone();
                } else if kind == "snapshot" {
                    self.wm["snapshot"] = result.clone();
                } else if kind != "preview"
                    && kind != "navigate"
                    && kind != "undo"
                    && kind != "recall"
                    && kind != "window_action"
                    && kind != "collapse"
                {
                    self.wm_refresh(false)?;
                }
            }
            Command::WindowsAdd { candidates, view, root } => {
                result = self.add_windows(candidates, &view, &root)?;
            }
            Command::WindowsViewAdd { name } => {
                let mut config: Configuration =
                    serde_json::from_value(self.wm["configuration"].clone())?;
                let name = name_valid(&name)?.to_owned();
                let id = new_id("view");
                let workspace =
                    config.workspaces.keys().next().context("작업 공간이 없습니다.")?.clone();
                config.views.insert(
                    id.clone(),
                    window_manager_core::View {
                        id: id.clone(),
                        workspace,
                        name,
                        roots: BTreeMap::from([(
                            "main".into(),
                            Node::Group(window_manager_core::Group::new("기본 그룹".into())),
                        )]),
                    },
                );
                result = self.wm_request(&json!({"kind":"configure","expected_revision":config.revision,"configuration":config}))?;
                self.wm_refresh(false)?;
                result = json!({"view":id,"configuration":result});
            }
            Command::WindowsSlotAdd { name, device } => {
                let name = name_valid(&name)?.to_owned();
                let monitor = self.wm["monitors"]["monitors"]
                    .as_array()
                    .and_then(|monitors| {
                        monitors.iter().find(|monitor| monitor["device"] == device)
                    })
                    .context("현재 모니터 목록에서 화면을 선택하세요.")?;
                let alias =
                    monitor["hardware"].as_str().map_or_else(|| new_id("monitor"), str::to_owned);
                let epoch = self.wm["monitors"]["epoch"].clone();
                self.wm_request(
                    &json!({"kind":"confirm_monitor","epoch":epoch,"device":device,"alias":alias}),
                )?;
                let mut config: Configuration =
                    serde_json::from_value(self.wm["configuration"].clone())?;
                let id = new_id("slot");
                config.slots.insert(
                    id.clone(),
                    window_manager_core::DisplaySlot {
                        id: id.clone(),
                        name,
                        display: alias,
                        region: [0.0, 0.0, 1.0, 1.0],
                        fallback_displays: vec![],
                    },
                );
                result=self.wm_request(&json!({"kind":"configure","expected_revision":config.revision,"configuration":config}))?;
                self.wm_refresh(false)?;
                result = json!({"slot":id,"configuration":result});
            }
            Command::StopAll {} | Command::Shutdown {} => {
                let mut failures = vec![];
                if let Err(error) = self.stop_audio() {
                    failures.push(format!("오디오: {error:#}"));
                }
                if let Err(error) = self.stop_monitor() {
                    failures.push(format!("케이블: {error:#}"));
                }
                for worker in self.mirrors.values_mut() {
                    if let Err(error) = worker.stop() {
                        failures.push(format!("미러링: {error:#}"));
                    }
                }
                self.session = None;
                self.wm["connected"] = json!(false);
                self.shutdown = matches!(command, Command::Shutdown {});
                if !failures.is_empty() {
                    bail!("{}", failures.join("\n"));
                }
            }
        }
        Ok(json!({"state":self.snapshot(),"result":result}))
    }
    fn add_windows(&mut self, candidates: Vec<String>, view: &str, root: &str) -> Result<Value> {
        if candidates.is_empty() || candidates.len() > 64 {
            bail!("1~64개의 창을 선택하세요.");
        }
        if candidates.iter().collect::<std::collections::BTreeSet<_>>().len() != candidates.len() {
            bail!("같은 창을 중복 선택할 수 없습니다.");
        }
        let mut config: Configuration = serde_json::from_value(self.wm["configuration"].clone())?;
        let expected = config.revision;
        let inventory = self.wm["inventory"].as_array().context("창 목록이 없습니다.")?;
        let mut bindings = vec![];
        for candidate in candidates {
            let found = inventory
                .iter()
                .find(|item| item["id"] == candidate)
                .context("선택 토큰이 만료됐습니다. 목록을 다시 검색하세요.")?;
            let alias = found["title"].as_str().unwrap_or("창").to_owned();
            let existing =
                found["reference"].as_str().filter(|id| config.windows.contains_key(*id));
            let id = existing.map_or_else(|| new_id("window"), str::to_owned);
            let Some(Node::Group(group)) =
                config.views.get_mut(view).and_then(|view| view.roots.get_mut(root))
            else {
                bail!("대상 루트 그룹을 선택하세요.");
            };
            if group
                .children
                .iter()
                .any(|node| matches!(node, Node::Placement(placement) if placement.window == id))
            {
                bail!("선택한 창이 이미 이 루트에 등록되어 있습니다.");
            }
            group.children.push(Node::Placement(Placement::new(id.clone(), alias.clone())));
            if !group.ratios.is_empty() {
                group.ratios.push(1.0);
            }
            if existing.is_none() {
                config.windows.insert(id.clone(), WindowRef::unbound(id.clone(), alias));
                bindings.push((id, candidate));
            }
        }
        config.validate()?;
        self.wm_request(
            &json!({"kind":"configure","expected_revision":expected,"configuration":config}),
        )?;
        for (window, candidate) in bindings {
            self.wm_request(&json!({"kind":"bind","expected_revision":expected+1,"window":window,"candidate":candidate}))?;
        }
        self.wm_refresh(true)?;
        Ok(self.wm["configuration"].clone())
    }
    fn poll(&mut self) -> Result<()> {
        for worker in
            self.audio.iter_mut().chain(self.monitor.iter_mut()).chain(self.mirrors.values_mut())
        {
            worker.poll()?;
        }
        Ok(())
    }
    pub fn snapshot(&self) -> Value {
        json!({"revision":self.sequence,"traces":self.traces,"settings":{"last_used":self.draft,"presets":self.store.presets},"cableSettings":self.cable,"endpoints":self.endpoints.iter().map(|endpoint|json!({"id":endpoint.id,"name":endpoint.name,"flow":if endpoint.flow==EndpointFlow::Playback {"playback"} else {"capture"},"isDefault":endpoint.is_default})).collect::<Vec<_>>(),"cables":self.cables,"displays":self.displays,"errors":self.errors,"storageError":self.storage_error,"cableStorageError":self.cable_storage_error,"jobs":{"audio":self.audio.as_ref().map_or_else(stopped,OwnedProcess::snapshot),"cable":self.monitor.as_ref().map_or_else(stopped,OwnedProcess::snapshot),"mirrors":self.draft.displays().enumerate().map(|(index,_)|self.mirrors.get(&index).map_or_else(stopped,OwnedProcess::snapshot)).collect::<Vec<_>>()},"wm":self.wm,"configRoot":self.root})
    }
}

#[cfg(windows)]
fn native_endpoints() -> Result<Vec<Endpoint>> {
    windows_audio_router::WindowsAudioBackend.endpoints()
}
#[cfg(not(windows))]
fn native_endpoints() -> Result<Vec<Endpoint>> {
    bail!("Windows에서만 장치를 조회할 수 있습니다.")
}
#[cfg(windows)]
fn discover_cables(endpoints: &[Endpoint]) -> Vec<Cable> {
    windows_audio_router::WindowsAudioBackend.installed_cables(endpoints)
}
#[cfg(not(windows))]
fn discover_cables(_: &[Endpoint]) -> Vec<Cable> {
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn controller(root: &std::path::Path) -> Controller {
        let mut controller = Controller::new(root.to_path_buf(), root.to_path_buf());
        controller.endpoints = vec![
            Endpoint {
                id: "source".into(),
                name: "Source".into(),
                flow: EndpointFlow::Playback,
                is_default: true,
            },
            Endpoint {
                id: "target".into(),
                name: "Target".into(),
                flow: EndpointFlow::Playback,
                is_default: false,
            },
        ];
        controller.draft.audio.source = "default".into();
        controller.draft.audio.target = "target".into();
        controller
    }
    #[test]
    fn startup_never_writes_settings_or_starts_jobs() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let controller = controller(directory.path());
        assert_eq!(controller.snapshot()["jobs"]["audio"]["phase"], "stopped");
        assert!(directory.path().read_dir()?.next().is_none());
        assert!(controller.audio_worker("default", "source", 0.0, false).is_err());
        Ok(())
    }
    #[test]
    fn corrupt_existing_settings_are_never_overwritten() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("presets.json");
        std::fs::write(&path, b"malformed")?;
        let mut controller = controller(directory.path());
        assert!(controller.execute(Command::Save {}).is_err());
        assert!(
            controller
                .execute(Command::PresetSave { name: "test".into(), overwrite: false })
                .is_err()
        );
        assert_eq!(std::fs::read(&path)?, b"malformed");
        Ok(())
    }
    #[test]
    fn preset_round_trip_and_external_write_conflict() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().to_owned();
        let mut controller = controller(&root);
        controller.execute(Command::PresetSave { name: "saved".into(), overwrite: false })?;
        let mut stored = PresetStore::load(&root.join("presets.json"))?;
        assert_eq!(stored.presets.len(), 1);
        assert!(
            controller
                .execute(Command::PresetSave { name: "saved".into(), overwrite: false })
                .is_err()
        );
        stored.last_used.audio.target = "external".into();
        stored.save(&root.join("presets.json"))?;
        assert!(controller.execute(Command::Save {}).is_err());
        assert_eq!(
            PresetStore::load(&root.join("presets.json"))?.last_used.audio.target,
            "external"
        );
        controller.execute(Command::Reload {})?;
        assert_eq!(controller.snapshot()["settings"]["last_used"]["audio"]["target"], "external");
        Ok(())
    }
    #[test]
    fn mirror_bounds_and_command_schema_fail_closed() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut controller = controller(directory.path());
        assert!(controller.execute(Command::MirrorRemove { index: 0 }).is_err());
        assert!(controller.execute(Command::MirrorRemove { index: usize::MAX }).is_err());
        for _ in 0..15 {
            controller.execute(Command::MirrorAdd {})?;
        }
        assert!(controller.execute(Command::MirrorAdd {}).is_err());
        assert!(
            serde_json::from_value::<Command>(json!({"kind":"shutdown","shell":"evil"})).is_err()
        );
        assert!(
            serde_json::from_value::<Command>(json!({"kind":"exec","program":"anything"})).is_err()
        );
        Ok(())
    }
    #[test]
    fn invalid_cable_volume_and_external_cable_edit_are_rejected() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().to_owned();
        let mut controller = controller(&root);
        for value in [f32::NAN, f32::INFINITY, -1.0, 101.0] {
            assert!(controller.execute(Command::CableVolume { value }).is_err());
        }
        controller.execute(Command::CableVolume { value: 35.0 })?;
        controller.execute(Command::CableSave {})?;
        let mut settings = CableSettings::load(&root.join("audio-cable/settings.json"))?;
        settings.set_volume_percent(80.0)?;
        settings.save(&root.join("audio-cable/settings.json"))?;
        assert!(controller.execute(Command::CableSave {}).is_err());
        assert!(
            (CableSettings::load(&root.join("audio-cable/settings.json"))?.volume_percent() - 80.0)
                .abs()
                < 0.01
        );
        Ok(())
    }
}
