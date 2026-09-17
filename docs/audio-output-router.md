# Audio Output Router

`audio-output-router` is a Windows-first CLI for cloning the audio of one output device into another output device.

## What it does

- opens loopback capture on a source render endpoint
- reads the source device's shared-mode mix format
- replays that audio into a target render endpoint

This is intentionally simpler and more stable than trying to infer per-display or per-process audio ownership.

## Usage

Start the interactive Audio Repeater and select source and target outputs by number:

```powershell
just audio-repeater
```

This runs `cargo run -p audio-output-router`. Install `just` and ensure `cargo` is on
your PATH (for a mise-managed environment, use `mise exec -- just audio-repeater`).
Use `just audio-devices` to list outputs without starting the repeater.

After building, you can also run `target\debug\audio-output-router.exe` from a terminal.
Press Ctrl+C to stop. The source continues playing while the target receives a copy.
This is a terminal interface; there is no graphical control window yet.

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
- Selecting the same source and target is rejected to prevent feedback.
- A name fragment must match exactly one device. For duplicate names, use interactive selection or a full device ID from `list-audio-devices`.
- Devices whose friendly names cannot be read appear as `Unnamed output` with their IDs. A name lookup failure no longer prevents listing or selecting other devices; it does not guarantee the unnamed endpoint can be opened.
- If a stream fails (for example after disconnecting a device), routing exits with an error. Reconnect the device and restart; automatic reconnection is not implemented.
- `default` is resolved when routing starts. Changing the Windows default later does not switch the running source.

## Virtual cable use

No virtual cable is required to copy an existing output. To avoid playing on a physical source speaker, install and configure a separate virtual audio cable, direct Windows or the desired application to its playback endpoint, then select that endpoint as the repeater source and a physical output as the target. This app does not install or create virtual audio devices, and does not capture microphone/recording endpoints.
