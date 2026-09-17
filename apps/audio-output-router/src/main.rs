use anyhow::{Context, Result, bail};
use std::io::{self, Write};
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
        Command::Interactive => interactive(),
        Command::Help => {
            println!(
                "Audio Repeater\nUsage: audio-output-router [list-audio-devices | route <SOURCE|default> <TARGET|default>]\nWithout arguments, select devices interactively. Selectors accept a full device ID or a unique name fragment.\nThe source keeps playing. Press Ctrl+C to stop."
            );
            Ok(())
        }
        Command::ListAudioDevices => list_audio_devices(),
        Command::Route { source_device, target_device } => {
            info!("Press Ctrl+C to stop the router");
            run_output_audio_router(&source_device, &target_device)
        }
    }
}

enum Command {
    Interactive,
    Help,
    ListAudioDevices,
    Route { source_device: String, target_device: String },
}

impl Command {
    fn from_env() -> Result<Self> {
        let mut args = std::env::args().skip(1);
        let Some(command) = args.next() else {
            return Ok(Self::Interactive);
        };

        let command = match command.as_str() {
            "--help" | "-h" | "help" => Self::Help,
            "list-audio-devices" => Self::ListAudioDevices,
            "route" => {
                let source_device = args
                    .next()
                    .context("route requires a source output device match or 'default'")?;
                let target_device = args
                    .next()
                    .context("route requires a target output device match or 'default'")?;
                Self::Route { source_device, target_device }
            }
            other => bail!("Unknown command: {other}"),
        };
        if args.next().is_some() {
            bail!("Unexpected extra arguments; use --help for usage");
        }
        Ok(command)
    }
}

fn interactive() -> Result<()> {
    let devices = list_output_devices()?;
    if devices.len() < 2 {
        bail!(
            "Audio Repeater needs at least two active output devices. Connect another output and try again."
        );
    }
    println!("Audio Repeater — copy PC audio to another output\n");
    for (index, device) in devices.iter().enumerate() {
        let marker = if device.is_default { " (default)" } else { "" };
        println!("{}. {}{}", index + 1, device.friendly_name, marker);
    }
    let source = read_device_number("Source output number: ", devices.len())?;
    let target = loop {
        let target = read_device_number("Target output number: ", devices.len())?;
        if source != target {
            break target;
        }
        println!("Choose a different output to prevent audio feedback.");
    };
    println!("\n{} -> {}", devices[source].friendly_name, devices[target].friendly_name);
    println!("The source keeps playing. Press Ctrl+C to stop.");
    run_output_audio_router(&devices[source].id, &devices[target].id)
}

fn read_device_number(prompt: &str, count: usize) -> Result<usize> {
    loop {
        print!("{prompt}");
        io::stdout().flush()?;
        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            bail!("Input closed; use route <SOURCE> <TARGET> for non-interactive use");
        }
        if let Ok(number) = input.trim().parse::<usize>()
            && (1..=count).contains(&number)
        {
            return Ok(number - 1);
        }
        println!("Enter a number between 1 and {count}.");
    }
}

fn list_audio_devices() -> Result<()> {
    for device in list_output_devices()? {
        let default_marker = if device.is_default { " (default)" } else { "" };
        println!("{}\t{}{}", device.id, device.friendly_name, default_marker);
    }

    Ok(())
}
