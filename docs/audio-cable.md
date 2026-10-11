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
