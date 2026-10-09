//! Capture and restore owned windows without mixing workspace and screen coordinates.
// The persistence library and Windows bindings depend on distinct windows-link versions.
#![allow(clippy::multiple_crate_versions)]

#[cfg(target_os = "windows")]
mod windows_impl;

#[cfg(target_os = "windows")]
pub use windows_impl::{capture, capture_physical_client, restore, restore_physical_client};

#[cfg(not(target_os = "windows"))]
pub fn capture(
    _: raw_window_handle::WindowHandle<'_>,
) -> anyhow::Result<Option<desktop_presets::WindowPlacement>> {
    anyhow::bail!("창 배치 저장은 Windows에서만 지원합니다.");
}

#[cfg(not(target_os = "windows"))]
pub fn restore(
    _: raw_window_handle::WindowHandle<'_>,
    _: desktop_presets::WindowPlacement,
) -> anyhow::Result<()> {
    anyhow::bail!("창 배치 복원은 Windows에서만 지원합니다.");
}

#[cfg(not(target_os = "windows"))]
pub fn capture_physical_client(
    window: raw_window_handle::WindowHandle<'_>,
) -> anyhow::Result<Option<desktop_presets::WindowPlacement>> {
    capture(window)
}

#[cfg(not(target_os = "windows"))]
pub fn restore_physical_client(
    window: raw_window_handle::WindowHandle<'_>,
    placement: desktop_presets::WindowPlacement,
) -> anyhow::Result<()> {
    restore(window, placement)
}
