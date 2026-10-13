#![allow(clippy::multiple_crate_versions)] // Existing Windows adapters use two windows-link versions.
use anyhow::{Context, Result, bail};
use desktop_workspace_service::{Command, Controller};
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let mut root = None;
    while let Some(arg) = args.next() {
        if arg == "--audio-worker" {
            return desktop_workspace_service::run_audio_worker();
        }
        if arg == "--config-root" {
            root = Some(PathBuf::from(args.next().context("--config-root requires a directory")?));
        } else {
            bail!("Unknown argument: {}", arg.to_string_lossy());
        }
    }
    let root = match root {
        Some(path) => std::path::absolute(path)?,
        None => PathBuf::from(
            std::env::var_os("LOCALAPPDATA")
                .context("LOCALAPPDATA unavailable; use --config-root")?,
        )
        .join("DesktopEnvironment"),
    };
    let binaries =
        std::env::current_exe()?.parent().context("Executable directory missing")?.to_owned();
    let mut controller = Controller::new(root, binaries);
    controller.refresh()?;
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    loop {
        let mut bytes = vec![];
        let count = input.by_ref().take(1_048_577).read_until(b'\n', &mut bytes)?;
        if count == 0 {
            break;
        }
        if count > 1_048_576 {
            bail!("Command exceeds 1 MiB budget");
        }
        let parsed: Result<Value> = serde_json::from_slice(&bytes).map_err(Into::into);
        let response = match parsed {
            Ok(envelope) => {
                let id = envelope["id"].as_str().filter(|id| !id.is_empty() && id.len() <= 200);
                id.map_or_else(|| json!({"id":null,"ok":false,"error":{"code":"INVALID_REQUEST","message":"Invalid request ID"}}), |id| {
                    let operation = envelope["command"]["kind"].as_str().unwrap_or("invalid");
                    let result = serde_json::from_value::<Command>(envelope["command"].clone())
                        .map_err(Into::into)
                        .and_then(|command| controller.execute(command));
                    match result {
                        Ok(mut data) => {
                            controller.record_trace(id, operation, true, "명령 완료");
                            data["state"] = controller.snapshot();
                            json!({"id":id,"ok":true,"data":data})
                        }
                        Err(error) => {
                            let message = format!("{error:#}");
                            controller.record_trace(id, operation, false, &message);
                            json!({"id":id,"ok":false,"error":{"code":"COMMAND_FAILED","message":message},"data":{"state":controller.snapshot()}})
                        }
                    }
                })
            }
            Err(error) => {
                json!({"id":null,"ok":false,"error":{"code":"INVALID_REQUEST","message":error.to_string()}})
            }
        };
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
        if controller.shutdown {
            break;
        }
    }
    Ok(())
}
