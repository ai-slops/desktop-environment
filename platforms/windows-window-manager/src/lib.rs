//! Ordinary-user Win32 discovery, lifetime-validated execution, and independent reveal recovery.
#![allow(clippy::multiple_crate_versions)]

use serde::{Deserialize, Serialize};
use window_manager_core::{Binding, Id, ObservedWindow, Rect};
mod monitors;
pub use monitors::*;

#[derive(Clone, Debug)]
pub struct Candidate {
    pub handle: u64,
    pub title: String,
    pub class: String,
    pub process: u32,
    pub frame: Rect,
}
pub struct SubmissionReport {
    pub results: std::collections::BTreeMap<Id, window_manager_core::Result<()>>,
    pub batched_windows: usize,
    pub individual_windows: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryEntry {
    pub window: Id,
    pub prior: ObservedWindow,
    pub request: Id,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub version: u32,
    pub entries: Vec<RecoveryEntry>,
}

#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::*;

#[cfg(not(windows))]
mod unsupported;
#[cfg(not(windows))]
pub use unsupported::*;

/// Owns the observation thread and its registrations. Dropping it releases native resources.
pub struct EventStream {
    receiver: std::sync::mpsc::Receiver<NativeEvent>,
    stop: Option<Box<dyn FnOnce() + Send>>,
}
impl std::ops::Deref for EventStream {
    type Target = std::sync::mpsc::Receiver<NativeEvent>;
    fn deref(&self) -> &Self::Target {
        &self.receiver
    }
}
impl Drop for EventStream {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

/// Journal writes precede hiding. A failed write prevents the native hide.
impl Journal {
    pub fn load(path: &std::path::Path) -> window_manager_core::Result<Self> {
        match std::fs::metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self { version: 1, entries: Vec::new() });
            }
            Err(error) => {
                return Err(window_manager_core::Error::new(
                    window_manager_core::ErrorCode::StorageFailure,
                    error.to_string(),
                    "journal",
                ));
            }
            Ok(_) => {}
        }
        let journal: Self = window_manager_core::read_json(path)?;
        if journal.version != 1 {
            return Err(window_manager_core::Error::new(
                window_manager_core::ErrorCode::InvalidConfiguration,
                "Unsupported journal version",
                "journal",
            ));
        }
        Ok(journal)
    }

    pub fn save(&self, path: &std::path::Path) -> window_manager_core::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| {
            window_manager_core::Error::new(
                window_manager_core::ErrorCode::StorageFailure,
                error.to_string(),
                "journal",
            )
        })?;
        if bytes.len() as u64 > window_manager_core::MAX_CONFIGURATION_BYTES {
            return Err(window_manager_core::Error::new(
                window_manager_core::ErrorCode::InvalidConfiguration,
                "Recovery journal size budget exceeded",
                "journal",
            ));
        }
        window_manager_core::atomic_write(path, &bytes)
    }
}

#[must_use]
pub fn binding_key(binding: &Binding) -> String {
    format!("{}:{}:{}", binding.process, binding.process_started, binding.generation)
}
