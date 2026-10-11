//! Provider-independent cable identities, audio controls and backend contract.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
};

pub const MIN_GAIN_DB: f32 = -96.0;
static NEXT_SAVE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointFlow {
    Playback,
    Capture,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub id: String,
    pub name: String,
    pub flow: EndpointFlow,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cable {
    /// Stable provider identifier, not a displayed device name.
    pub provider: String,
    pub name: String,
    pub playback_id: String,
    pub capture_id: String,
}

impl Cable {
    pub fn validate(&self, endpoints: &[Endpoint]) -> Result<()> {
        for (id, flow) in
            [(&self.playback_id, EndpointFlow::Playback), (&self.capture_id, EndpointFlow::Capture)]
        {
            if !endpoints.iter().any(|e| &e.id == id && e.flow == flow) {
                bail!("케이블의 재생 또는 녹음 장치가 없습니다. 장치를 다시 선택하세요.");
            }
        }
        Ok(())
    }
}

/// Enumerates native endpoints and owns the lifetime of one monitor route.
///
/// Creation/installation belongs to a separate cable provider:
/// installing a driver must never be an incidental effect of enumeration/run.
pub trait AudioBackend: Send + Sync {
    fn endpoints(&self) -> Result<Vec<Endpoint>>;
    fn installed_cables(&self, endpoints: &[Endpoint]) -> Vec<Cable>;
    fn monitor(&self, cable: &Cable, output_id: &str, control: Arc<RouteControl>) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub cable: Option<Cable>,
    pub output_id: String,
    pub gain_db: f32,
    pub muted: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self { version: 1, cable: None, output_id: String::new(), gain_db: -30.0, muted: false }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("지원하지 않는 설정 버전입니다: {}", self.version);
        }
        validate_gain(self.gain_db)
    }
    pub fn validate_route(&self, endpoints: &[Endpoint]) -> Result<()> {
        self.validate()?;
        let cable = self.cable.as_ref().context("가상 케이블을 선택하세요.")?;
        cable.validate(endpoints)?;
        if cable.playback_id == self.output_id {
            bail!("케이블로 다시 출력할 수 없습니다. 들을 장치를 선택하세요.");
        }
        if !endpoints.iter().any(|e| e.flow == EndpointFlow::Playback && e.id == self.output_id) {
            bail!("들을 출력 장치를 선택하세요.");
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let settings: Self =
                    serde_json::from_slice(&bytes).context("설정을 읽지 못했습니다")?;
                settings.validate()?;
                Ok(settings)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent =
            path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(
            ".audio-cable-{}-{}.tmp",
            std::process::id(),
            NEXT_SAVE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        let result = (|| -> Result<()> {
            serde_json::to_writer_pretty(&mut file, self)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

pub fn validate_gain(db: f32) -> Result<()> {
    if !db.is_finite() || !(MIN_GAIN_DB..=0.0).contains(&db) {
        bail!("감쇠는 {MIN_GAIN_DB}~0 dB의 유한한 값이어야 합니다.");
    }
    Ok(())
}
#[must_use]
pub fn linear_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RouteState {
    Starting = 0,
    Running = 1,
    Recovering = 2,
    Stopped = 3,
}

pub struct RouteControl {
    gain: AtomicU32,
    mute: AtomicBool,
    stopped: AtomicBool,
    state: AtomicU8,
}
impl RouteControl {
    pub fn new(db: f32, mute: bool) -> Result<Self> {
        validate_gain(db)?;
        Ok(Self {
            gain: AtomicU32::new(db.to_bits()),
            mute: AtomicBool::new(mute),
            stopped: AtomicBool::new(false),
            state: AtomicU8::new(RouteState::Starting as u8),
        })
    }
    pub fn set_gain(&self, db: f32) -> Result<()> {
        validate_gain(db)?;
        self.gain.store(db.to_bits(), Ordering::Relaxed);
        Ok(())
    }
    pub fn set_mute(&self, mute: bool) {
        self.mute.store(mute, Ordering::Relaxed);
    }
    pub fn target_gain(&self) -> f32 {
        if self.mute.load(Ordering::Relaxed) {
            0.0
        } else {
            linear_gain(f32::from_bits(self.gain.load(Ordering::Relaxed)))
        }
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }
    pub fn set_state(&self, state: RouteState) {
        self.state.store(state as u8, Ordering::Relaxed);
    }
    pub fn state(&self) -> RouteState {
        match self.state.load(Ordering::Relaxed) {
            0 => RouteState::Starting,
            1 => RouteState::Running,
            2 => RouteState::Recovering,
            _ => RouteState::Stopped,
        }
    }
}

/// Linear 20 ms gain ramps, per frame (all channels receive the same gain).
///
/// Startup begins at the selected attenuation, never at full volume. f32 input
/// preserves low levels until the target audio engine's final conversion.
pub struct GainRamp {
    current: f32,
    target: f32,
    step: f32,
    remaining: u16,
    ramp_frames: u16,
}
impl GainRamp {
    #[must_use]
    pub fn new(rate: u32, initial: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            step: 0.0,
            remaining: 0,
            ramp_frames: u16::try_from((rate / 50).max(1)).unwrap_or(u16::MAX),
        }
    }
    pub fn process_f32_bytes(&mut self, bytes: &mut [u8], channels: usize, target: f32) {
        if channels == 0 {
            return;
        }
        if target.to_bits() != self.target.to_bits() {
            self.target = target;
            self.remaining = self.ramp_frames;
            self.step = (target - self.current) / f32::from(self.ramp_frames);
        }
        for frame in bytes.chunks_exact_mut(channels * 4) {
            if self.remaining > 0 {
                self.current += self.step;
                self.remaining -= 1;
                if self.remaining == 0 {
                    self.current = self.target;
                }
            }
            for sample in frame.chunks_exact_mut(4) {
                let value = f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]);
                // A muted stream must contain exact zero even for invalid samples.
                let value = if value.is_finite() && self.current != 0.0 {
                    value * self.current
                } else {
                    0.0
                };
                sample.copy_from_slice(&value.to_le_bytes());
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;
    fn process(ramp: &mut GainRamp, frames: usize, channels: usize, gain: f32) -> Vec<f32> {
        let mut bytes: Vec<_> =
            std::iter::repeat_n(0.5_f32.to_le_bytes(), frames * channels).flatten().collect();
        ramp.process_f32_bytes(&mut bytes, channels, gain);
        bytes.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect()
    }
    #[test]
    fn sub_minimum_levels_preserve_stereo_samples_and_start_quietly() {
        for db in [-20.0, -40.0, -60.0, -96.0] {
            let gain = linear_gain(db);
            let samples = process(&mut GainRamp::new(48_000, gain), 10, 2, gain);
            assert!(samples.iter().all(|s| 0.5_f32.mul_add(-gain, *s).abs() < 1e-7));
            assert!(samples.iter().all(|s| *s > 0.0));
        }
    }
    #[test]
    fn ramp_is_packet_independent_and_mute_reaches_exact_zero() {
        let mut whole = GainRamp::new(48_000, 0.1);
        let mut chunks = GainRamp::new(48_000, 0.1);
        let expected = process(&mut whole, 1200, 2, 0.0);
        let mut actual = process(&mut chunks, 137, 2, 0.0);
        actual.extend(process(&mut chunks, 1063, 2, 0.0));
        assert_eq!(actual, expected);
        assert_eq!(actual[960 * 2], 0.0);
        assert!(actual.windows(2).all(|w| w[0] >= w[1]));
    }
    #[test]
    fn invalid_gains_and_unknown_versions_are_rejected() {
        for db in [f32::NAN, f32::INFINITY, -97.0, 0.1] {
            assert!(RouteControl::new(db, false).is_err());
        }
        assert!(Settings { version: 2, ..Settings::default() }.validate().is_err());
    }
    #[test]
    fn settings_keep_provider_neutral_device_pairs_and_never_autostart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let settings = Settings {
            cable: Some(Cable {
                provider: "custom-driver".into(),
                name: "My cable".into(),
                playback_id: "render-id".into(),
                capture_id: "capture-id".into(),
            }),
            gain_db: -63.0,
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.cable, settings.cable);
        assert_eq!(loaded.gain_db, -63.0);
        assert!(loaded.validate_route(&[]).is_err());
        let invalid = Settings { gain_db: f32::NAN, ..loaded };
        assert!(invalid.save(&path).is_err());
        assert_eq!(Settings::load(&path).unwrap().gain_db, -63.0);
    }
}
