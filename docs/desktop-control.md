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
3. **현재 설정 저장** saves the working configuration for the next launch.
   To keep several configurations, enter a name in **새 프리셋 이름** and press
   **새 프리셋으로 저장**.
4. Select a saved preset and press **선택한 설정 불러오기** to apply it.
   **이 프리셋으로 시작** loads and launches it in one step.
   **덮어쓰기** explicitly replaces that preset with the current settings.
   Deleting a preset requires **삭제 확인** and keeps the current settings.
5. **저장하고 시작** saves the configuration and starts the enabled tools.
   **모두 중지** stops them together; individual stop buttons are also available.

Opening the panel restores the last saved configuration without starting either tool.
Unsaved edits are marked and are not retained across launches. While a tool is running,
stop all running tools before changing settings or loading another preset.

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

Saves write and flush a temporary file in the same directory before replacing the
existing JSON. Failed saves leave the in-memory preset list unchanged. If an existing
file is unreadable, malformed, or has an unsupported schema version, the panel shows
the error and prevents writes; repair the file or choose another path and restart.

The panel shows whether each child process is running and retains the latest 120 log
lines per tool. **실행 로그** includes child startup and device errors. A running process
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
