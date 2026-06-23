//! Input injection helpers for driving a hidden or capture-card-only monitor.

#[cfg(target_os = "windows")]
mod windows_impl;

#[cfg(target_os = "windows")]
pub use windows_impl::{InjectedKeyEvent, MouseButton, RemoteInputController};

#[cfg(not(target_os = "windows"))]
mod unsupported {
    use anyhow::{Result, bail};
    use display_relay_core::VirtualDesktop;

    #[derive(Debug, Clone, Copy)]
    pub enum MouseButton {
        Left,
        Right,
        Middle,
    }

    #[derive(Debug, Clone, Copy)]
    pub struct InjectedKeyEvent {
        pub scan_code: u16,
        pub pressed: bool,
    }

    pub struct RemoteInputController;

    impl RemoteInputController {
        pub fn new(_: VirtualDesktop) -> Self {
            Self
        }

        pub fn move_mouse(&self, _: i32, _: i32) -> Result<()> {
            bail!("Input injection is only supported on Windows")
        }

        pub fn click(&self, _: MouseButton, _: bool) -> Result<()> {
            bail!("Input injection is only supported on Windows")
        }

        pub fn send_key(&self, _: InjectedKeyEvent) -> Result<()> {
            bail!("Input injection is only supported on Windows")
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub use unsupported::{InjectedKeyEvent, MouseButton, RemoteInputController};
