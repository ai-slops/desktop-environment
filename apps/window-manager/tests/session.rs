#![cfg(windows)]
#![allow(unused_crate_dependencies)] // Integration tests share the application's dependencies.
#![allow(clippy::multiple_crate_versions)]
use std::io::Write;
use std::process::{Command, Stdio};
use window_manager_core::Configuration;

#[test]
fn inherited_pipe_session_is_read_only_strict_and_privacy_preserving()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("config.json");
    Configuration::default().save(&path)?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_window-manager"))
        .args(["--session", "--config"])
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("stdin missing")?;
    input.write_all(concat!(
        "{\"id\":\"read\",\"command\":{\"kind\":\"snapshot\"}}\n",
        "{\"id\":\"control\",\"command\":{\"kind\":\"recover\"}}\n",
        "{\"id\":\"provider\",\"command\":{\"kind\":\"provider_disconnect\",\"provider\":\"p\"}}\n",
        "{\"id\":\"widen\",\"command\":{\"kind\":\"snapshot\",\"allow_control\":true}}\n"
    ).as_bytes())?;
    drop(input);
    let output = child.wait_with_output()?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let output = String::from_utf8(output.stdout)?;
    let replies: Vec<serde_json::Value> =
        output.lines().map(serde_json::from_str).collect::<std::result::Result<_, _>>()?;
    assert_eq!(replies.len(), 4);
    assert_eq!(replies[0]["result"]["revision"], 0);
    assert_eq!(replies[1]["error"]["code"], "PERMISSION_DENIED");
    assert_eq!(replies[2]["error"]["code"], "PERMISSION_DENIED");
    assert_eq!(replies[3]["error"]["code"], "INVALID_CONFIGURATION");
    for forbidden in ["handle", "binding", "token_property", "process_started", "title"] {
        assert!(!output.contains(forbidden));
    }
    assert!(!path.with_extension("recovery.json").exists());
    Ok(())
}

#[test]
fn oversized_command_ends_the_session_without_applying_control()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("config.json");
    Configuration::default().save(&path)?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_window-manager"))
        .args(["--session", "--config"])
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().ok_or("stdin missing")?.write_all(&vec![b'x'; 65_537])?;
    let output = child.wait_with_output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("64 KiB"));
    assert!(output.stdout.is_empty());
    Ok(())
}
