//! Windows desktop shortcuts for the settings control panel.

// windows and tempfile require different windows-link versions.
#![allow(clippy::multiple_crate_versions)]

#[cfg(target_os = "windows")]
mod windows_impl;

#[cfg(target_os = "windows")]
pub use windows_impl::create_desktop_shortcut;

#[cfg(not(target_os = "windows"))]
pub fn create_desktop_shortcut(
    _: &std::path::Path,
    _: &std::path::Path,
) -> anyhow::Result<std::path::PathBuf> {
    anyhow::bail!("바탕화면 바로가기는 Windows에서만 만들 수 있습니다.");
}
