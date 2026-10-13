# Desktop Workspace

Local web control panels for Desktop Control, Audio Cable, and Window Manager.
Production controls existing Windows adapters through a Rust host. Mirroring
video remains in separate native display-relay windows.

## Start on Windows

Double-click `start-desktop-workspace.cmd` in the repository root. The first run
builds missing binaries/assets and needs Rust, Node.js 22.12+ and pnpm. Later runs
reuse prepared files. Open <http://127.0.0.1:4178> and allow popups for detached panels.

```powershell
# Close the existing workspace with 앱 종료 before preparing updated source.
.\tools\start-desktop-workspace.ps1 -Prepare
.\tools\start-desktop-workspace.ps1 -NoBrowser
.\tools\stop-desktop-workspace.ps1
```

`-BinariesDirectory` accepts a folder containing desktop-workspace-host.exe,
window-manager.exe and display-relay.exe. `-Port` and `-ConfigRoot` allow an isolated
instance. Runtime copies and server logs are in `target/desktop-workspace/runs`;
builds use a separate directory to avoid overwriting executables in use. Identical
binary versions share a cached runtime rather than copying them on every launch.

Existing settings are read without conversion:

- `%LOCALAPPDATA%/DesktopEnvironment/presets.json`
- `%LOCALAPPDATA%/DesktopEnvironment/audio-cable/settings.json`
- `%LOCALAPPDATA%/DesktopEnvironment/window-manager.json` on explicit connection

Startup does not start audio, open mirrors, connect the window manager or save
settings. Stop the old Window Manager before connecting to the same locked file.
Preset/cable saves reject external file changes and protect malformed originals.
Stop owned outputs and use **설정 다시 읽기** before saving after an external change.

## Panels and windows

Drag tabs onto tab strips to combine, or onto panel edges to split. Drag separators
to resize. **새 창으로 이동** detaches a panel; **주 창으로 합치기** returns it. The
activity bar reopens closed panels. Selections, unsaved window edits, previews and
jobs survive panel moves within one workspace instance.

Closing panels or browser tabs preserves native jobs. Use **전체 작업 중지** or
**앱 종료** to stop them. A crashed Rust host terminates its audio/mirror children
through Windows Job Objects; window recovery uses the existing independent watchdog.
The stop script can terminate a recorded broken server after checking PID/start time.

The last complete grid is saved under `desktop-workspace.layout.v1`. Reload restores
that grid and reconnects to jobs, but does not automatically open popups. Unsaved
editor drafts reset on full reload. Independently launched pages do not exchange
panel DOM or drafts. Native settings remain in files.

## Available controls

| Panel | Controls |
| --- | --- |
| Desktop Control | Real source/target output discovery and selection, start/stop, settings and preset CRUD, multiple mirror rows, display/fullscreen/FPS, native mirror launch/stop, final worker logs |
| Audio Cable | Recognized/manual playback-capture pairs, output selection, monitoring start/stop, volume/mute, settings and presets, worker logs |
| Window Manager | Explicit connection, inventories, reference registration/rebinding, view and region creation, group strategies/name/gap/grid columns, window order and placement removal, region adjustment, capability opt-ins, saved editing, preview/apply, undo preview, pause/resume and hidden-window recovery |
| Diagnostics | Command ID, time, outcome, full worker cause/logs, settings directory and connection errors |

Advanced structural editing, collections/selectors, package import/export,
attention providers and shortcut editing remain in the existing native GUI/CLI.
Existing authored fields are preserved when editing the web panel. See
[integration and verification](../../docs/desktop-workspace.md).

## Develop and verify

```powershell
cd apps/desktop-workspace
pnpm install --frozen-lockfile
pnpm dev
pnpm build
pnpm format:check
pnpm test
pnpm test:e2e
pnpm test:native
```

Vite at <http://127.0.0.1:4177> defaults to fixtures. Explicit `?mode=fixture` uses
virtual devices; `?mode=native` requires a same-origin native server. Production
4178 defaults to real service mode and never silently falls back to fixtures.

Chrome tests use an isolated profile and mock native API effects. Native integration
requires host/window-manager under `target/desktop-control/debug`: it queries real
devices and uses temporary settings plus one disposable WinForms window to test
actual move/resize/undo and repeated previews. It never routes audio or moves the
user's existing windows. Rust tests verify corrupt-file protection, save conflicts,
final worker errors and owned-process termination.

These tests do not certify audible output, every driver, capture rendering or every
external application's DPI/resize/hide compatibility.
