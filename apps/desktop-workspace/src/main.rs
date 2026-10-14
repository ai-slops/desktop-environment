//! Stable installed launcher. Build outputs are never used as live executables.
#![cfg_attr(windows, windows_subsystem = "windows")]
#![allow(clippy::multiple_crate_versions)]
use anyhow::{Context, Result, bail};

fn launch() -> Result<()> {
    let executable = std::env::current_exe()?;
    let root = executable.parent().context("Install directory missing")?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("current.json"))?)?;
    let version = manifest["version"].as_str().context("Missing installed version")?;
    if version.is_empty()
        || !version.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        bail!("Invalid installed version");
    }
    let binary = root.join("versions").join(version).join("DesktopWorkspaceUI.exe");
    if !binary.is_file() {
        bail!("Installed UI is missing. Run just desktop-workspace-install.");
    }
    let mut command = std::process::Command::new(binary);
    command.args(std::env::args_os().skip(1));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command.spawn().context("Unable to start installed Desktop Workspace")?;
    Ok(())
}
fn main() {
    if let Err(error) = launch() {
        #[cfg(windows)]
        {
            use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
            let message: Vec<u16> =
                format!("{error:#}").encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: message is NUL-terminated and lives through the synchronous dialog.
            let _ = unsafe {
                MessageBoxW(
                    None,
                    windows::core::PCWSTR(message.as_ptr()),
                    windows::core::w!("Desktop Workspace"),
                    MB_OK | MB_ICONERROR,
                )
            };
        }
        #[cfg(not(windows))]
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
