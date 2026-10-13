use anyhow::{Result, bail};
use audio_cable_core::{RouteControl, RouteState};
use serde::Deserialize;
use serde_json::json;
use std::io::{BufRead, Read};
use std::sync::Arc;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Initial {
    source: String,
    target: String,
    gain_db: f32,
    muted: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    gain_db: f32,
    muted: bool,
}

#[cfg(windows)]
pub fn run_audio_worker() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_target(false)
        .init();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();
    input.by_ref().take(65_537).read_line(&mut line)?;
    if line.len() > 65_536 {
        bail!("Worker command exceeds budget");
    }
    let initial: Initial = serde_json::from_str(&line)?;
    let control = Arc::new(RouteControl::new(initial.gain_db, initial.muted)?);
    let route = control.clone();
    let worker = std::thread::spawn(move || {
        let result = windows_audio_router::run_controlled_audio_router(
            &initial.source,
            &initial.target,
            &route,
        );
        if let Err(error) = result {
            eprintln!("Error: {error:#}");
            std::process::exit(1);
        }
    });
    let observing = control.clone();
    let observer = std::thread::spawn(move || {
        let mut last = RouteState::Stopped;
        while !observing.is_stopped() {
            let state = observing.state();
            if state != last && state != RouteState::Stopped {
                let phase = if state == RouteState::Running { "running" } else { "recovering" };
                println!("{}", json!({"kind":"worker_state","phase":phase}));
                last = state;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    });
    let result = (|| -> Result<()> {
        loop {
            line.clear();
            let count = input.by_ref().take(65_537).read_line(&mut line)?;
            if count == 0 {
                break;
            }
            if count > 65_536 {
                bail!("Worker command exceeds budget");
            }
            let update: Update = serde_json::from_str(&line)?;
            control.set_gain(update.gain_db)?;
            control.set_mute(update.muted);
        }
        Ok(())
    })();
    control.stop();
    worker.join().map_err(|_| anyhow::anyhow!("Audio worker panicked"))?;
    observer.join().map_err(|_| anyhow::anyhow!("Audio observer panicked"))?;
    result
}
#[cfg(not(windows))]
pub fn run_audio_worker() -> Result<()> {
    bail!("Native audio workers require Windows")
}
