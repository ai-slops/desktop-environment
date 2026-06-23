# Display Relay for Headless Capture Outputs

This workspace now includes a Windows-first relay app that targets the "monitor connected only to a capture card" workflow:

- capture a desktop-attached output with Desktop Duplication
- mirror it into a controllable local window
- forward mouse and keyboard input back into the hidden source display

## Crate split

- `libs/display-relay-core`: shared display geometry and input-coordinate mapping.
- `platforms/windows-desktop-duplication`: DXGI/D3D11 capture adapter.
- `platforms/windows-input`: `SendInput` wrapper for remote pointer and keyboard injection.
- `apps/display-relay`: executable that lists displays or opens the relay window.

## Why this shape

The capture-card-only use case usually needs two things at once:

1. A stable way to grab frames from a specific Windows output.
2. A deterministic way to translate local window input into the hidden display's desktop coordinates.

Keeping those concerns separate makes it easier to reuse the platform crates later for:

- a low-latency streamer
- a web-controlled remote surface
- a multi-monitor router
- an OBS-facing helper tool

## Usage

List available desktop-attached outputs:

```powershell
cargo run -p display-relay -- list
```

Mirror a specific display into a local window:

```powershell
cargo run -p display-relay -- mirror \\.\DISPLAY3
```

Open the mirror fullscreen on another monitor:

```powershell
cargo run -p display-relay -- mirror \\.\DISPLAY3 --fullscreen
```

## Important constraints

- The target output still needs to exist as a Windows desktop display. Many HDMI dummy plugs and capture devices do this well; pure EDID-less sinks do not.
- Desktop Duplication can lose access when the GPU topology changes, the display sleeps, or the session is disconnected. Recreate the session when that happens.
- Keyboard forwarding currently covers common keys through scan-code mapping, not every extended key.
- The current renderer copies the captured frame into CPU memory each frame. It is correct for an MVP, but not yet the lowest-latency path.

## Good next steps

- move rendering to a GPU-backed presenter to avoid CPU blits
- add explicit output selection for the mirror window itself
- add wheel input and more complete extended-key support
- add optional cursor-lock / relative-input mode
