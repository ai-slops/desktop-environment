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
commit `77b6fc3`. Desktop Control GUI/tool compilation and the full workspace
test run remain in progress. These synthetic callback tests do not certify live
A/B/C audio routing; that acceptance remains **pending**.
The Hyper-V test VM was created through
UAC elevation on 2026-10-10 and started with 32 GiB RAM / 12 vCPUs. Its console was
opened for Windows installation. Windows 11 Pro build 26200 and the C++/mise/Rust
toolchain are installed; audio fixture validation remains.
No host audio defaults, playback, or running router processes were modified.
