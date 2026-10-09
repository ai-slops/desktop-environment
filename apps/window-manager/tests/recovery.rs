#![cfg(windows)]
#![allow(clippy::multiple_crate_versions)]
#![allow(unused_crate_dependencies)] // GUI-only dependencies are supplied to every integration test.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use window_manager_core::{Mutation, Rect};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW,
    SW_SHOWNOACTIVATE, ShowWindow, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
};
use windows::core::w;
use windows_window_manager::{
    Candidate, Journal, RecoveryEntry, bind, observe, process_started, submit,
};

struct Owned(HWND);
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: the test owns this window and destroys it on its creation thread.
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

#[test]
#[ignore = "Spawned only as the watchdog's disposable parent fixture"]
fn helper_parent_fixture() -> std::io::Result<()> {
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(())
}

#[test]
fn separate_helper_recovers_hidden_window_after_parent_process_death()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let config = directory.path().join("config.json");
    let path = config.with_extension("recovery.json");
    // SAFETY: built-in STATIC class; no application-specific pointer or callback.
    let window = Owned(unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("Independent recovery fixture"),
            WS_OVERLAPPEDWINDOW,
            100,
            100,
            400,
            300,
            None,
            None,
            None,
            None,
        )
    }?);
    let _ = unsafe { ShowWindow(window.0, SW_SHOWNOACTIVATE) };
    let before = bind(
        &Candidate {
            handle: window.0.0 as usize as u64,
            title: "fixture".into(),
            class: "STATIC".into(),
            process: std::process::id(),
            frame: Rect::default(),
        },
        true,
    )?;
    Journal {
        version: 1,
        entries: vec![RecoveryEntry {
            window: "fixture".into(),
            prior: before.clone(),
            request: "test-hide".into(),
        }],
    }
    .save(&path)?;
    let mut parent = Command::new(std::env::current_exe()?)
        .args(["--ignored", "--exact", "helper_parent_fixture"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()?;
    let started = process_started(parent.id())?;
    let mut helper = Command::new(env!("CARGO_BIN_EXE_window-manager"))
        .arg("--watch-parent")
        .arg(parent.id().to_string())
        .arg(started.to_string())
        .arg("--config")
        .arg(&config)
        .stdout(Stdio::null())
        .spawn()?;
    submit(&Mutation {
        window: "fixture".into(),
        binding: before.binding.clone(),
        geometry: None,
        move_only: true,
        visible: Some(false),
        focus: false,
        show_state: None,
    })?;
    assert!(!observe(&before.binding, true)?.visible);
    parent.kill()?;
    parent.wait()?;
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut message = MSG::default();
    loop {
        // SAFETY: pump the fixture's queue so cross-thread asynchronous requests can settle.
        while unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            unsafe { DispatchMessageW(&raw const message) };
        }
        if observe(&before.binding, true)?.visible {
            break;
        }
        if Instant::now() >= deadline {
            let _ = helper.kill();
            return Err("Independent recovery helper timed out".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    helper.wait()?;
    assert_eq!(observe(&before.binding, true)?.frame, before.frame);
    assert!(Journal::load(&path)?.entries.is_empty());
    Ok(())
}
