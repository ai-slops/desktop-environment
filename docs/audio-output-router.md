# Audio Output Router

`audio-output-router` is a Windows-first CLI for cloning the audio of one output device into another output device.

## What it does

- opens loopback capture on a source render endpoint
- reads the source device's shared-mode mix format
- replays that audio into a target render endpoint

This is intentionally simpler and more stable than trying to infer per-display or per-process audio ownership.

## Usage

List audio output devices:

```powershell
cargo run -p audio-output-router -- list-audio-devices
```

Clone the default output device into another output device:

```powershell
cargo run -p audio-output-router -- route default "GC553PRO"
```

Clone one specific source output into another output by matching part of each friendly name:

```powershell
cargo run -p audio-output-router -- route "Speakers" "Headphones"
```

## Current behavior and limits

- Windows must already be sending the desired app's audio to the chosen source output device.
- This duplicates the whole source endpoint mix, not one process.
- Source and target are shared-mode streams, so final latency and resampling behavior follow Windows audio engine rules.
