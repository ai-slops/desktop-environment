//! Windows-first audio routing helpers for duplicating one output device into another.

#[cfg(target_os = "windows")]
mod windows_impl;

#[cfg(target_os = "windows")]
pub use windows_impl::{AudioOutputDevice, list_output_devices, run_output_audio_router};

#[cfg(not(target_os = "windows"))]
mod unsupported {
    use anyhow::{Result, bail};

    #[derive(Debug, Clone)]
    pub struct AudioOutputDevice {
        pub id: String,
        pub friendly_name: String,
        pub is_default: bool,
    }

    pub fn list_output_devices() -> Result<Vec<AudioOutputDevice>> {
        bail!("Audio routing is only supported on Windows")
    }

    pub fn run_output_audio_router(_: &str, _: &str) -> Result<()> {
        bail!("Audio routing is only supported on Windows")
    }
}

#[cfg(not(target_os = "windows"))]
pub use unsupported::{AudioOutputDevice, list_output_devices, run_output_audio_router};
