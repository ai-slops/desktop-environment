//! Persistent, named configurations shared by the desktop tools' control panel.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub enabled: bool,
    pub source: String,
    pub source_label: String,
    pub target: String,
    pub target_label: String,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            source: "default".into(),
            source_label: "Windows 기본 출력".into(),
            target: String::new(),
            target_label: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplaySettings {
    pub enabled: bool,
    pub display: String,
    pub display_label: String,
    pub fullscreen: bool,
    pub fps: u32,
    pub timeout_ms: u32,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            display: String::new(),
            display_label: String::new(),
            fullscreen: false,
            fps: 60,
            timeout_ms: 16,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub audio: AudioSettings,
    pub display: DisplaySettings,
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !self.audio.enabled && !self.display.enabled {
            bail!("오디오 또는 디스플레이 중 하나 이상을 선택하세요.");
        }
        if self.audio.enabled {
            if self.audio.source.trim().is_empty() || self.audio.target.trim().is_empty() {
                bail!("오디오 원본과 대상 출력을 선택하세요.");
            }
            if self.audio.source == self.audio.target {
                bail!("오디오 원본과 대상은 서로 달라야 합니다.");
            }
        }
        if self.display.enabled {
            if self.display.display.trim().is_empty() {
                bail!("미러링할 디스플레이를 선택하세요.");
            }
            if !(1..=240).contains(&self.display.fps) {
                bail!("FPS는 1~240 사이여야 합니다.");
            }
            if !(1..=1000).contains(&self.display.timeout_ms) {
                bail!("캡처 대기 시간은 1~1000ms 사이여야 합니다.");
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn audio_args(&self) -> Vec<String> {
        vec!["route".into(), self.audio.source.clone(), self.audio.target.clone()]
    }

    #[must_use]
    pub fn display_args(&self) -> Vec<String> {
        let mut args = vec![
            "mirror".into(),
            self.display.display.clone(),
            "--fps".into(),
            self.display.fps.to_string(),
            "--timeout-ms".into(),
            self.display.timeout_ms.to_string(),
        ];
        if self.display.fullscreen {
            args.push("--fullscreen".into());
        }
        args
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub settings: Settings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetStore {
    pub version: u32,
    pub last_used: Settings,
    pub presets: Vec<Preset>,
}

impl Default for PresetStore {
    fn default() -> Self {
        Self { version: 1, last_used: Settings::default(), presets: Vec::new() }
    }
}

impl PresetStore {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).context("설정 파일을 읽을 수 없습니다."),
        };
        let store: Self =
            serde_json::from_slice(&bytes).context("설정 파일 형식이 올바르지 않습니다.")?;
        if store.version != 1 {
            bail!("지원하지 않는 설정 파일 버전: {}", store.version);
        }
        for (index, preset) in store.presets.iter().enumerate() {
            validate_name(&preset.name)?;
            if store.presets[..index].iter().any(|other| other.name == preset.name) {
                bail!("설정 파일에 중복된 프리셋 이름이 있습니다: {}", preset.name);
            }
        }
        Ok(store)
    }

    /// Replace the file only after a complete, flushed JSON document is ready.
    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().context("설정 파일 폴더가 없습니다.")?;
        std::fs::create_dir_all(parent).context("설정 폴더를 만들 수 없습니다.")?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut temporary, self)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).context("설정 파일을 저장할 수 없습니다.")?;
        Ok(())
    }

    pub fn add(&mut self, name: &str, settings: Settings) -> Result<()> {
        let name = name.trim();
        validate_name(name)?;
        settings.validate()?;
        if self.presets.iter().any(|preset| preset.name == name) {
            bail!("같은 이름의 프리셋이 있습니다. 선택 후 ‘덮어쓰기’를 사용하세요.");
        }
        self.presets.push(Preset { name: name.into(), settings });
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        bail!("프리셋 이름은 1~80자의 한 줄로 입력하세요.");
    }
    Ok(())
}

pub fn default_store_path() -> Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .context("LOCALAPPDATA가 없습니다. --config <파일 경로>를 지정하세요.")?;
    Ok(base.join("DesktopEnvironment").join("presets.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio_settings() -> Settings {
        let mut settings = Settings::default();
        settings.audio.target = "{headphones-id}".into();
        settings.audio.target_label = "헤드폰 🦀".into();
        settings
    }

    #[test]
    fn round_trip_and_replace_preserve_both_tools() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("nested").join("presets.json");
        let mut settings = audio_settings();
        settings.display = DisplaySettings {
            enabled: true,
            display: r"\\.\DISPLAY3".into(),
            display_label: "캡처 카드".into(),
            fullscreen: true,
            fps: 120,
            timeout_ms: 8,
        };
        let mut store = PresetStore::load(&path)?;
        store.add(" 방송 ", settings.clone())?;
        store.last_used = settings;
        store.save(&path)?;
        assert_eq!(PresetStore::load(&path)?, store);
        store.last_used.display.fps = 60;
        store.save(&path)?;
        assert_eq!(PresetStore::load(&path)?, store);
        assert_eq!(store.presets[0].name, "방송");
        Ok(())
    }

    #[test]
    fn bad_files_are_rejected_and_preserved() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("presets.json");
        for bytes in [b"broken JSON".as_slice(), br#"{"version":999}"#] {
            std::fs::write(&path, bytes)?;
            assert!(PresetStore::load(&path).is_err());
            assert_eq!(std::fs::read(&path)?, bytes);
        }
        Ok(())
    }

    #[test]
    fn duplicate_or_invalid_names_do_not_change_presets() -> Result<()> {
        let mut store = PresetStore::default();
        store.add("방송", audio_settings())?;
        for name in ["방송", " 방송 ", " ", "line\nbreak", &"a".repeat(81)] {
            assert!(store.add(name, audio_settings()).is_err());
            assert_eq!(store.presets.len(), 1);
        }
        Ok(())
    }

    #[test]
    fn validate_only_enabled_tools_and_keep_arguments_literal() -> Result<()> {
        let mut settings = audio_settings();
        settings.audio.target = "Output with spaces & symbols".into();
        settings.validate()?;
        assert_eq!(settings.audio_args()[2], "Output with spaces & symbols");
        settings.audio.target = "default".into();
        assert!(settings.validate().is_err());
        settings.audio.enabled = false;
        assert!(settings.validate().is_err());
        settings.display.enabled = true;
        settings.display.display = r"\\.\DISPLAY3".into();
        settings.display.fullscreen = true;
        settings.validate()?;
        assert_eq!(settings.display_args().last().map(String::as_str), Some("--fullscreen"));
        settings.display.fps = 0;
        assert!(settings.validate().is_err());
        settings.display.fps = 60;
        settings.display.timeout_ms = 1001;
        assert!(settings.validate().is_err());
        Ok(())
    }
}
