use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Normal (unmaximized) outer bounds in Windows placement/workspace coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub normal_rect: [i32; 4],
    pub maximized: bool,
    /// Optional pixel size of the normal client area, independent of DPI-scaled borders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_client_size: Option<[u32; 2]>,
}

impl WindowPlacement {
    pub fn validate(self) -> Result<()> {
        let [left, top, right, bottom] = self.normal_rect;
        let width = i64::from(right) - i64::from(left);
        let height = i64::from(bottom) - i64::from(top);
        if !(1..=65536).contains(&width) || !(1..=65536).contains(&height) {
            bail!("저장된 창 크기가 올바르지 않습니다.");
        }
        if let Some([width, height]) = self.normal_client_size
            && (!(1..=65536).contains(&width) || !(1..=65536).contains(&height))
        {
            bail!("저장된 영상 영역 크기가 올바르지 않습니다.");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WindowStates {
    version: u32,
    windows: BTreeMap<String, WindowPlacement>,
}

/// Geometry has its own file so automatic saves never save unsaved tool settings
/// or contend with a running relay writing to the same presets file.
pub struct WindowStateFile {
    path: PathBuf,
    data: WindowStates,
}

impl WindowStateFile {
    pub fn load(path: PathBuf) -> Result<Self> {
        let data = match std::fs::read(&path) {
            Ok(bytes) => {
                let data: WindowStates = serde_json::from_slice(&bytes)
                    .context("창 배치 파일 형식이 올바르지 않습니다.")?;
                if data.version != 1 {
                    bail!("지원하지 않는 창 배치 파일 버전: {}", data.version);
                }
                for placement in data.windows.values() {
                    placement.validate()?;
                }
                data
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                WindowStates { version: 1, windows: BTreeMap::new() }
            }
            Err(error) => return Err(error).context("창 배치 파일을 읽을 수 없습니다."),
        };
        Ok(Self { path, data })
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<WindowPlacement> {
        self.data.windows.get(key).copied()
    }

    /// Persist only changed placements, replacing the file after a flushed write.
    pub fn remember(&mut self, key: &str, placement: WindowPlacement) -> Result<()> {
        placement.validate()?;
        if self.get(key) == Some(placement) {
            return Ok(());
        }
        let mut updated = self.data.clone();
        updated.windows.insert(key.into(), placement);
        let parent = self.path.parent().context("창 배치 파일 폴더가 없습니다.")?;
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut temporary, &updated)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path).context("창 배치를 저장할 수 없습니다.")?;
        self.data = updated;
        Ok(())
    }
}

#[must_use]
pub fn window_state_path(config: &Path, kind: &str) -> PathBuf {
    let mut filename = config.file_name().unwrap_or_default().to_os_string();
    filename.push(format!(".{kind}-window.json"));
    config.with_file_name(filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLACEMENT: WindowPlacement = WindowPlacement {
        normal_rect: [-1400, 100, -500, 750],
        maximized: false,
        normal_client_size: None,
    };

    #[test]
    fn displays_and_configurations_keep_independent_geometry() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let config = directory.path().join("한글 설정.json");
        let relay_path = window_state_path(&config, "relay");
        let control_path = window_state_path(&config, "control");
        assert_ne!(relay_path, control_path);
        assert_ne!(
            control_path,
            window_state_path(&directory.path().join("other.json"), "control")
        );
        let mut relay = WindowStateFile::load(relay_path.clone())?;
        relay.remember(r"\\.\DISPLAY1", PLACEMENT)?;
        let maximized = WindowPlacement { maximized: true, ..PLACEMENT };
        relay.remember(r"\\.\DISPLAY2", maximized)?;
        let loaded = WindowStateFile::load(relay_path)?;
        assert_eq!(loaded.get(r"\\.\DISPLAY1"), Some(PLACEMENT));
        assert_eq!(loaded.get(r"\\.\DISPLAY2"), Some(maximized));
        assert_eq!(WindowStateFile::load(control_path)?.get("control"), None);
        assert!(!config.exists());
        Ok(())
    }

    #[test]
    fn bad_geometry_file_is_preserved_and_failed_save_keeps_memory() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("geometry.json");
        for contents in [
            "broken",
            r#"{"version":7,"windows":{}}"#,
            r#"{"version":1,"windows":{"control":{"normal_rect":[0,0,0,0],"maximized":false}}}"#,
        ] {
            std::fs::write(&path, contents)?;
            assert!(WindowStateFile::load(path.clone()).is_err());
            assert_eq!(std::fs::read_to_string(&path)?, contents);
        }
        let blocked_path = directory.path().join("blocked");
        let mut state = WindowStateFile::load(blocked_path.clone())?;
        std::fs::create_dir(blocked_path)?;
        assert!(state.remember("control", PLACEMENT).is_err());
        assert_eq!(state.get("control"), None);
        Ok(())
    }

    #[test]
    fn unchanged_placement_does_not_write_and_invalid_bounds_are_rejected() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("geometry.json");
        let mut state = WindowStateFile::load(path.clone())?;
        state.remember("control", PLACEMENT)?;
        // If remember wrote again, this deliberately obstructed destination would fail.
        std::fs::remove_file(&path)?;
        std::fs::create_dir(&path)?;
        state.remember("control", PLACEMENT)?;
        for rect in [[0, 0, 0, 10], [0, 0, 20, -1], [i32::MIN, 0, i32::MAX, 10]] {
            assert!(
                WindowPlacement { normal_rect: rect, maximized: false, normal_client_size: None }
                    .validate()
                    .is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn physical_client_size_is_optional_for_legacy_files_and_validated() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("geometry.json");
        std::fs::write(
            &path,
            r#"{"version":1,"windows":{"relay":{"normal_rect":[0,0,1000,600],"maximized":false}}}"#,
        )?;
        let mut state = WindowStateFile::load(path.clone())?;
        let legacy =
            state.get("relay").ok_or_else(|| anyhow::anyhow!("Missing legacy placement"))?;
        assert_eq!(legacy.normal_client_size, None);
        let physical = WindowPlacement { normal_client_size: Some([960, 540]), ..legacy };
        state.remember("relay", physical)?;
        assert_eq!(WindowStateFile::load(path)?.get("relay"), Some(physical));
        for size in [[0, 540], [960, 0], [65537, 10]] {
            assert!(
                WindowPlacement { normal_client_size: Some(size), ..physical }.validate().is_err()
            );
        }
        Ok(())
    }
}
