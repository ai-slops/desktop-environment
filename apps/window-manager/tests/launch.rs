#![cfg(windows)]
#![allow(unused_crate_dependencies)]
#![allow(clippy::multiple_crate_versions)]

use std::process::{Command, Stdio};

#[test]
fn executable_uses_gui_subsystem_without_an_automatic_console()
-> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(env!("CARGO_BIN_EXE_window-manager"))?;
    let offset = u32::from_le_bytes(bytes[0x3c..0x40].try_into()?) as usize;
    assert_eq!(&bytes[offset..offset + 4], b"PE\0\0");
    let subsystem = offset + 4 + 20 + 68;
    assert_eq!(u16::from_le_bytes(bytes[subsystem..subsystem + 2].try_into()?), 2);
    Ok(())
}

#[test]
fn cli_help_keeps_redirected_output() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_window-manager"))
        .arg("--help")
        .stdin(Stdio::null())
        .output()?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("--config"));
    Ok(())
}

#[test]
fn cli_errors_keep_redirected_diagnostics_and_failure_status()
-> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_window-manager"))
        .arg("--invalid-launch-option")
        .stdin(Stdio::null())
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unknown argument"));
    Ok(())
}
