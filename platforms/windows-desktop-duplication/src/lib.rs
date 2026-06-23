//! Safe-ish wrappers around the Windows Desktop Duplication API.

#[cfg(target_os = "windows")]
mod windows_impl;

#[cfg(target_os = "windows")]
pub use windows_impl::{CaptureFrameView, DesktopDuplicator, DisplayInfo, enumerate_displays};

#[cfg(not(target_os = "windows"))]
mod unsupported {
    use anyhow::{Result, bail};
    use display_relay_core::{DisplayArea, VirtualDesktop};

    #[derive(Debug, Clone)]
    pub struct DisplayInfo {
        pub name: String,
        pub friendly_name: String,
        pub area: DisplayArea,
        pub virtual_desktop: VirtualDesktop,
    }

    #[derive(Debug, Clone)]
    pub struct CaptureFrameView<'a> {
        pub width: u32,
        pub height: u32,
        pub pixels_bgra: &'a [u8],
    }

    pub fn enumerate_displays() -> Result<Vec<DisplayInfo>> {
        bail!("Desktop Duplication is only supported on Windows")
    }

    pub struct DesktopDuplicator;

    impl DesktopDuplicator {
        pub fn new(_: &str) -> Result<Self> {
            bail!("Desktop Duplication is only supported on Windows")
        }

        pub fn display_info(&self) -> &DisplayInfo {
            unreachable!()
        }

        pub fn capture_frame<'a>(&'a mut self, _: u32) -> Result<CaptureFrameView<'a>> {
            bail!("Desktop Duplication is only supported on Windows")
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub use unsupported::{CaptureFrameView, DesktopDuplicator, DisplayInfo, enumerate_displays};
