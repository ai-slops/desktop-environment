use anyhow::{Context, Result, bail};
use tracing::info;
use windows_audio_router::{list_output_devices, run_output_audio_router};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    match Command::from_env()? {
        Command::ListAudioDevices => list_audio_devices(),
        Command::Route { source_device, target_device } => {
            info!("Press Ctrl+C to stop the router");
            run_output_audio_router(&source_device, &target_device)
        }
    }
}

enum Command {
    ListAudioDevices,
    Route { source_device: String, target_device: String },
}

impl Command {
    fn from_env() -> Result<Self> {
        let mut args = std::env::args().skip(1);
        let Some(command) = args.next() else {
            bail!(
                "Usage: audio-output-router list-audio-devices | route <SOURCE_MATCH|default> <TARGET_MATCH|default>"
            )
        };

        match command.as_str() {
            "list-audio-devices" => Ok(Self::ListAudioDevices),
            "route" => {
                let source_device = args
                    .next()
                    .context("route requires a source output device match or 'default'")?;
                let target_device = args
                    .next()
                    .context("route requires a target output device match or 'default'")?;
                Ok(Self::Route { source_device, target_device })
            }
            other => bail!("Unknown command: {other}"),
        }
    }
}

fn list_audio_devices() -> Result<()> {
    for device in list_output_devices()? {
        let default_marker = if device.is_default { " (default)" } else { "" };
        println!("{}\t{}{}", device.id, device.friendly_name, default_marker);
    }

    Ok(())
}
