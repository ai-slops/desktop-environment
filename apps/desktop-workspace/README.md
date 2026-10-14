# Desktop Workspace

Installed desktop control panels for Desktop Control, Audio Cable, and Window Manager.
Production controls existing Windows adapters through a Rust host. Mirroring
video remains in separate native display-relay windows.

## Start on Windows

Run `just desktop-workspace-install`, then `just desktop-workspace`, or double-click
`start-desktop-workspace.cmd`. The native EXE has Windows' normal title bar and
borders, with no browser tabs, address bar or application menu. Drag a panel tab
outside all workspace windows and release to create another native window.
Installation needs Rust, Node.js 22.12+ and pnpm; the installed app bundles its
runtime and needs none of those build tools to run.

```powershell
just desktop-workspace-install  # safe while UI windows are open
just desktop-workspace
just desktop-workspace-daemon   # start only the background service
just desktop-workspace-stop     # explicitly stop service and its jobs
```

Default installation is `%LOCALAPPDATA%/Programs/DesktopWorkspace`, or
`$XDG_DATA_HOME/desktop-workspace` when explicitly configured. The stable
`DesktopWorkspace.exe` launcher reads `current.json` and starts an immutable
version under `versions/`. Building writes only `target/desktop-workspace-build`;
installing a new version changes the next launch without replacing active images.
Existing daemons/jobs remain on their running version until explicitly stopped.
No installation step stops them. Windows is currently the supported native target.

`-Port`, `-ConfigRoot` and `-InstallRoot` allow separate instances. Settings keep
their existing paths; daemon logs are under the settings directory's `daemon/`.
The packaged UI launches a detached daemon, then quits independently when its
windows close. Reopening the UI reconnects to that daemon. It is a per-user session
daemon, not a privileged Windows system service; automatic boot/logon startup is
not enabled. Use the panel/settings menu to stop the service explicitly.

The browser development path remains available with `-Development` (or explicit
`-BinariesDirectory`); it is not the default installed launch.

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
to resize. In the native EXE, dragging a tab outside the windows detaches it without
a separate button. In the browser, **새 창으로 이동** detaches a panel. The
activity bar reopens closed panels. Selections, unsaved window edits, previews and
jobs survive panel moves within one workspace instance.

Closing panels or browser tabs preserves native jobs. Use **전체 작업 중지** or
**서비스 중지** to stop them. Closing the EXE's UI windows also preserves jobs.
A crashed Rust host terminates its audio/mirror children
through Windows Job Objects; window recovery uses the existing independent watchdog.
The stop script can terminate a recorded broken server after checking PID/start time.

The last complete grid is saved under `desktop-workspace.layout.v1`. Reload restores
that grid and reconnects to jobs, but does not automatically open popups. Unsaved
editor drafts reset on full reload. Independently launched pages do not exchange
panel DOM or drafts. Native settings remain in files.

## Available controls

Monitoring volume accepts direct fractional percentages such as 0.5%, 0.05% and
0.01%. Press Enter or leave the input to apply; the slider's keyboard step is 0.01%.
The existing DSP gain floor is -96 dB (approximately 0.001585%), and the UI shows
the actual returned setting. Presets retain fractional gain rather than rounding.

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

Development/execution tests for this workflow run inside the dedicated test VM.
`tools/test-desktop-workspace-vm.ps1` refuses physical hosts. It installs the prepared
package outside build output, tests fractional values and real native tab tear-off,
closes/reopens the UI around an active audio job, and runs native/DSP regressions.
The scoped VM worker uses the checksum-verified source/package archive. Runtime
cache reuse transfers source changes without recopying Electron on every run.

The owned Hyper-V guest is `DesktopEnvironment-AudioTest`; its identity is stored
in `target/vm-hyperv/owner.json`. Connect with `tools/connect-desktop-test-vm.ps1`
from an elevated PowerShell, passing a DPAPI-protected credential exported with
`Export-Clixml`. No passwords belong in source files. Log into the guest desktop
before native mouse tests. The optional `prepare-workspace-vm-desktop.ps1` helper
performs one guest-only autologon and restores its registry settings at logon.

After preparing `target/desktop-workspace-package`, submit the fixed VM workflow:

```powershell
./tools/package-desktop-workspace-vm.ps1
./tools/request-desktop-test-vm-session.ps1 -Action SyncSource
# Wait for this request's successful response before submitting Test.
./tools/request-desktop-test-vm-session.ps1 -Action Test
```

Requests return paths to results/logs under `target/vm-session/responses`.
`-ReuseGuestSource source-<sha256>` can reuse an earlier complete guest package.
`Shutdown` stops the VM without deleting its installed tools or ownership marker.

Chrome tests use an isolated profile and mock native API effects. Native integration
requires host/window-manager under `target/desktop-control/debug`: it queries real
devices and uses temporary settings plus one disposable WinForms window to test
actual move/resize/undo and repeated previews. It never routes audio or moves the
user's existing windows. Rust tests verify corrupt-file protection, save conflicts,
final worker errors and owned-process termination.

These tests do not certify audible output, every driver, capture rendering or every
external application's DPI/resize/hide compatibility.
