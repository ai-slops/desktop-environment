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

For graphical display selection, fullscreen/FPS/capture-timeout controls, and saved
presets shared with the audio router, run `just desktop-control`.
See [Desktop Control](desktop-control.md).

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

Windowed size, position, and maximized state are automatically remembered per source
display. Minimized and fullscreen bounds do not overwrite the normal window placement.
The CLI uses `%LOCALAPPDATA%\DesktopEnvironment\presets.json.relay-window.json` by
default; the GUI supplies the file associated with its selected configuration.
For a separate layout file:

```powershell
cargo run -p display-relay -- mirror \\.\DISPLAY3 --window-state-file .\relay-windows.json
```

The mirror's video area uses physical pixels, so moving between displays with
different Windows Display Scale values (for example 100% and 200%) preserves its
pixel width and height. Initial sizing and resize increments also use pixels.
Title bars and borders still follow Windows DPI; fullscreen and maximized windows
continue to fit their monitor. Saved placements include the normal client pixel
size, and reopening on a different scale accounts for the new border sizes after
the startup DPI events finish. Older placement files remain readable and gain this
size information the next time a windowed mirror saves its placement.

To test moving an owned mirror window across the connected monitors and reopening
it without changing Windows settings:

```powershell
./tools/smoke-display-dpi.ps1
```

The check uses a temporary file under `target`, closes only its own processes, and
reports which DPI values were actually exercised. If all connected monitors use
the same DPI, it reports that a cross-DPI transition was not verified.

## Important constraints

- The target output still needs to exist as a Windows desktop display. Many HDMI dummy plugs and capture devices do this well; pure EDID-less sinks do not.
- Desktop Duplication can lose access when the GPU topology changes (for example another display is connected or disconnected), the display sleeps, the session is disconnected, or the secure desktop is shown (UAC prompts, the lock screen, Ctrl+Alt+Del). The secure desktop case also makes the swap chain's `Present` call and `GetCursorPos` fail with access-denied, even when capture and the device are otherwise fine. The relay window no longer closes when any of this happens: it keeps the window, D3D11 device, and swap chain untouched and just freezes on the last captured frame (hiding the cursor overlay too) until things recover, which happens automatically.
  - For Desktop Duplication specifically, recovery re-acquires just the duplication interface roughly twice a second, reusing the existing device rather than recreating it. The device is *not* what goes stale when a display is unplugged and replugged — a cached DXGI adapter is — so each retry re-enumerates adapters from a brand-new DXGI factory and matches the one that owns the existing device by GPU LUID, rather than creating a new device to sidestep the staleness. Each retry also explicitly releases the previous duplication interface first (Desktop Duplication allows only one active interface per output at a time; leaving the old, already-inaccessible one alive made every subsequent `DuplicateOutput` call fail with access-denied forever, well past the point the secure desktop was gone) and un-minimizes the window if Windows auto-minimized it for the secure desktop (only while actively recovering, so it never fights you minimizing the window yourself while the relay is healthy).
  - `Present` failing during the secure desktop is left alone entirely: just skip that frame and retry the next one. Earlier attempts recreated the whole session (device and swap chain included) on any access-denied error, mirroring what a full process restart does — but Windows allows only one flip-model swap chain per window for its *entire lifetime*, and D3D11 defers destroying a replaced one; tearing an old swap chain down and immediately creating a new one for the same window was observed to fail with access-denied indefinitely. The window, device, and swap chain are now created exactly once and never rebuilt.
  - `GetCursorPos` failing is handled the same way: the cursor overlay is just hidden for that frame instead of the redraw failing.
- Keyboard forwarding currently covers common keys through scan-code mapping, not every extended key.
- The relay now uploads BGRA frames into a GPU texture and lets the window renderer scale/present them. Capture still uses a CPU-readable staging texture because that is the Desktop Duplication handoff point in this implementation.

## Good next steps

- reduce the capture-side CPU copy by sharing a DXGI/D3D texture directly with the presenter
- add explicit output selection for the mirror window itself
- add wheel input and more complete extended-key support
- add optional cursor-lock / relative-input mode
