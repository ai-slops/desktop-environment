# Desktop Control

A Windows GUI for selecting, saving, and reusing configurations for
`audio-output-router` and `display-relay`. It launches the existing tools as
separate processes, so their CLI commands and capture/audio behavior remain available.

## Run

```powershell
just desktop-control
# Or, with the mise-managed Rust and just tools:
mise run desktop-control
```

The command builds all three executables in a separate target directory before
opening the panel, avoiding replacement of CLI binaries running from `target\debug`.
After building, double-click `target\desktop-control\debug\desktop-control.exe`.
For a release build:

```powershell
cargo build --target-dir target/desktop-control --release -p desktop-control -p audio-output-router -p display-relay
```

Keep the three executables in the same directory when moving them elsewhere.
Click **바탕화면 바로가기 만들기** at the bottom of the preset sidebar to create
`Desktop Control.lnk` on your Windows desktop (including a redirected/OneDrive desktop).
The shortcut opens this GUI executable with its current configuration file. Clicking
the button again refreshes the same shortcut. After moving the executables, create
the shortcut again from the new location. Save any edited settings before closing;
creating a shortcut does not save unsaved edits or start the tools.
Running `cargo run --target-dir target/desktop-control -p desktop-control` alone requires the other two binaries to
have already been built in the same profile.

## Save and reuse settings

1. Enable **오디오 출력 복제**, **디스플레이 미러링**, or both.
2. Select the source/target audio outputs and the source display. Set fullscreen,
   FPS (1–240), and capture wait (1–1000 ms) as needed.
   Press **미러링 추가** for more windows, then choose each window's source and options.
   The same source display can be opened in multiple windows.
3. **현재 설정 저장** saves the working configuration for the next launch.
   To keep several configurations, enter a name in **새 프리셋 이름** and press
   **새 프리셋으로 저장**.
4. Select a saved preset and press **선택한 설정 불러오기** to apply it.
   **이 프리셋으로 시작** loads and launches it in one step.
   **덮어쓰기** explicitly replaces that preset with the current settings.
   Deleting a preset requires **삭제 확인** and keeps the current settings.
5. **저장하고 시작** saves the configuration and starts enabled tools that are stopped.
   Already running windows and audio are kept running without creating duplicates.
   **모두 중지** stops them together; individual stop buttons are also available.

Opening the panel restores the last saved configuration without starting either tool.
Unsaved edits are marked and are not retained across launches. A running tool's
settings are locked until that tool stops. Other mirror rows remain editable, and
more windows can be added while audio or mirrors are running. Loading another preset
or removing the last additional row requires all tools to be stopped. Existing
single-display presets remain compatible; new presets save all mirror rows.

Each mirror row has **창 열기**, **중지**, and **다시 열기** controls. Closing a mirror
window directly (or pressing Esc) updates only its row to **종료됨**. Use **다시 열기**
to restore that window while other mirrors and audio keep running. Per-window open
uses that row's current settings and does not require valid audio settings or other
mirror rows; use **현재 설정 저장** to retain any edits across panel launches.

The control panel automatically remembers its window size, position, and maximized
state. Each mirror row remembers its own last placement for each source display.
Move or resize the windows normally; no extra save button is needed for geometry.
Mirror video areas keep their physical pixel size when moving between different
Windows Display Scale values; DPI-scaled title bars and borders may change size.
Saved mirror client pixels are also restored after reopening on another scale.
Minimizing a window does not overwrite its normal bounds, and fullscreen mirroring
keeps the previous windowed placement. Windows moves a restored window back onto
an available screen if its saved bounds would be completely off-screen.

Audio outputs are stored by endpoint ID; displays use their Windows display name
(for example `\\.\DISPLAY3`). IDs and settings are passed as literal arguments,
without shell interpolation. The audio dropdown also offers **Windows 기본 출력**,
which resolves when the router starts. Hover over an audio option to see its endpoint
ID and distinguish outputs with the same friendly name.

Disconnected selections stay visible as **연결 안 됨**. They are not silently replaced
with another device. Use **장치 다시 검색** after connecting hardware. Starting rejects
missing devices and source/target pairs that resolve to the same audio endpoint.
Windows can reassign display names when topology changes, so verify the display
selection after rearranging monitors.

## Storage and diagnostics

The default file is `%LOCALAPPDATA%\DesktopEnvironment\presets.json` and its path
is shown in the panel. To use another file:

```powershell
target\desktop-control\debug\desktop-control.exe --config .\my-presets.json
```

Window placements are stored separately beside that file:
`presets.json.control-window.json` and `presets.json.relay-window.json`.
Additional mirror rows use `presets.json.relay-2-window.json`,
`presets.json.relay-3-window.json`, and so on. Separate files prevent concurrently
running windows from overwriting each other's placements, including when they show
the same source display.
Using another `--config` file gives it independent window placements. Automatic
geometry saves do not save unsaved device or preset edits. Invalid placement files
are preserved, and their errors are reported without preventing tool settings from
being used. To reset window placements, close the tools and remove the relevant
placement file.

Saves write and flush a temporary file in the same directory before replacing the
existing JSON. Failed saves leave the in-memory preset list unchanged. If an existing
file is unreadable, malformed, or has an unsupported schema version, the panel shows
the error and prevents writes; repair the file or choose another path and restart.

The panel shows whether each child process is running and retains the latest 120 log
lines for audio and each mirror window. **실행 로그** includes child startup and device errors. A running process
may still be recovering from a device interruption; consult its logs for details.
Closing the panel stops and waits for the child processes it launched. Other tool
instances started outside this panel are unaffected. The mirror window can also be
closed directly or with Esc.

The GUI uses the installed Windows Malgun Gothic font for Korean labels and device
names. The operating-system integrations remain Windows-only.

## GUI render smoke check

The optional `ui-smoke` feature captures the panel after device enumeration and
two seconds of rendering, then closes it. It does not start routing or mirroring.
Use a test config to avoid changing your regular presets:

```powershell
$env:DESKTOP_CONTROL_SCREENSHOT = "$PWD\target\desktop-control.png"
cargo run -p desktop-control --features ui-smoke -- --config .\target\smoke-presets.json
Remove-Item Env:DESKTOP_CONTROL_SCREENSHOT
```

To verify opening two real mirrors, closing and reopening one independently, and
stopping all owned windows (requires a connected display):

```powershell
cargo build --target-dir target/desktop-control -p display-relay
$env:DESKTOP_CONTROL_RELAY_SMOKE_BINARY = "$PWD\target\desktop-control\debug\display-relay.exe"
cargo test -p desktop-control real_mirror_close_reopen_keeps_other_windows_running -- --ignored --nocapture
Remove-Item Env:DESKTOP_CONTROL_RELAY_SMOKE_BINARY
```
