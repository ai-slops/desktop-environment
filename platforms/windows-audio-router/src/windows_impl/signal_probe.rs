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
    let stream = open_loopback_capture(&output_device_by_id(id)?)?;
    let (_, rate, bytes, float) = layout(&stream.format)?;
    unsafe { stream.client.Start() }?;
    let end = Instant::now() + Duration::from_secs(seconds);
    let mut samples = Vec::new();
    while Instant::now() < end {
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
    save_wave(&root.join(format!("{label}.wav")), &samples, rate)?;
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
    if wanted < 0.03 || leaked > wanted * 0.01 {
        bail!("{label}: absent expected tone or leaked old source exceeds -40 dB");
    }
    Ok(())
}

fn check_silent(root: &Path, label: &str, id: &str) -> Result<()> {
    thread::sleep(Duration::from_secs(2));
    let (samples, rate) = capture(id, 2)?;
    save_wave(&root.join(format!("{label}.wav")), &samples, rate)?;
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
