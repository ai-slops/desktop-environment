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

/// Journal writes precede hiding. A failed write prevents the native hide.
impl Journal {
    pub fn load(path: &std::path::Path) -> window_manager_core::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() <= 4 * 1024 * 1024 => {
                let journal: Self = serde_json::from_slice(&bytes).map_err(|error| {
                    window_manager_core::Error::new(
                        window_manager_core::ErrorCode::StorageFailure,
                        error.to_string(),
                        "journal",
                    )
                })?;
                if journal.version != 1 {
                    return Err(window_manager_core::Error::new(
                        window_manager_core::ErrorCode::InvalidConfiguration,
                        "Unsupported journal version",
                        "journal",
                    ));
                }
                Ok(journal)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self { version: 1, entries: Vec::new() })
            }
            _ => Err(window_manager_core::Error::new(
                window_manager_core::ErrorCode::StorageFailure,
                "Cannot read recovery journal",
                "journal",
            )),
        }
    }

    pub fn save(&self, path: &std::path::Path) -> window_manager_core::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| {
            window_manager_core::Error::new(
                window_manager_core::ErrorCode::StorageFailure,
                error.to_string(),
                "journal",
            )
        })?;
        window_manager_core::atomic_write(path, &bytes)
    }
}

#[must_use]
pub fn binding_key(binding: &Binding) -> String {
    format!("{}:{}:{}", binding.process, binding.process_started, binding.generation)
}
