# Desktop Control audio default-switch regression

Perform audio playback, endpoint/default changes, GUI interactions, and display
tests inside a Windows VM while the host is broadcasting. Host checks for this fix
are limited to source inspection, formatting, and a single-job static build. Do not
replace or restart the host's running Desktop Control/router binaries during a stream.
The same-host VM still uses host CPU, memory, and disk; use an existing guest with
modest resource limits or a separate test machine. Do not enable virtualization
features, reboot the host, attach its production audio devices, or change its OBS
configuration as part of this test.

For host setup on this Windows PC or a Linux host, see
[the VM setup guide](desktop-test-vm-hosts.md). Development defaults are 32 GiB RAM
and 12 vCPUs. Both hosts run these acceptance checks inside a Windows guest.

## Guest setup and automated checks

Use an interactive Windows guest with Rust 1.88 or newer and the MSVC linker/Windows
SDK. Copy/extract the source into a guest-local directory. Avoid writing through a
shared checkout. For live acceptance, the guest needs three distinct active render
endpoints A, B, and C, independent of the host's broadcasting devices. Endpoint
names alone do not prove this: confirm distinct IDs with `list-audio-devices`.
A guest exposing only one remote audio endpoint cannot validate this scenario.

From the guest, run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/test-desktop-control-vm.ps1
```

The script checks the VM model before running tests, builds into guest-local
`%LOCALAPPDATA%/DesktopEnvironment/vm-test-target`, and enumerates guest endpoints.
It does not change defaults or start audio playback. Automated tests cover the
notification role filter, missing defaults, successive events during reconnect,
default selector re-resolution, fixed endpoint selection, and feedback collisions.
They do not certify audible routing or native callback delivery.

## Live acceptance inside the VM

Start the guest-local `desktop-control.exe` with a new guest-local `--config` path.
Disable display mirroring, enable audio cloning, select **Windows 기본 출력** as
source and C as target. Start with A as the guest's default output and use a guest
audio player configured to follow that default. Observe the router log and compare
the captured/output signal; use different known signals on A and B when practical.
Reopening the player after a default change may be needed if the player itself
pins its original endpoint. Record Windows version, endpoint IDs/driver versions,
the tested commit, transition time, and captured signal evidence for each result.

| Action | Expected result |
| --- | --- |
| Start default A → fixed C | A's signal is cloned to C. |
| Change guest default A → B while A remains active | Log reconnects B → C; C receives B's signal and stops receiving A's signal. |
| Change B → A repeatedly, including during reconnect | Final connection follows the final default; no manual router restart is needed. |
| Change default to C | Cloning pauses with a feedback-prevention diagnostic; no C → C stream opens. |
| Change default C → B | Routing resumes B → C. |
| Remove/disable the current default, then restore it | After an established route, retry/reconnect resumes without restarting the GUI. |
| Use fixed source A → C and change the default | A remains the selected source. |
| Use fixed source C → default A, then change default to B | Destination reconnects to B; C remains the selected source. |
| Stop/restart routing through the GUI | Stops without changing the guest default; restart uses the current default. |

## Current evidence

The old implementation resolved `default` only at session startup and waited for a
stream error before re-resolving it. A connected old endpoint could keep working
indefinitely after a default change. The fix watches render/eConsole changes using
`IMMNotificationClient`, marks each change with an atomic epoch, and tears down the
old streams on the routing thread, including during render-buffer waits. Epochs
are sampled before device selection so changes during reconnection are retained.

Static checks of the fix and regression test code pass. On 2026-10-10 the Windows
11 Pro guest passed all **7 windows-audio-router tests** (including default-change
notifications, removal/reconnection and fixed-selector regressions) and all
**9 desktop-presets tests**, with zero failures. The tested source snapshot is
commit `77b6fc3`. Desktop Control GUI/tool compilation completed. A subsequent
interactive guest run at `f3e8efd` passed all 19 workspace test suites: **133
passed, zero failed, 5 ignored**. Four window-manager service tests initially
failed in PowerShell Direct's session 0 because it has no interactive monitor
inventory; they all passed when the runner used the logged-in guest desktop.
These synthetic callback tests do not certify live
A/B/C audio signal fidelity; captured-tone acceptance remains **pending**.
The guest now has VB-CABLE Pack45 and Voicemeeter Banana 2.1.3.0 installed,
with ten active virtual render endpoints after reboot.

At `2f71890`, `tools/test-desktop-control-default-audio.ps1` passed four real
Windows endpoint/stream checks using Voicemeeter Input (A), Voicemeeter AUX Input
(B) and CABLE Input (C):

- A remained active while changing the default A to B; the same router process
  started capture/render on B to C and emitted the successful resume event.
- Default C triggered feedback prevention; default B resumed B to C.
- Fixed A to C remained running when the default changed to B.
- Fixed C to default A reconnected to C to B.

The VM-only script uses NirSoft's official SoundVolumeView command-line helper to
change eConsole defaults, verifies each change via actual endpoint enumeration,
and restores the original guest default/kills its owned router on every exit.
It records router logs in the guest and validates events after each transition,
including successful stream starts. It generates no test tones and therefore
does **not** prove that C receives only B's signal after switching. SoundVolumeView
is downloaded only inside the VM; it is not bundled in this application.
See the [publisher's command documentation](https://www.nirsoft.net/articles/set_default_audio_device_command_line.html).
The Hyper-V test VM was created through
UAC elevation on 2026-10-10 and started with 32 GiB RAM / 12 vCPUs. Its console was
opened for Windows installation. Windows 11 Pro build 26200 and the C++/mise/Rust
toolchain are installed; audio fixture validation remains.
No host audio defaults, playback, or running router processes were modified.

### Mixed DPI and topology regressions

The workspace suite includes synthetic monitor inventories and planner snapshots
for the following cases. Run them through the VM worker's `Test` action, which
also runs native tests on the logged-in guest desktop.

| Case | Expected behavior |
| --- | --- |
| Add a 100%, 125%, 150%, 175% or 200% monitor | Existing target geometry, DPI and saved preferences stay unchanged. |
| Fractional scaling on a portrait monitor at negative coordinates | Logical size/position and nonclient borders round to the expected physical pixels; settlement rejects wrong DPI or a one-pixel client mismatch. |
| Remove the target while an unrelated scaled monitor exists | No implicit fallback or saved mapping rewrite. |
| Ordered fallback through missing, scaled and same-scale monitors | First available configured target requires explicit cross-DPI compatibility; rejection preserves configuration. |
| Reconnect the original monitor with a new DPI and work area | Original logical preferences use the fresh DPI/work area; fallback preferences remain independent on the next disconnect. |
| Reorder, add or remove uniquely identified monitors | Hardware identity survives enumeration reordering and hotplug; inventory reports current DPI and work area. |
| Change scale, taskbar work area, orientation or connection state | Manual confirmations expire; stale confirmations fail and reconnecting requires current-topology confirmation. |
| Same hardware identifier at different scales/positions | Geometry and DPI do not silently resolve ambiguous identities. |
| Invalid DPI or duplicate device inventory | Reject the update without discarding a valid confirmed mapping. |

These tests exercise mapping/planning contracts without changing host display
settings. They do not emulate Windows delivering `WM_DPICHANGED` to a real app.
Physical docking, mixed-DPI native window behavior and Chrome/Edge compatibility
remain separate acceptance checks; cross-DPI transfer remains opt-in.

On 2026-10-10 the Windows guest passed all nine new regressions and the full
workspace suite at `29d8d8e`: **142 passed, zero failed, 5 ignored across 19
suites**. The host-side worker receipt is
`target/vm-session/responses/e8c17142-a38d-4f71-989a-315762fe7661.json` and the
corresponding `.log` contains individual test results (both are ignored local
artifacts). The guest audio setup also passed its four live reconnection checks.
The stalled-window service fixtures now create their disposable windows with
per-monitor DPI awareness, matching the adapter's coordinate system on scaled
guest desktops; production cross-DPI compatibility restrictions are unchanged.
The full VM runner uses `--no-fail-fast` to collect results from every test target.
