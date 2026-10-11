# Audio Cable

Minimal Windows GUI for an installed virtual audio cable and quieter listening.
Run `mise run audio-cable`, or build with `mise run audio-cable-build` and open
`target/release/audio-cable.exe`. The executable is standalone; it does not need
the router executable beside it. Install VB-CABLE separately using the official
link provided in the app. Driver installation may require elevation and reboot.

1. Select **CABLE Input** as the source application's playback device.
2. In Audio Cable, select the cable and the headphones/speakers used for listening.
3. Choose additional attenuation and start listening. The default is −30 dB;
   the range is 0 to −96 dB, with a separate mute control.
4. In OBS, Discord or another recording application, select **CABLE Output**.

The cable's recording endpoint keeps the original signal level. Additional
attenuation affects only this app's listening output. It multiplies floating-point
samples before rendering, so it works independently of the device's minimum
volume step. −20 dB gives 10% amplitude; −60 dB gives 0.1%. Changes ramp over
20 ms. This app does not change Windows defaults or endpoint volume. Listening
starts only on request and stops when the app closes.

Settings are saved to `%LOCALAPPDATA%/DesktopEnvironment/audio-cable/settings.json`.
Use `--config <path>` for a separate configuration. Saved endpoint IDs are
validated against current devices; missing devices are not silently substituted.
Manual playback/recording pairing supports other installed cable drivers.

## Backend boundary

`audio-cable-core` owns device descriptors, settings, route controls and gain
processing, without Windows or VB-CABLE dependencies. `AudioBackend` supplies
endpoint discovery, cable pairing, setup links and monitoring. The Windows
adapter uses WASAPI; VB-CABLE name recognition and its installer link live in
`installed_cables.rs`. The GUI uses the backend contract and accepts manual
provider-independent pairs.

A future own Windows driver can supply another discovered pair without replacing
the GUI or gain processing. A Linux backend can implement the same contract using
PipeWire. **Native Linux routing and an own Windows driver are not implemented in
this initial version.** This application does not create or install audio devices.

## Validation

The VM-only acceptance test records the actual cable recording endpoint and the
listening endpoint, checks −20/−40/−60/−96/−30 dB updates on one running route,
mute and recovery, and verifies that listening attenuation leaves recording
levels intact. The VM runner also captures the native GUI and runs workspace
tests. Host audio is never used for these checks.

Very low output levels depend on the destination's conversion precision. The VM
Voicemeeter endpoint was observed to truncate samples to `2^-18` steps: at −96 dB
the recorded waveform contains zero and one negative step. Consequently total
RMS includes a DC component and differs from the requested gain. The acceptance
probe checks the known tone in 100 ms windows and total RMS separately, allowing
this measured quantization limit only at −96 dB. These fixture checks do not
certify all-channel fidelity, bit-perfect reproduction or dropout-free playback.

### Verified Windows VM run (2026-10-11)

Commit `44480bc`, worker receipt `2e73c0d9-8178-41dd-bd28-09c38fe074d8`:

| Requested gain | Recorded known-tone gain |
| --- | --- |
| −20 dB | −20.0000 dB |
| −40 dB | −39.9996 dB |
| −60 dB | −59.9950 dB |
| −96 dB | −95.8174 dB |
| −30 dB | −30.0001 dB |

Live gain/mute/unmute, unchanged cable recording, owned route shutdown, default
output switching and GUI router lifecycle all passed. Native Audio Cable and
Desktop Control viewports rendered and exited successfully. Full workspace:
151 passed, 0 failed, 8 intentionally ignored across 21 suites; the three opt-in
live acceptance tests passed separately. New core/app Clippy checks and the
release build passed. The Hyper-V VM retained 32 GiB RAM and 12 virtual CPUs.

Enhanced-session screenshots expose Remote Audio rather than the virtual cable
inventory. Actual cable signal checks run in session 0 using guest-only fixtures;
the rendered GUI and provider-independent control test cover the separate UI
path. Screenshot: `target/vm-audio-cable-gui-evidence/08036a15-2c30-4329-9b7b-64131064c6d3/panel.png`.
