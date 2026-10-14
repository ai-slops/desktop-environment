# Audio Cable

Minimal Windows GUI for an installed virtual audio cable and quieter listening.
Run `mise run audio-cable`, or build with `mise run audio-cable-build` and open
`target/release/audio-cable.exe`. The executable is standalone; it does not need
the router executable beside it. Install VB-CABLE separately using the official
link provided in the app. Driver installation may require elevation and reboot.

1. Select **CABLE Input** as the source application's playback device.
2. In Audio Cable, select the cable and the headphones/speakers used for listening.
3. Set listening volume as a percentage (100% original amplitude, 10% one tenth,
   0% mute). Enter a preset name and click **프리셋 저장**.
4. Click a saved preset to start listening or switch outputs and volume immediately.
   You do not need to stop listening first. Saving alone does not start playback.
5. In OBS, Discord or another recording application, select **CABLE Output**.

The cable's recording endpoint keeps the original signal level. Additional
attenuation affects only this app's listening output. It multiplies floating-point
samples before rendering, so it works independently of the device's minimum
volume step. The percentage scales signal amplitude, rather than perceived loudness. Changes ramp over
20 ms. This app does not change Windows defaults or endpoint volume. Listening
starts only on request and stops when the app closes.

The Windows routing thread registers with MMCSS (`Pro Audio`) so ordinary CPU
work does not share its scheduling priority. Shared WASAPI buffers request
100 ms capacity, and listening starts with 50 ms of queued silence to absorb
scheduling jitter. This adds roughly 50 ms of monitoring delay; it does not
change the cable recording stream. If MMCSS is unavailable, the router logs a
warning and continues with buffering alone. Long scheduler stalls can still
exhaust the reserve.

A preset stores the cable, listening output and volume. Clicking an unavailable
preset shows an error and preserves the current route. Reusing the same output
updates gain in place; changing outputs stops the old owned route before starting
the new one. Closing and reopening the app restores saved presets without auto-start.
Existing single-configuration files still load. Extremely small nonzero values are
limited to the engine’s −96 dB gain floor and destination conversion precision.

Settings are saved to `%LOCALAPPDATA%/DesktopEnvironment/audio-cable/settings.json`.
Use `--config <path>` for a separate configuration. Saved endpoint IDs are
validated against current devices; missing devices are not silently substituted.
Manual playback/recording pairing supports other installed cable drivers.

## Backend boundary

If a recognized VB-CABLE playback endpoint rejects shared loopback with
`AUDCLNT_E_DEVICE_IN_USE` (`0x8889000A`), the router tries that cable's uniquely
matched recording endpoint in shared mode without the loopback flag. For example,
`CABLE Output` carries the signal entering `CABLE Input` as described in the
[VB-CABLE manual](https://vb-audio.com/Cable/VBCABLE_ReferenceManual.pdf).
The log identifies this path. Physical outputs, ambiguous/missing cable pairs and
other error codes never select a different capture device. If the paired recording
endpoint also fails, its cause and the original loopback error are both preserved.
This does not change driver settings, Windows defaults or other applications.

`audio-output-router check-route <source> <target>` validates the same fallback
without starting streams. The opt-in `native_cable_initialization_diagnosis` test
compares raw loopback and paired recording initialization on the selected host
devices using `DESKTOP_AUDIO_SOURCE` and optional `DESKTOP_AUDIO_TARGET`. Unlike
signal/CPU-load acceptance tests, this diagnostic does not generate or play audio.

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

## VM CPU-load validation

Inside the dedicated Windows test VM, run
`mise exec -- powershell -NoProfile -ExecutionPolicy Bypass -File tools/test-desktop-control-vm.ps1 -AudioOnly`.
The runner refuses physical hosts. It verifies attenuation and runs one bounded
CPU worker per guest logical processor while recording the cable input and
listening output together. The probe requires measured CPU usage of at least
90%, verifies the upstream tone, and checks output tone amplitude and sustained
silence. Recordings go to the guest's local test output directory, not tracked
reports. The VM source archive also accepts the ignored per-run option
`target/vm-test-options.json` containing `{"scope":"audio"}` for the scoped worker.

