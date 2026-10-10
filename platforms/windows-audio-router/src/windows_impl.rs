use anyhow::{Context, Result, bail};
use std::slice;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::thread;
use std::time::Duration;
use tracing::{debug, info, warn};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, DEVICE_STATE_ACTIVE,
    IAudioCaptureClient, IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceCollection,
    IMMDeviceEnumerator, IMMNotificationClient, MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::PWSTR;

mod default_endpoint;
use default_endpoint::DefaultEndpointNotification;

const RENDER_STREAM_FLAGS: u32 =
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
const CAPTURE_STREAM_FLAGS: u32 = AUDCLNT_STREAMFLAGS_LOOPBACK
    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;

#[derive(Debug, Clone)]
pub struct AudioOutputDevice {
    pub id: String,
    pub friendly_name: String,
    pub is_default: bool,
}

pub fn list_output_devices() -> Result<Vec<AudioOutputDevice>> {
    let _com = ComGuard::new()?;
    let enumerator = device_enumerator()?;
    let default_id = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
        .ok()
        .and_then(|device| endpoint_id(&device).ok());
    let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }?;
    read_output_devices(&collection, default_id.as_deref())
}

const DEVICE_RECOVERY_RETRY_DELAY: Duration = Duration::from_millis(500);

pub fn run_output_audio_router(source_selector: &str, target_selector: &str) -> Result<()> {
    let _com = ComGuard::new()?;
    let watch = if source_selector.eq_ignore_ascii_case("default")
        || target_selector.eq_ignore_ascii_case("default")
    {
        Some(DefaultEndpointWatch::new()?)
    } else {
        None
    };
    let mut started_once = false;

    loop {
        let generation = watch.as_ref().map_or(0, DefaultEndpointWatch::generation);
        match run_audio_session(source_selector, target_selector, &mut started_once, watch.as_ref())
        {
            Ok(()) => info!("Default audio output changed; reconnecting to the current endpoints"),
            Err(error)
                if started_once
                    || watch.as_ref().is_some_and(|watch| watch.generation() != generation) =>
            {
                warn!(
                    "Audio routing interrupted, likely due to a device change; waiting to reconnect: {error:#}"
                );
                thread::sleep(DEVICE_RECOVERY_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
}

fn run_audio_session(
    source_selector: &str,
    target_selector: &str,
    started_once: &mut bool,
    watch: Option<&DefaultEndpointWatch>,
) -> Result<()> {
    // Snapshot before resolving either selector so changes during setup cannot be lost.
    let generation = watch.map_or(0, DefaultEndpointWatch::generation);
    let changed = || watch.is_some_and(|watch| watch.generation() != generation);
    let source = select_output_device(source_selector)
        .with_context(|| format!("failed to resolve source device '{source_selector}'"))?;
    let target = select_output_device(target_selector)
        .with_context(|| format!("failed to resolve target device '{target_selector}'"))?;
    if changed() {
        return Ok(());
    }
    if source.id == target.id {
        bail!("Source and target must be different output devices (audio feedback prevention)");
    }

    info!("Cloning audio from {} to {}", source.friendly_name, target.friendly_name);
    debug!("Source device id={}", source.id);
    debug!("Target device id={}", target.id);

    let source_device = output_device_by_id(&source.id)?;
    let target_device = output_device_by_id(&target.id)?;

    let capture_stream = open_loopback_capture(&source_device)
        .with_context(|| format!("failed to open loopback capture on {}", source.friendly_name))?;
    debug!("Source loopback mix format={}", capture_stream.format.describe());

    let render_stream = open_render_client(&target_device, &capture_stream.format)
        .with_context(|| format!("failed to open render client on {}", target.friendly_name))?;
    debug!("Target render initialized with format={}", render_stream.format.describe());

    if changed() {
        return Ok(());
    }
    unsafe { render_stream.client.Start() }.context("failed to start render client")?;
    unsafe { capture_stream.client.Start() }.context("failed to start capture client")?;
    debug!("Started source capture and target render streams");
    if *started_once {
        info!("Audio routing resumed from {} to {}", source.friendly_name, target.friendly_name);
    } else {
        *started_once = true;
    }

    loop {
        if pump_audio(&capture_stream, &render_stream, &changed)? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(3));
    }
}

fn pump_audio(
    capture: &AudioCaptureStream,
    render: &AudioRenderStream,
    changed: &impl Fn() -> bool,
) -> Result<bool> {
    loop {
        if changed() {
            return Ok(true);
        }
        let packet_frames = unsafe { capture.capture.GetNextPacketSize() }?;
        if packet_frames == 0 {
            return Ok(false);
        }

        let mut data = std::ptr::null_mut();
        let mut frames = 0;
        let mut flags = 0;
        unsafe {
            capture.capture.GetBuffer(
                &raw mut data,
                &raw mut frames,
                &raw mut flags,
                None,
                None,
            )?;
        }

        let silent = (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
        let mut frame_offset = 0_u32;
        while frame_offset < frames {
            // Also interrupt a full render buffer; otherwise a default change could
            // leave us stuck waiting for the old endpoint to accept more audio.
            if changed() {
                unsafe { capture.capture.ReleaseBuffer(frames) }?;
                return Ok(true);
            }
            let padding = unsafe { render.client.GetCurrentPadding() }?;
            let capacity = render.buffer_frames.saturating_sub(padding);
            if capacity == 0 {
                thread::sleep(Duration::from_millis(2));
                continue;
            }

            let frames_to_write = capacity.min(frames - frame_offset);
            let render_ptr = unsafe { render.render.GetBuffer(frames_to_write) }?;
            if silent {
                unsafe {
                    render
                        .render
                        .ReleaseBuffer(frames_to_write, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
                };
            } else {
                let bytes = frames_to_write as usize * render.format.block_align;
                let src_offset = frame_offset as usize * render.format.block_align;
                let src_ptr = unsafe { data.add(src_offset) };
                unsafe {
                    std::ptr::copy_nonoverlapping(src_ptr, render_ptr, bytes);
                    render.render.ReleaseBuffer(frames_to_write, 0)?;
                }
            }

            frame_offset += frames_to_write;
        }

        unsafe { capture.capture.ReleaseBuffer(frames) }?;
    }
}

struct AudioRenderStream {
    client: IAudioClient,
    render: IAudioRenderClient,
    buffer_frames: u32,
    format: WaveFormatOwned,
}

struct AudioCaptureStream {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    format: WaveFormatOwned,
}

fn open_loopback_capture(device: &IMMDevice) -> Result<AudioCaptureStream> {
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }?;
    let format = WaveFormatOwned::from_mix_format(&client)?;

    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            CAPTURE_STREAM_FLAGS,
            0,
            0,
            format.as_ptr(),
            None,
        )
    }
    .with_context(|| {
        format!("capture Initialize failed with source loopback format {}", format.describe())
    })?;

    let capture = unsafe { client.GetService::<IAudioCaptureClient>() }?;
    Ok(AudioCaptureStream { client, capture, format })
}

fn open_render_client(device: &IMMDevice, format: &WaveFormatOwned) -> Result<AudioRenderStream> {
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }?;

    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            RENDER_STREAM_FLAGS,
            0,
            0,
            format.as_ptr(),
            None,
        )
    }
    .with_context(|| format!("render Initialize failed with format {}", format.describe()))?;

    let buffer_frames = unsafe { client.GetBufferSize() }?;
    let render = unsafe { client.GetService::<IAudioRenderClient>() }?;
    Ok(AudioRenderStream { client, render, buffer_frames, format: format.clone() })
}

struct DefaultEndpointWatch {
    enumerator: IMMDeviceEnumerator,
    callback: IMMNotificationClient,
    generation: Arc<AtomicU64>,
}

impl DefaultEndpointWatch {
    fn new() -> Result<Self> {
        let enumerator = device_enumerator()?;
        let generation = Arc::new(AtomicU64::new(0));
        let callback: IMMNotificationClient =
            DefaultEndpointNotification { generation: generation.clone() }.into();
        unsafe { enumerator.RegisterEndpointNotificationCallback(&callback) }
            .context("failed to watch default audio output changes")?;
        Ok(Self { enumerator, callback, generation })
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }
}

impl Drop for DefaultEndpointWatch {
    fn drop(&mut self) {
        let _ = unsafe { self.enumerator.UnregisterEndpointNotificationCallback(&self.callback) };
    }
}

impl Drop for AudioCaptureStream {
    fn drop(&mut self) {
        let _ = unsafe { self.client.Stop() };
    }
}

impl Drop for AudioRenderStream {
    fn drop(&mut self) {
        let _ = unsafe { self.client.Stop() };
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WaveFormatOwned {
    bytes: Vec<u8>,
    block_align: usize,
}

impl WaveFormatOwned {
    fn from_mix_format(client: &IAudioClient) -> Result<Self> {
        let raw = unsafe { client.GetMixFormat() }?;
        let format = unsafe { wave_format_from_ptr(raw) };
        unsafe { CoTaskMemFree(Some(raw.cast())) };
        format
    }

    const fn as_ptr(&self) -> *const windows::Win32::Media::Audio::WAVEFORMATEX {
        self.bytes.as_ptr().cast()
    }

    fn describe(&self) -> String {
        let format_tag = u16::from_le_bytes([self.bytes[0], self.bytes[1]]);
        let channels = u16::from_le_bytes([self.bytes[2], self.bytes[3]]);
        let rate = u32::from_le_bytes([self.bytes[4], self.bytes[5], self.bytes[6], self.bytes[7]]);
        let block_align = u16::from_le_bytes([self.bytes[12], self.bytes[13]]);
        let bits = u16::from_le_bytes([self.bytes[14], self.bytes[15]]);
        let extra = u16::from_le_bytes([self.bytes[16], self.bytes[17]]);
        format!(
            "tag={format_tag} channels={channels} rate={rate} bits={bits} block_align={block_align} extra={extra}"
        )
    }
}

unsafe fn wave_format_from_ptr(
    raw: *mut windows::Win32::Media::Audio::WAVEFORMATEX,
) -> Result<WaveFormatOwned> {
    if raw.is_null() {
        bail!("IAudioClient::GetMixFormat returned a null format pointer");
    }

    let block_align = unsafe { (*raw).nBlockAlign } as usize;
    let total = std::mem::size_of::<windows::Win32::Media::Audio::WAVEFORMATEX>()
        + unsafe { (*raw).cbSize } as usize;
    let bytes = unsafe { slice::from_raw_parts(raw.cast::<u8>(), total) }.to_vec();
    Ok(WaveFormatOwned { bytes, block_align })
}

fn select_output_device(selector: &str) -> Result<AudioOutputDevice> {
    resolve_output_device(list_output_devices()?, selector)
}

fn resolve_output_device(
    devices: Vec<AudioOutputDevice>,
    selector: &str,
) -> Result<AudioOutputDevice> {
    if selector.trim().is_empty() {
        bail!("Device selector must not be empty");
    }
    if let Some(device) = devices.iter().find(|device| device.id.eq_ignore_ascii_case(selector)) {
        return Ok(device.clone());
    }
    if selector.eq_ignore_ascii_case("default") {
        return devices
            .into_iter()
            .find(|device| device.is_default)
            .context("no default audio output device found");
    }

    let selector_lower = selector.to_ascii_lowercase();
    let matches: Vec<_> = devices
        .into_iter()
        .filter(|device| {
            device.friendly_name.to_ascii_lowercase().contains(&selector_lower)
                || device.id.to_ascii_lowercase().contains(&selector_lower)
        })
        .collect();
    match matches.as_slice() {
        [device] => Ok(device.clone()),
        [] => bail!("no audio output matched '{selector}'"),
        _ => bail!(
            "Multiple outputs match '{selector}'; use a full device ID from list-audio-devices"
        ),
    }
}

fn output_device_by_id(id: &str) -> Result<IMMDevice> {
    let enumerator = device_enumerator()?;
    let id_wide = wide_null(id);
    unsafe { enumerator.GetDevice(windows::core::PCWSTR(id_wide.as_ptr())) }.map_err(Into::into)
}

fn device_enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(Into::into)
}

fn read_output_devices(
    collection: &IMMDeviceCollection,
    default_id: Option<&str>,
) -> Result<Vec<AudioOutputDevice>> {
    let count = unsafe { collection.GetCount() }?;
    let mut devices = Vec::with_capacity(count as usize);
    for index in 0..count {
        let device = unsafe { collection.Item(index) }?;
        let id = endpoint_id(&device)?;
        devices.push(AudioOutputDevice {
            friendly_name: device_friendly_name(&device).unwrap_or_else(|error| {
                warn!(device_id = %id, %error, "Device name unavailable; use its ID to select it");
                format!("Unnamed output ({id})")
            }),
            is_default: Some(id.as_str()) == default_id,
            id,
        });
    }
    Ok(devices)
}

fn endpoint_id(device: &IMMDevice) -> Result<String> {
    let id = unsafe { device.GetId() }?;
    let string = pwstr_to_string(id);
    unsafe { CoTaskMemFree(Some(id.0.cast())) };
    string
}

fn device_friendly_name(device: &IMMDevice) -> Result<String> {
    let store: IPropertyStore = unsafe { device.OpenPropertyStore(STGM_READ) }
        .context("failed to open device property store")?;
    let mut value = unsafe { store.GetValue(std::ptr::from_ref(&PKEY_Device_FriendlyName)) }
        .context("failed to read device friendly name")?;
    // GetValue owns the variant; clear it even when string conversion fails.
    let text = unsafe { PropVariantToStringAlloc(&raw const value) };
    let clear_result = unsafe { PropVariantClear(&raw mut value) };
    let text = text?;
    let friendly_name = pwstr_to_string(text);
    // PropVariantToStringAlloc returns a separate COM allocation.
    unsafe { CoTaskMemFree(Some(text.0.cast())) };
    clear_result?;
    friendly_name
}

fn pwstr_to_string(text: PWSTR) -> Result<String> {
    if text.is_null() {
        bail!("received null wide string");
    }

    let mut len = 0;
    unsafe {
        while *text.0.add(len) != 0 {
            len += 1;
        }
        Ok(String::from_utf16_lossy(slice::from_raw_parts(text.0, len)))
    }
}

fn wide_null(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

struct ComGuard;

impl ComGuard {
    fn new() -> Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioOutputDevice, DefaultEndpointNotification, resolve_output_device};
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    use windows::Win32::Media::Audio::{
        DEVICE_STATE_ACTIVE, IMMNotificationClient, eCapture, eCommunications, eConsole,
        eMultimedia, eRender,
    };
    use windows::core::PCWSTR;

    fn devices() -> Vec<AudioOutputDevice> {
        vec![
            AudioOutputDevice {
                id: "output-a".into(),
                friendly_name: "Monitor audio".into(),
                is_default: true,
            },
            AudioOutputDevice {
                id: "output-b".into(),
                friendly_name: "Monitor audio".into(),
                is_default: false,
            },
            AudioOutputDevice {
                id: "output-c".into(),
                friendly_name: "Headphones".into(),
                is_default: false,
            },
        ]
    }

    #[test]
    fn rejects_ambiguous_or_empty_selectors() {
        for selector in ["Monitor", "output", "", "  ", "missing"] {
            assert!(resolve_output_device(devices(), selector).is_err());
        }
    }

    #[test]
    fn resolves_id_default_and_unique_name() -> anyhow::Result<()> {
        for (selector, id) in
            [("OUTPUT-B", "output-b"), ("DEFAULT", "output-a"), ("head", "output-c")]
        {
            assert_eq!(resolve_output_device(devices(), selector)?.id, id);
        }
        Ok(())
    }

    #[test]
    fn reports_missing_default() {
        let mut outputs = devices();
        for output in &mut outputs {
            output.is_default = false;
        }
        assert!(resolve_output_device(outputs, "default").is_err());
    }

    // These callbacks are invoked on a local COM object only. No endpoint is
    // registered, opened, played, or changed by the regression tests.
    fn notification() -> (IMMNotificationClient, Arc<AtomicU64>) {
        let generation = Arc::new(AtomicU64::new(0));
        let callback = DefaultEndpointNotification { generation: generation.clone() }.into();
        (callback, generation)
    }

    #[test]
    fn only_render_console_default_changes_request_reconnection() -> anyhow::Result<()> {
        let (callback, generation) = notification();
        let endpoint = windows::core::w!("output-b");
        unsafe {
            callback.OnDefaultDeviceChanged(eCapture, eConsole, endpoint)?;
            callback.OnDefaultDeviceChanged(eRender, eCommunications, endpoint)?;
            callback.OnDefaultDeviceChanged(eRender, eMultimedia, endpoint)?;
            callback.OnDeviceAdded(endpoint)?;
            callback.OnDeviceRemoved(endpoint)?;
            callback.OnDeviceStateChanged(endpoint, DEVICE_STATE_ACTIVE)?;
        }
        assert_eq!(generation.load(Ordering::Relaxed), 0);
        unsafe {
            callback.OnDefaultDeviceChanged(eRender, eConsole, endpoint)?;
        }
        assert_eq!(generation.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[test]
    fn default_removal_and_changes_during_reconnection_are_not_lost() -> anyhow::Result<()> {
        let (callback, generation) = notification();
        let original_session = generation.load(Ordering::Relaxed);
        unsafe {
            callback.OnDefaultDeviceChanged(eRender, eConsole, PCWSTR::null())?;
        }
        assert_ne!(generation.load(Ordering::Relaxed), original_session);
        let reconnecting_session = generation.load(Ordering::Relaxed);
        unsafe {
            callback.OnDefaultDeviceChanged(eRender, eConsole, windows::core::w!("output-b"))?;
            callback.OnDefaultDeviceChanged(eRender, eConsole, windows::core::w!("output-a"))?;
        }
        assert_ne!(generation.load(Ordering::Relaxed), reconnecting_session);
        assert_eq!(generation.load(Ordering::Relaxed), 3);
        Ok(())
    }

    #[test]
    fn switching_default_reresolves_source_and_target_but_keeps_fixed_ids() -> anyhow::Result<()> {
        let (callback, generation) = notification();
        let session_generation = generation.load(Ordering::Relaxed);
        let fixed_target = resolve_output_device(devices(), "output-c")?;
        assert_eq!(resolve_output_device(devices(), "default")?.id, "output-a");
        let mut outputs = devices();
        for output in &mut outputs {
            output.is_default = output.id == "output-b";
        }
        unsafe {
            callback.OnDefaultDeviceChanged(eRender, eConsole, windows::core::w!("output-b"))?;
        }
        assert_ne!(generation.load(Ordering::Relaxed), session_generation);
        assert_eq!(resolve_output_device(outputs.clone(), "default")?.id, "output-b");
        assert_eq!(resolve_output_device(outputs.clone(), "DEFAULT")?.id, "output-b");
        assert_eq!(resolve_output_device(outputs.clone(), "output-a")?.id, "output-a");
        assert_eq!(resolve_output_device(outputs, "output-c")?.id, fixed_target.id);
        Ok(())
    }

    #[test]
    fn default_switch_can_create_feedback_and_later_clear_it() -> anyhow::Result<()> {
        let mut outputs = devices();
        let target = resolve_output_device(outputs.clone(), "output-c")?;
        assert_ne!(resolve_output_device(outputs.clone(), "default")?.id, target.id);
        for output in &mut outputs {
            output.is_default = output.id == target.id;
        }
        assert_eq!(resolve_output_device(outputs.clone(), "default")?.id, target.id);
        for output in &mut outputs {
            output.is_default = output.id == "output-b";
        }
        assert_ne!(resolve_output_device(outputs, "default")?.id, target.id);
        Ok(())
    }
}
