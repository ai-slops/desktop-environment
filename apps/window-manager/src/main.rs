//! Windows-first multi-view window manager with a separate recovery invocation.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#![allow(clippy::multiple_crate_versions)]
#![cfg_attr(test, allow(unused_crate_dependencies))] // Integration tests share this binary's dev dependencies.

mod cli;
mod reflow;
mod replay;
mod service;
mod session;
mod ui;

use anyhow::{Context, Result};
use std::path::PathBuf;

fn default_path() -> Result<PathBuf> {
    let root =
        std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable; use --config")?;
    Ok(PathBuf::from(root).join("DesktopEnvironment").join("window-manager.json"))
}

fn main() -> std::process::ExitCode {
    let arguments = cli::parse(std::env::args().skip(1));
    let gui = arguments.as_ref().is_ok_and(|args| matches!(args.action, cli::Action::Gui));
    #[cfg(windows)]
    if !gui {
        attach_cli_console();
    }
    let result = arguments.and_then(|mut args| {
        let path = std::path::absolute(args.path.take().map_or_else(default_path, Ok)?)?;
        cli::execute(args, path)
    });
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            #[cfg(windows)]
            if gui {
                show_startup_error(&error);
            }
            #[cfg(not(windows))]
            let _ = gui;
            eprintln!("{error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(windows)]
fn attach_cli_console() {
    use windows::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE, SetStdHandle,
    };
    // Preserve pipes used by JSON sessions and redirected CLI output when attaching to
    // an existing terminal. Never allocate a new console, including in the watchdog.
    unsafe {
        let handles = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
            .map(|kind| (kind, GetStdHandle(kind).ok().filter(|handle| !handle.is_invalid())));
        if AttachConsole(ATTACH_PARENT_PROCESS).is_ok() {
            for (kind, handle) in handles {
                if let Some(handle) = handle {
                    let _ = SetStdHandle(kind, handle);
                }
            }
        }
    }
}

#[cfg(windows)]
fn show_startup_error(error: &anyhow::Error) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::{PCWSTR, w};
    let message: Vec<u16> = format!("{error:#}").encode_utf16().chain(Some(0)).collect();
    // The message remains valid throughout the synchronous dialog call.
    unsafe {
        MessageBoxW(None, PCWSTR(message.as_ptr()), w!("Window Manager"), MB_OK | MB_ICONERROR);
    }
}
