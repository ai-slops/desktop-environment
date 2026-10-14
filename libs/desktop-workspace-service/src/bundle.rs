use super::process::OwnedProcess;
use super::{Command, Controller, name_valid};
use anyhow::{Context, Result, bail};
use audio_cable_core::Settings as CableSettings;
use desktop_presets::{AudioSettings, DisplaySettings};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use window_manager_core::new_id;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowTarget {
    pub view: String,
    pub slot: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preset {
    audio: Option<AudioSettings>,
    displays: Option<Vec<DisplaySettings>>,
    cable: Option<CableSettings>,
    windows: Option<WindowTarget>,
}
impl Preset {
    pub(super) const fn window_target(&self) -> Option<&WindowTarget> {
        self.windows.as_ref()
    }
    pub(super) fn components(&self) -> Vec<&str> {
        let mut names = vec![];
        if self.audio.is_some() {
            names.push("audio");
        }
        if self.displays.is_some() {
            names.push("display");
        }
        if self.cable.is_some() {
            names.push("cable");
        }
        if self.windows.is_some() {
            names.push("windows");
        }
        names
    }
    fn validate(&self) -> Result<()> {
        if self.components().is_empty() {
            bail!("통합 프리셋에 포함할 기능을 선택하세요.");
        }
        if let Some(displays) = &self.displays {
            if displays.is_empty() || displays.len() > 16 {
                bail!("미러링 설정은 1~16개여야 합니다.");
            }
            for display in displays {
                display.validate()?;
            }
        }
        if let Some(cable) = &self.cable {
            cable.validate()?;
        }
        if let Some(windows) = &self.windows
            && (windows.view.is_empty() || windows.slot.is_empty())
        {
            bail!("창 배치와 화면 영역을 선택하세요.");
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Store {
    version: u32,
    pub presets: BTreeMap<String, Preset>,
}
impl Default for Store {
    fn default() -> Self {
        Self { version: 1, presets: BTreeMap::new() }
    }
}
impl Store {
    pub(super) fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        if path.metadata()?.len() > 1_048_576 {
            bail!("통합 프리셋 파일이 1 MiB를 초과합니다.");
        }
        let store: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        if store.version != 1 {
            bail!("지원하지 않는 통합 프리셋 버전입니다.");
        }
        for (name, preset) in &store.presets {
            name_valid(name)?;
            preset.validate()?;
        }
        Ok(store)
    }
    fn save(&self, path: &Path) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        if bytes.len() > 1_048_576 {
            bail!("통합 프리셋 파일이 1 MiB를 초과합니다.");
        }
        let parent = path.parent().context("설정 폴더가 없습니다.")?;
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).context("통합 프리셋을 저장할 수 없습니다.")?;
        Ok(())
    }
}
impl Controller {
    fn check_bundle_disk(&self) -> Result<()> {
        if let Some(error) = &self.bundle_storage_error {
            bail!("기존 통합 프리셋 보호: {error}");
        }
        if serde_json::to_value(Store::load(&self.root.join("workspace-presets.json"))?)?
            != self.bundle_disk
        {
            bail!("다른 앱이 통합 프리셋을 변경했습니다. 설정을 다시 읽으세요.");
        }
        Ok(())
    }
    pub(super) fn save_bundle(
        &mut self,
        name: &str,
        components: &[String],
        view: Option<&str>,
        slot: Option<&str>,
        overwrite: bool,
    ) -> Result<()> {
        self.check_bundle_disk()?;
        let name = name_valid(name)?.to_owned();
        if components.is_empty()
            || components.len() > 4
            || components.iter().any(|component| {
                !matches!(component.as_str(), "audio" | "display" | "cable" | "windows")
            })
        {
            bail!("포함할 기능을 선택하세요.");
        }
        if self.bundle_store.presets.contains_key(&name) && !overwrite {
            bail!("같은 이름의 통합 프리셋이 있습니다. 덮어쓰기를 선택하세요.");
        }
        let mut preset = Preset::default();
        for component in components {
            match component.as_str() {
                "audio" => preset.audio = Some(self.draft.audio.clone()),
                "display" => preset.displays = Some(self.draft.displays().cloned().collect()),
                "cable" => {
                    let mut cable = self.cable.clone();
                    cable.presets.clear();
                    preset.cable = Some(cable);
                }
                "windows" => {
                    let target = WindowTarget {
                        view: view.context("창 배치를 선택하세요.")?.to_owned(),
                        slot: slot.context("화면 영역을 선택하세요.")?.to_owned(),
                    };
                    if !self.wm["connected"].as_bool().unwrap_or(false)
                        || self.wm["configuration"]["views"][&target.view].is_null()
                        || self.wm["configuration"]["slots"][&target.slot].is_null()
                    {
                        bail!("창 관리자에 연결하고 유효한 배치와 영역을 선택하세요.");
                    }
                    preset.windows = Some(target);
                }
                _ => unreachable!(),
            }
        }
        preset.validate()?;
        let mut store = self.bundle_store.clone();
        store.presets.insert(name, preset);
        store.save(&self.root.join("workspace-presets.json"))?;
        self.bundle_disk = serde_json::to_value(&store)?;
        self.bundle_store = store;
        Ok(())
    }
    pub(super) fn delete_bundle(&mut self, name: &str) -> Result<()> {
        self.check_bundle_disk()?;
        let mut store = self.bundle_store.clone();
        store.presets.remove(name);
        store.save(&self.root.join("workspace-presets.json"))?;
        self.bundle_disk = serde_json::to_value(&store)?;
        self.bundle_store = store;
        Ok(())
    }
    #[allow(clippy::too_many_lines)] // Keep preflight and the ordered, reported multi-component transition together.
    pub(super) fn apply_bundle(&mut self, name: &str, start: bool) -> Result<Value> {
        let mut preset =
            self.bundle_store.presets.get(name).context("통합 프리셋을 찾을 수 없습니다.")?.clone();
        preset.validate()?;
        self.refresh()?;
        // Validate every selected route before changing any existing output.
        if let Some(audio) = &preset.audio
            && self.playback(&audio.source)?.id == self.playback(&audio.target)?.id
        {
            bail!("통합 프리셋 원본과 대상이 같은 장치입니다.");
        }
        if let Some(cable) = &preset.cable {
            cable.validate_route(&self.endpoints)?;
        }
        if start {
            let audio = preset.audio.as_ref().or_else(|| {
                self.audio.as_ref().filter(|worker| worker.alive()).map(|_| &self.draft.audio)
            });
            let cable = preset.cable.as_ref().or_else(|| {
                self.monitor.as_ref().filter(|worker| worker.alive()).map(|_| &self.cable)
            });
            if let (Some(audio), Some(cable)) = (audio, cable)
                && let Some(pair) = &cable.cable
            {
                let ids = (
                    self.playback(&audio.source).ok(),
                    self.playback(&audio.target).ok(),
                    self.playback(&pair.playback_id).ok(),
                    self.playback(&cable.output_id).ok(),
                );
                if let (Some(a), Some(b), Some(c), Some(d)) = ids
                    && a.id == d.id
                    && b.id == c.id
                {
                    bail!("Audio Repeater와 Audio Cable 경로가 서로 되먹임을 만듭니다.");
                }
            }
        }
        if let Some(displays) = &mut preset.displays {
            if start && !displays.iter().any(|display| display.enabled) {
                displays[0].enabled = true;
            }
            for display in displays.iter().filter(|display| display.enabled) {
                display.validate()?;
                if !self.displays.as_array().is_some_and(|inventory| {
                    inventory.iter().any(|item| item["id"] == display.display)
                }) {
                    bail!("통합 프리셋 디스플레이가 연결되어 있지 않습니다.");
                }
            }
        }
        let mut plan = None;
        let mut window_request = None;
        if let Some(target) = &preset.windows {
            if self.session.is_none() {
                self.execute(Command::WindowsConnect {})?;
            }
            let config = &self.wm["configuration"];
            let roots = config["views"][&target.view]["roots"]
                .as_object()
                .context("저장된 창 배치를 찾을 수 없습니다.")?;
            if config["slots"][&target.slot].is_null() {
                bail!("저장된 화면 영역을 찾을 수 없습니다.");
            }
            if start {
                let mapped: BTreeMap<&String, &String> =
                    roots.keys().map(|root| (root, &target.slot)).collect();
                let request = new_id("bundle");
                let command = json!({"kind":"preview","request":{"id":request,"expected_revision":config["revision"],"scope":[target.slot],"targets":[{"view":target.view,"roots":mapped}],"mode":"open","retain":[],"selected_tabs":{},"focus":null}});
                window_request = Some(request);
                let response = self.execute(Command::WindowsSession { command })?;
                plan = Some(
                    response["result"]["plan"]
                        .as_str()
                        .context("창 배치 미리보기 토큰이 없습니다.")?
                        .to_owned(),
                );
            }
        }
        let mut applied = vec![];
        let changes: Result<()> = (|| {
            if let Some(audio) = preset.audio {
                if !start
                    || self.draft.audio.source != audio.source
                    || self.draft.audio.target != audio.target
                {
                    self.stop_audio()?;
                }
                self.draft.audio = audio;
                if start && !self.audio.as_ref().is_some_and(OwnedProcess::alive) {
                    self.execute(Command::AudioStart {})?;
                }
                applied.push("Audio Repeater");
            }
            if let Some(cable) = preset.cable {
                let same = self.cable.cable.as_ref().map(|c| (&c.playback_id, &c.capture_id))
                    == cable.cable.as_ref().map(|c| (&c.playback_id, &c.capture_id))
                    && self.cable.output_id == cable.output_id;
                if !start || !same {
                    self.stop_monitor()?;
                }
                self.cable.cable = cable.cable;
                self.cable.output_id = cable.output_id;
                self.cable.gain_db = cable.gain_db;
                self.cable.muted = cable.muted;
                if start {
                    if let Some(worker) = self.monitor.as_mut().filter(|worker| worker.alive()) {
                        worker.write(
                            &json!({"gain_db":self.cable.gain_db,"muted":self.cable.muted}),
                        )?;
                    } else {
                        self.execute(Command::CableStart {})?;
                    }
                }
                applied.push("Audio Cable");
            }
            if let Some(displays) = preset.displays {
                let current: Vec<_> = self.draft.displays().cloned().collect();
                if !start || current != displays {
                    for worker in self.mirrors.values_mut() {
                        worker.stop()?;
                    }
                    self.mirrors.clear();
                }
                self.draft.display = displays[0].clone();
                self.draft.additional_displays = displays[1..].to_vec();
                if start {
                    for (index, display) in displays.iter().enumerate() {
                        if display.enabled
                            && !self.mirrors.get(&index).is_some_and(OwnedProcess::alive)
                        {
                            self.execute(Command::MirrorStart { index })?;
                        }
                    }
                }
                applied.push("Display Relay");
            }
            if let Some(target) = preset.windows {
                self.wm["bundleTarget"] = serde_json::to_value(target)?;
                if let Some(plan) = &plan {
                    let report = self.execute(Command::WindowsSession {
                        command: json!({"kind":"apply","plan":plan}),
                    })?;
                    self.wm["bundleTarget"]["request"] = json!(window_request);
                    if report["result"]["status"] != "settled" {
                        bail!("창 배치가 완전히 적용되지 않았습니다: {}", report["result"]);
                    }
                }
                applied.push("Window Manager");
            }
            Ok(())
        })();
        if let Err(error) = changes {
            if let Some(plan) = plan {
                let _ = self.wm_request(&json!({"kind":"discard_preview","plan":plan}));
            }
            bail!("통합 프리셋 적용 실패. 완료한 기능: {}. {error:#}", applied.join(", "));
        }
        Ok(json!({"name":name,"applied":applied,"started":start}))
    }
}
