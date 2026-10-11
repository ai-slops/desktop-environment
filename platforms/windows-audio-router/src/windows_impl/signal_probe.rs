//! Explicitly opted-in, VM-only live signal acceptance. Never part of normal tests.
use super::*;
use std::f64::consts::TAU;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn command(path: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(path);
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW, guest helper only.
    command
}

fn layout(format: &WaveFormatOwned) -> Result<(usize, u32, usize, bool)> {
    let bytes = &format.bytes;
    let tag = u16::from_le_bytes([bytes[0], bytes[1]]);
    let channels = usize::from(u16::from_le_bytes([bytes[2], bytes[3]]));
    let rate = u32::from_le_bytes(bytes[4..8].try_into()?);
    let bits = usize::from(u16::from_le_bytes([bytes[14], bytes[15]]));
    let float = tag == 3 || (tag == 0xfffe && bytes.get(24..28) == Some(&[3, 0, 0, 0]));
    if channels == 0
        || rate == 0
        || ![16, 24, 32].contains(&bits)
        || (float && bits != 32)
        || format.block_align != channels * bits / 8
    {
        bail!("Unsupported probe format: {}", format.describe());
    }
    Ok((channels, rate, bits / 8, float))
}

struct Tone {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<Result<()>>>,
}
impl Tone {
    fn start(id: String, frequency: f64) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (ready, receive) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || -> Result<()> {
            let _com = ComGuard::new()?;
            let device = output_device_by_id(&id)?;
            let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }?;
            let format = WaveFormatOwned::from_mix_format(&client)?;
            let (channels, rate, bytes, float) = layout(&format)?;
            let render = open_render_client(&device, &format)?;
            unsafe { render.client.Start() }?;
            ready.send(())?;
            let mut frame_index = 0_u64;
            while !stopped.load(Ordering::Relaxed) {
                let available = render
                    .buffer_frames
                    .saturating_sub(unsafe { render.client.GetCurrentPadding() }?);
                if available == 0 {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                let pointer = unsafe { render.render.GetBuffer(available) }?;
                let data = unsafe {
                    slice::from_raw_parts_mut(pointer, available as usize * format.block_align)
                };
                for frame in data.chunks_exact_mut(format.block_align) {
                    let sample =
                        (TAU * frequency * frame_index as f64 / f64::from(rate)).sin() * 0.15;
                    for channel in frame.chunks_exact_mut(bytes).take(channels) {
                        if float {
                            channel.copy_from_slice(&(sample as f32).to_le_bytes());
                        } else {
                            let value = (sample * ((1_u64 << (bytes * 8 - 1)) - 1) as f64) as i32;
                            channel.copy_from_slice(&value.to_le_bytes()[..bytes]);
                        }
                    }
                    frame_index += 1;
                }
                unsafe { render.render.ReleaseBuffer(available, 0) }?;
            }
            Ok(())
        });
        let tone = Self { stop, thread: Some(thread) };
        receive.recv_timeout(Duration::from_secs(10)).context("Tone stream did not start")?;
        Ok(tone)
    }
}
impl Drop for Tone {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            if let Ok(Err(error)) = thread.join() {
                eprintln!("Tone stream failed: {error:#}");
            }
        }
    }
}

fn capture(id: &str, seconds: u64) -> Result<(Vec<f32>, u32)> {
    capture_with_tick(id, seconds, |_, _, _| Ok(()))
}

fn capture_with_tick(
    id: &str,
    seconds: u64,
    tick: impl FnMut(&[f32], u32, Duration) -> Result<()>,
) -> Result<(Vec<f32>, u32)> {
    let stream = open_loopback_capture(&output_device_by_id(id)?)?;
    capture_stream_with_tick(stream, seconds, tick)
}

fn capture_stream_with_tick(
    stream: AudioCaptureStream,
    seconds: u64,
    mut tick: impl FnMut(&[f32], u32, Duration) -> Result<()>,
) -> Result<(Vec<f32>, u32)> {
    let (_, rate, bytes, float) = layout(&stream.format)?;
    unsafe { stream.client.Start() }?;
    let started = Instant::now();
    let end = started + Duration::from_secs(seconds);
    let mut samples = Vec::new();
    while Instant::now() < end {
        tick(&samples, rate, started.elapsed())?;
        if unsafe { stream.capture.GetNextPacketSize() }? == 0 {
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        let mut pointer = std::ptr::null_mut();
        let mut frames = 0;
        let mut flags = 0;
        unsafe {
            stream.capture.GetBuffer(&raw mut pointer, &raw mut frames, &raw mut flags, None, None)
        }?;
        if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
            samples.resize(samples.len() + frames as usize, 0.0);
        } else {
            let data = unsafe {
                slice::from_raw_parts(pointer, frames as usize * stream.format.block_align)
            };
            for frame in data.chunks_exact(stream.format.block_align) {
                let sample = if float {
                    f32::from_le_bytes(frame[..4].try_into()?)
                } else {
                    let mut value = if frame[bytes - 1] & 0x80 == 0 { [0; 4] } else { [0xff; 4] };
                    value[..bytes].copy_from_slice(&frame[..bytes]);
                    i32::from_le_bytes(value) as f32 / (1_u64 << (bytes * 8 - 1)) as f32
                };
                samples.push(sample);
            }
        }
        unsafe { stream.capture.ReleaseBuffer(frames) }?;
    }
    if samples.len() < rate as usize {
        bail!(
            "Less than one second captured: {} samples, rate {rate}. Start the guest Voicemeeter engine.",
            samples.len()
        );
    }
    Ok((samples, rate))
}

fn capture_recording(id: &str, seconds: u64) -> Result<(Vec<f32>, u32)> {
    let device = output_device_by_id(id)?;
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }?;
    let format = WaveFormatOwned::from_mix_format(&client)?;
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            RENDER_STREAM_FLAGS,
            0,
            0,
            format.as_ptr(),
            None,
        )
    }?;
    let capture = unsafe { client.GetService::<IAudioCaptureClient>() }?;
    capture_stream_with_tick(AudioCaptureStream { client, capture, format }, seconds, |_, _, _| {
        Ok(())
    })
}

struct ControlledRoute {
    control: Arc<RouteControl>,
    thread: Option<std::thread::JoinHandle<Result<()>>>,
}
impl Drop for ControlledRoute {
    fn drop(&mut self) {
        self.control.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
#[ignore = "VM-only; records a virtual cable and live software attenuation"]
fn live_cable_gain_acceptance() -> Result<()> {
    if std::env::var("DESKTOP_AUDIO_VM_PROBE").as_deref() != Ok("1") {
        bail!("Explicit VM probe opt-in required");
    }
    let guard = command("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", "$c=Get-CimInstance Win32_ComputerSystem; if (($c.Model -notmatch 'Virtual Machine|VirtualBox|VMware|KVM|QEMU|HVM domU|Parallels') -and ($c.Manufacturer -notmatch '^QEMU$|^VMware|^innotek GmbH$')) {exit 1}"]).status()?;
    if !guard.success() {
        bail!("Refusing cable probe outside a VM");
    }
    let _com = ComGuard::new()?;
    let backend = WindowsAudioBackend;
    let endpoints = backend.endpoints()?;
    let cables: Vec<_> = backend
        .installed_cables(&endpoints)
        .into_iter()
        .filter(|c| c.name == "CABLE Input (VB-Audio Virtual Cable)")
        .collect();
    let cable = match cables.as_slice() {
        [cable] => cable.clone(),
        _ => bail!("Missing or ambiguous cable fixture"),
    };
    let targets: Vec<_> = endpoints
        .iter()
        .filter(|e| {
            e.flow == EndpointFlow::Playback
                && e.name == "Voicemeeter AUX Input (VB-Audio Voicemeeter VAIO)"
        })
        .collect();
    let target = match targets.as_slice() {
        [target] => target.id.clone(),
        _ => bail!("Missing or ambiguous monitor fixture"),
    };
    let root =
        std::path::PathBuf::from(std::env::var("DESKTOP_AUDIO_PROBE_OUTPUT")?).join("cable-gain");
    std::fs::create_dir_all(&root)?;
    let _source = Tone::start(cable.playback_id.clone(), 440.0)?;
    let _clock = Tone::start(target.clone(), 0.0)?;
    let (raw, rate) = capture_recording(&cable.capture_id, 2)?;
    let rms = |samples: &[f32]| -> f64 {
        (samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / samples.len() as f64).sqrt()
    };
    let baseline = rms(&raw[rate as usize / 4..]);
    if baseline < 0.05 || raw.iter().any(|s| !s.is_finite()) {
        bail!("Virtual cable playback -> recording did not carry the fixture tone");
    }
    save_wave(&root.join("cable-recording.wav"), &raw, rate)?;
    let control = Arc::new(RouteControl::new(-20.0, false)?);
    let monitor = control.clone();
    let selected = cable.clone();
    let output = target.clone();
    let mut route = ControlledRoute {
        control: control.clone(),
        thread: Some(std::thread::spawn(move || backend.monitor(&selected, &output, monitor))),
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while control.state() != RouteState::Running {
        if route.thread.as_ref().is_some_and(|t| t.is_finished()) {
            route
                .thread
                .take()
                .context("Route missing")?
                .join()
                .map_err(|_| anyhow::anyhow!("Route panicked"))??;
            bail!("Route ended before starting");
        }
        if Instant::now() >= deadline {
            bail!("Controlled route did not start");
        }
        thread::sleep(Duration::from_millis(10));
    }
    for db in [-20.0, -40.0, -60.0, -96.0, -30.0] {
        control.set_gain(db)?;
        // The fixture driver can retain old samples beyond the DSP's 20 ms ramp.
        // Use the same two-second endpoint settling bound as the switch probe.
        thread::sleep(Duration::from_secs(2));
        let (samples, rate) = capture(&target, 2)?;
        if samples.iter().any(|s| !s.is_finite()) {
            bail!("Nonfinite monitor samples");
        }
        let measured = rms(&samples[rate as usize / 4..]);
        let measured_db = 20.0 * (measured / baseline).log10();
        let report =
            format!("{{\"gain_db\":{db},\"measured_db\":{measured_db},\"rms\":{measured}}}");
        println!("CABLE_GAIN_METRIC {report}");
        std::fs::write(root.join(format!("gain-{}.json", -db)), report)?;
        save_wave(&root.join(format!("gain-{}.wav", -db)), &samples, rate)?;
        if !measured_db.is_finite() || (measured_db - f64::from(db)).abs() > 1.0 {
            bail!("Gain mismatch: requested {db}, measured {measured_db}");
        }
        if route.thread.as_ref().is_some_and(|t| t.is_finished()) {
            bail!("Gain adjustment restarted/ended the monitor");
        }
    }
    control.set_mute(true);
    thread::sleep(Duration::from_secs(2));
    let (muted, rate) = capture(&target, 2)?;
    if rms(&muted[rate as usize / 4..]) > 0.000_001 {
        bail!("Mute did not reach silence");
    }
    control.set_mute(false);
    thread::sleep(Duration::from_secs(2));
    let (resumed, rate) = capture(&target, 2)?;
    if (20.0 * (rms(&resumed[rate as usize / 4..]) / baseline).log10() + 30.0).abs() > 1.0 {
        bail!("Unmute did not preserve attenuation");
    }
    let (unaltered, rate) = capture_recording(&cable.capture_id, 2)?;
    if (rms(&unaltered[rate as usize / 4..]) / baseline - 1.0).abs() > 0.1 {
        bail!("Monitor attenuation changed the cable recording endpoint");
    }
    control.stop();
    route
        .thread
        .take()
        .context("Route missing")?
        .join()
        .map_err(|_| anyhow::anyhow!("Route panicked"))??;
    if control.state() != RouteState::Stopped {
        bail!("Monitor did not stop");
    }
    println!(
        "PASS: cable playback -> recording, live -20/-40/-60/-96 dB gain, mute/unmute, unchanged recording and owned monitor shutdown; evidence {}",
        root.display()
    );
    Ok(())
}

fn amplitude(samples: &[f32], rate: u32, frequency: f64) -> f64 {
    let mut real = 0.0;
    let mut imaginary = 0.0;
    for (index, sample) in samples.iter().enumerate() {
        let phase = TAU * frequency * index as f64 / f64::from(rate);
        real += f64::from(*sample) * phase.cos();
        imaginary += f64::from(*sample) * phase.sin();
    }
    2.0 * real.hypot(imaginary) / samples.len() as f64
}

fn save_wave(path: &Path, samples: &[f32], rate: u32) -> Result<()> {
    let size = u32::try_from(samples.len() * 4)?;
    let mut wave = Vec::with_capacity(size as usize + 44);
    wave.extend(b"RIFF");
    wave.extend((size + 36).to_le_bytes());
    wave.extend(b"WAVEfmt ");
    wave.extend(16_u32.to_le_bytes());
    wave.extend(3_u16.to_le_bytes());
    wave.extend(1_u16.to_le_bytes());
    wave.extend(rate.to_le_bytes());
    wave.extend((rate * 4).to_le_bytes());
    wave.extend(4_u16.to_le_bytes());
    wave.extend(32_u16.to_le_bytes());
    wave.extend(b"data");
    wave.extend(size.to_le_bytes());
    for sample in samples {
        wave.extend(sample.to_le_bytes());
    }
    std::fs::write(path, wave)?;
    Ok(())
}

struct Routing {
    child: Child,
}
impl Drop for Routing {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
struct DefaultRestore {
    tool: String,
    original: String,
}
impl DefaultRestore {
    fn set(&self, id: &str) -> Result<()> {
        if !command(&self.tool).args(["/SetDefault", id, "0"]).status()?.success() {
            bail!("Default helper failed");
        }
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if list_output_devices()?.iter().any(|device| device.id == id && device.is_default) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
        bail!("Requested default did not become active")
    }
}
impl Drop for DefaultRestore {
    fn drop(&mut self) {
        if let Err(error) = self.set(&self.original) {
            eprintln!("DEFAULT RESTORE FAILED: {error:#}");
        }
    }
}

fn check(root: &Path, label: &str, id: &str, expected: f64, unwanted: f64) -> Result<()> {
    thread::sleep(Duration::from_secs(2)); // Exclude the bounded reconnect/buffer drain period.
    let (samples, rate) = capture(id, 2)?;
    if samples.iter().any(|sample| !sample.is_finite()) {
        bail!("{label}: nonfinite captured samples");
    }
    save_wave(&root.join(format!("{label}.wav")), &samples, rate)?;
    // Preserve the entire recording, including driver pre-roll, as evidence.
    // Every new loopback stream warms up for 250 ms before steady-state analysis.
    let samples = &samples[rate as usize / 4..];
    let wanted = amplitude(&samples, rate, expected);
    let leaked = amplitude(&samples, rate, unwanted);
    let rms = (samples.iter().map(|sample| f64::from(*sample).powi(2)).sum::<f64>()
        / samples.len() as f64)
        .sqrt();
    let report = format!(
        "{{\"case\":\"{label}\",\"rate\":{rate},\"samples\":{},\"expected_hz\":{expected},\"unwanted_hz\":{unwanted},\"expected_amplitude\":{wanted},\"unwanted_amplitude\":{leaked},\"rms\":{rms}}}",
        samples.len()
    );
    std::fs::write(root.join(format!("{label}.json")), &report)?;
    println!("{report}");
    for quarter in 0..4 {
        let segment = &samples[samples.len() * quarter / 4..samples.len() * (quarter + 1) / 4];
        println!(
            "{label} quarter {quarter}: expected={} unwanted={}",
            amplitude(segment, rate, expected),
            amplitude(segment, rate, unwanted)
        );
    }
    if wanted < 0.03 || leaked > wanted * 0.01 {
        bail!("{label}: absent expected tone or leaked old source exceeds -40 dB");
    }
    Ok(())
}

fn check_silent(root: &Path, label: &str, id: &str) -> Result<()> {
    thread::sleep(Duration::from_secs(2));
    let (samples, rate) = capture(id, 2)?;
    if samples.iter().any(|sample| !sample.is_finite()) {
        bail!("{label}: nonfinite captured samples");
    }
    save_wave(&root.join(format!("{label}.wav")), &samples, rate)?;
    for quarter in 0..4 {
        let segment = &samples[samples.len() * quarter / 4..samples.len() * (quarter + 1) / 4];
        let rms = (segment.iter().map(|sample| f64::from(*sample).powi(2)).sum::<f64>()
            / segment.len() as f64)
            .sqrt();
        println!("{label} raw quarter {quarter}: rms={rms}");
    }
    let samples = &samples[rate as usize / 4..];
    let rms = (samples.iter().map(|sample| f64::from(*sample).powi(2)).sum::<f64>()
        / samples.len() as f64)
        .sqrt();
    let report = format!("{{\"case\":\"{label}\",\"rms\":{rms}}}");
    std::fs::write(root.join(format!("{label}.json")), &report)?;
    println!("{report}");
    if rms > 0.001 {
        bail!("{label}: output was not silent");
    }
    Ok(())
}

fn check_continuous_switch(root: &Path, c: &str, b: &str, restore: &DefaultRestore) -> Result<()> {
    let mut switch_frame = None;
    let (samples, rate) = capture_with_tick(c, 6, |samples, _, elapsed| {
        if switch_frame.is_none() && elapsed >= Duration::from_secs(2) {
            switch_frame = Some(samples.len());
            restore.set(b)?;
        }
        Ok(())
    })?;
    if samples.iter().any(|sample| !sample.is_finite()) {
        bail!("Continuous capture contains nonfinite samples");
    }
    let switch_frame = switch_frame.context("Continuous default change did not run")?;
    if switch_frame <= rate as usize / 4 || samples.len() < switch_frame + rate as usize * 2 {
        bail!("Continuous recording too short");
    }
    save_wave(&root.join("continuous-a-to-b.wav"), &samples, rate)?;
    let before = &samples[rate as usize / 4..switch_frame];
    // Same capture client remains running across the switch. Bound settling to
    // one second, rather than hiding a source leak behind capture startup.
    let after = &samples[switch_frame + rate as usize..];
    let before_wanted = amplitude(before, rate, 440.0);
    let before_unwanted = amplitude(before, rate, 880.0);
    let wanted = amplitude(after, rate, 880.0);
    let unwanted = amplitude(after, rate, 440.0);
    let rms = (after.iter().map(|sample| f64::from(*sample).powi(2)).sum::<f64>()
        / after.len() as f64)
        .sqrt();
    let report = format!(
        "{{\"case\":\"continuous-a-to-b\",\"rate\":{rate},\"switch_frame\":{switch_frame},\"settling_frames\":{rate},\"before_expected_amplitude\":{before_wanted},\"before_unwanted_amplitude\":{before_unwanted},\"expected_hz\":880,\"unwanted_hz\":440,\"expected_amplitude\":{wanted},\"unwanted_amplitude\":{unwanted},\"rms\":{rms}}}"
    );
    std::fs::write(root.join("continuous-a-to-b.json"), &report)?;
    println!("{report}");
    if before_wanted < 0.03
        || before_unwanted > before_wanted * 0.01
        || wanted < 0.03
        || unwanted > wanted * 0.01
    {
        bail!("Continuous A -> B signal attribution failed");
    }
    Ok(())
}

#[test]
fn spectral_probe_distinguishes_known_tones_and_detects_leakage() {
    let rate = 48_000;
    let samples = (0..rate)
        .map(|index| {
            (0.15 * (TAU * 440.0 * f64::from(index) / f64::from(rate)).sin()
                + 0.003 * (TAU * 880.0 * f64::from(index) / f64::from(rate)).sin())
                as f32
        })
        .collect::<Vec<_>>();
    let wanted = amplitude(&samples, rate, 440.0);
    let leaked = amplitude(&samples, rate, 880.0);
    assert!((wanted - 0.15).abs() < 0.000_001);
    assert!((leaked - 0.003).abs() < 0.000_001);
    assert!(leaked > wanted * 0.01);
    assert!(amplitude(&samples, rate, 1320.0) < 0.000_001);
}

#[test]
#[ignore = "VM-only; plays fixture tones and changes/restores the guest default"]
fn live_default_switch_signal_acceptance() -> Result<()> {
    if std::env::var("DESKTOP_AUDIO_VM_PROBE").as_deref() != Ok("1") {
        bail!("Explicit VM probe opt-in required");
    }
    let guard = command("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", "$c=Get-CimInstance Win32_ComputerSystem; if (($c.Model -notmatch 'Virtual Machine|VirtualBox|VMware|KVM|QEMU|HVM domU|Parallels') -and ($c.Manufacturer -notmatch '^QEMU$|^VMware|^innotek GmbH$')) {exit 1}"]).status()?;
    if !guard.success() {
        bail!("Refusing signal probe outside a VM");
    }
    let _com = ComGuard::new()?;
    let devices = list_output_devices()?;
    let exact = |name: &str| -> Result<String> {
        let found =
            devices.iter().filter(|device| device.friendly_name == name).collect::<Vec<_>>();
        match found.as_slice() {
            [device] => Ok(device.id.clone()),
            _ => bail!("Missing or ambiguous fixture: {name}"),
        }
    };
    let a = exact("Voicemeeter Input (VB-Audio Voicemeeter VAIO)")?;
    let b = exact("Voicemeeter AUX Input (VB-Audio Voicemeeter VAIO)")?;
    let c = exact("CABLE Input (VB-Audio Virtual Cable)")?;
    let root = std::path::PathBuf::from(std::env::var("DESKTOP_AUDIO_PROBE_OUTPUT")?);
    std::fs::create_dir_all(&root)?;
    let restore = DefaultRestore {
        tool: std::env::var("DESKTOP_AUDIO_DEFAULT_HELPER")?,
        original: devices
            .iter()
            .find(|device| device.is_default)
            .context("Default missing")?
            .id
            .clone(),
    };
    let _tone_a = Tone::start(a.clone(), 440.0)?;
    let _tone_b = Tone::start(b.clone(), 880.0)?;
    let _silence_c = Tone::start(c.clone(), 0.0)?;
    check(&root, "source-a", &a, 440.0, 880.0)?;
    check(&root, "source-b", &b, 880.0, 440.0)?;
    check_silent(&root, "destination-baseline", &c)?;
    restore.set(&a)?;
    let router = std::env::var("DESKTOP_AUDIO_ROUTER")?;
    let mut routing = Routing {
        child: command(&router)
            .args(["route", "default", &c])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?,
    };
    check(&root, "default-a-to-c", &c, 440.0, 880.0)?;
    restore.set(&b)?;
    check(&root, "default-b-to-c", &c, 880.0, 440.0)?;
    restore.set(&a)?;
    check(&root, "default-back-a-to-c", &c, 440.0, 880.0)?;
    check_continuous_switch(&root, &c, &b, &restore)?;
    restore.set(&c)?;
    check_silent(&root, "feedback-paused", &c)?;
    restore.set(&b)?;
    check(&root, "feedback-resumed-b", &c, 880.0, 440.0)?;
    if routing.child.try_wait()?.is_some() {
        bail!("Router exited during default switching");
    }
    drop(routing);
    check_silent(&root, "router-stopped", &c)?;
    let _fixed = Routing {
        child: command(&router)
            .args(["route", &a, &c])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?,
    };
    restore.set(&b)?;
    check(&root, "fixed-a-default-b", &c, 440.0, 880.0)?;
    drop(_fixed);
    let _tone_c = Tone::start(c.clone(), 1320.0)?;
    restore.set(&a)?;
    let _target = Routing {
        child: command(&router)
            .args(["route", &c, "default"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?,
    };
    check(&root, "default-target-a", &a, 1320.0, 880.0)?;
    restore.set(&b)?;
    check(&root, "default-target-b", &b, 1320.0, 440.0)?;
    check(&root, "old-target-a-no-longer-routed", &a, 440.0, 1320.0)?;
    restore.set(&restore.original)?;
    println!("PASS: recorded signal acceptance; evidence {}", root.display());
    Ok(())
}
