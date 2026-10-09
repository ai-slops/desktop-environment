#![cfg(windows)]
#![allow(unused_crate_dependencies)] // Integration tests share the application's dependencies.
#![allow(clippy::multiple_crate_versions)]
use std::io::Write;
use std::process::{Command, Stdio};
use window_manager_core::Configuration;

#[test]
fn read_only_session_simulates_without_native_authority_or_configuration_changes()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("config.json");
    let mut config = Configuration::default();
    config.slots.insert(
        "source-slot".into(),
        window_manager_core::DisplaySlot {
            id: "source-slot".into(),
            name: "simulation source".into(),
            display: "absent-real-display".into(),
            region: [0.0, 0.0, 1.0, 1.0],
            designated_public: false,
            fallback_displays: Vec::new(),
        },
    );
    let view = config.views.keys().next().ok_or("view missing")?.clone();
    config.save(&path)?;
    let before = std::fs::read(&path)?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_window-manager"))
        .args(["--session", "--config"])
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let input = window_manager_core::Simulation {
        target: window_manager_core::Target {
            view,
            roots: std::collections::BTreeMap::from([("main".into(), "source-slot".into())]),
        },
        width: 1600,
        height: 900,
        dpi: 96,
        group: None,
        count: Some(4),
        minimum: None,
        fixed_children: false,
        missing: std::collections::BTreeSet::new(),
        display_present: true,
    };
    let mut stdin = child.stdin.take().ok_or("stdin missing")?;
    serde_json::to_writer(
        &mut stdin,
        &serde_json::json!({"id":"simulation","command":{"kind":"simulate","input":input}}),
    )?;
    stdin.write_all(b"\n")?;
    drop(stdin);
    let output = child.wait_with_output()?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert!(reply["result"]["error"].is_null());
    assert_eq!(reply["result"]["desired"].as_object().ok_or("desired missing")?.len(), 4);
    for field in ["binding", "mutations", "token_property"] {
        assert!(!String::from_utf8_lossy(&output.stdout).contains(field));
    }
    assert_eq!(std::fs::read(&path)?, before);
    assert!(!path.with_extension("recovery.json").exists());
    Ok(())
}

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
    input.write_all(br#"{"id":"duplicate","command":{"kind":"snapshot","kind":"recover"}}
{"id":"duplicate-roots","command":{"kind":"recall","target":{"kind":"view","target":{"view":"v","roots":{"main":"a","main":"b"}}}}}
"#)?;
    drop(input);
    let output = child.wait_with_output()?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let output = String::from_utf8(output.stdout)?;
    let replies: Vec<serde_json::Value> =
        output.lines().map(serde_json::from_str).collect::<std::result::Result<_, _>>()?;
    assert_eq!(replies.len(), 6);
    assert_eq!(replies[0]["result"]["revision"], 0);
    assert_eq!(replies[1]["error"]["code"], "PERMISSION_DENIED");
    assert_eq!(replies[2]["error"]["code"], "PERMISSION_DENIED");
    assert_eq!(replies[3]["error"]["code"], "INVALID_CONFIGURATION");
    for reply in &replies[4..] {
        assert_eq!(reply["error"]["code"], "INVALID_CONFIGURATION");
        assert!(reply["id"].is_null());
    }
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

#[test]
fn structural_session_edits_require_revision_and_can_be_undone_without_native_effects()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("config.json");
    let mut config = Configuration::default();
    let view = config.views.values_mut().next().ok_or("View missing")?;
    let window_manager_core::Node::Group(root) =
        view.roots.values_mut().next().ok_or("root missing")?
    else {
        return Err("Group missing".into());
    };
    let source = window_manager_core::Group::new("source".into());
    let destination = window_manager_core::Group::new("destination".into());
    let source_id = source.id.clone();
    let destination_id = destination.id.clone();
    let view_id = view.id.clone();
    root.children.extend([
        window_manager_core::Node::Group(source),
        window_manager_core::Node::Group(destination),
    ]);
    config.save(&path)?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_window-manager"))
        .args(["--session", "--allow-control", "--config"])
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("stdin missing")?;
    let action = serde_json::json!({"kind":"copy","source":{"view":view_id,"node":source_id},"destination":{"view":view_id,"group":destination_id,"index":null}});
    for (id, command) in [
        (
            "stale",
            serde_json::json!({"kind":"structure","edit":{"expected_revision":4,"action":action}}),
        ),
        (
            "copy",
            serde_json::json!({"kind":"structure","edit":{"expected_revision":0,"action":action}}),
        ),
        (
            "copy",
            serde_json::json!({"kind":"structure","edit":{"expected_revision":0,"action":action}}),
        ),
        (
            "copy",
            serde_json::json!({"kind":"structure","edit":{"expected_revision":1,"action":action}}),
        ),
        ("undo-stale", serde_json::json!({"kind":"configuration_undo","expected_revision":0})),
        ("undo", serde_json::json!({"kind":"configuration_undo","expected_revision":1})),
        (
            "capture-stale",
            serde_json::json!({"kind":"capture_view","expected_revision":0,"workspace":"missing","slot":"missing","windows":["missing"],"name":"capture"}),
        ),
        (
            "navigate-stale",
            serde_json::json!({"kind":"navigate","expected_revision":0,"view":view_id,"placement":"missing","slot":"missing"}),
        ),
        ("placements-missing", serde_json::json!({"kind":"placements","window":"missing"})),
    ] {
        serde_json::to_writer(&mut input, &serde_json::json!({"id":id,"command":command}))?;
        input.write_all(b"\n")?;
    }
    drop(input);
    let output = child.wait_with_output()?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let replies: Vec<serde_json::Value> = String::from_utf8(output.stdout)?
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    assert_eq!(replies[0]["error"]["code"], "STALE_REVISION");
    assert_eq!(replies[1]["result"]["revision"], 1);
    assert_eq!(replies[2], replies[1]);
    assert_eq!(replies[3]["error"]["code"], "INVALID_CONFIGURATION");
    assert_eq!(replies[4]["error"]["code"], "STALE_REVISION");
    assert_eq!(replies[5]["result"]["revision"], 2);
    assert_eq!(replies[6]["error"]["code"], "STALE_REVISION");
    assert_eq!(replies[7]["error"]["code"], "STALE_REVISION");
    assert_eq!(replies[8]["error"]["code"], "TARGET_MISSING");
    config.revision = 2;
    assert_eq!(serde_json::to_value(config)?, serde_json::to_value(Configuration::load(&path)?)?);
    let journal = windows_window_manager::Journal::load(&path.with_extension("recovery.json"))?;
    assert!(journal.entries.is_empty());
    Ok(())
}
