use anyhow::{Context, Result, bail};
use audio_cable_core::{
    AudioBackend, Cable, Endpoint, EndpointFlow, GainRamp, RouteControl, RouteState,
};
use std::marker::PhantomData;
use std::rc::Rc;
use std::slice;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::thread;
use std::time::Duration;
use tracing::{debug, info, warn};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{HANDLE, RPC_E_CHANGED_MODE};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_DEVICE_IN_USE, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, DEVICE_STATE_ACTIVE, IAudioCaptureClient,
    IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceCollection, IMMDeviceEnumerator,
    IMMNotificationClient, MMDeviceEnumerator, eCapture, eConsole, eRender,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree, CoUninitialize, STGM_READ,
};
use windows::Win32::System::Threading::{
    AVRT_PRIORITY_HIGH, AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
    AvSetMmThreadPriority,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::{PWSTR, w};

mod default_endpoint;
mod installed_cables;
#[cfg(test)]
mod signal_probe;
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
    let _com = ComGuard::for_enumeration()?;
    let enumerator = device_enumerator()?;
    let default_id = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
        .ok()
        .and_then(|device| endpoint_id(&device).ok());
    let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }?;
    read_output_devices(&collection, default_id.as_deref())
}

pub struct WindowsAudioBackend;
impl AudioBackend for WindowsAudioBackend {
    fn endpoints(&self) -> Result<Vec<Endpoint>> {
        let _com = ComGuard::for_enumeration()?;
        let enumerator = device_enumerator()?;
        let mut endpoints = Vec::new();
        for (native_flow, flow) in
            [(eRender, EndpointFlow::Playback), (eCapture, EndpointFlow::Capture)]
        {
            let default_id = unsafe { enumerator.GetDefaultAudioEndpoint(native_flow, eConsole) }
                .ok()
                .and_then(|d| endpoint_id(&d).ok());
            let collection =
                unsafe { enumerator.EnumAudioEndpoints(native_flow, DEVICE_STATE_ACTIVE) }?;
            endpoints.extend(
                read_output_devices(&collection, default_id.as_deref())?.into_iter().map(|d| {
                    Endpoint { id: d.id, name: d.friendly_name, flow, is_default: d.is_default }
                }),
            );
        }
        Ok(endpoints)
    }
    fn installed_cables(&self, endpoints: &[Endpoint]) -> Vec<Cable> {
        installed_cables::discover(endpoints)
    }
    fn setup_links(&self) -> Vec<audio_cable_core::SetupLink> {
        installed_cables::setup_links()
    }
    fn monitor(&self, cable: &Cable, output_id: &str, control: Arc<RouteControl>) -> Result<()> {
        cable.validate(&self.endpoints()?)?;
        run_controlled_audio_router(&cable.playback_id, output_id, &control)
    }
}

const DEVICE_RECOVERY_RETRY_DELAY: Duration = Duration::from_millis(500);
// Shared-mode timer-driven streams need headroom for late scheduling under load.
// REFERENCE_TIME is in 100 ns units. Keep exclusive mode and engine periods unchanged.
const ROUTE_BUFFER_DURATION: i64 = 1_000_000; // 100 ms capacity.
const ROUTE_PREROLL_MS: u32 = 50;

/// MMCSS registration belongs to this thread and must be reverted on the same thread.
struct AudioScheduling(HANDLE);
impl AudioScheduling {
    fn new() -> Result<Self> {
        let mut index = 0;
        let handle = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &raw mut index) }
            .context("failed to register the audio routing thread with MMCSS")?;
        let scheduling = Self(handle);
        unsafe { AvSetMmThreadPriority(handle, AVRT_PRIORITY_HIGH) }
            .context("failed to set the audio routing thread's MMCSS priority")?;
        Ok(scheduling)
    }
}
impl Drop for AudioScheduling {
    fn drop(&mut self) {
        let _ = unsafe { AvRevertMmThreadCharacteristics(self.0) };
    }
}

pub fn run_output_audio_router(source_selector: &str, target_selector: &str) -> Result<()> {
    run_router(source_selector, target_selector, None)
}

/// Check endpoint selection and stream initialization without starting playback.
pub fn check_output_audio_route(source_selector: &str, target_selector: &str) -> Result<()> {
    let _com = ComGuard::new()?;
    let source = select_output_device(source_selector)?;
    let target = select_output_device(target_selector)?;
    if source.id == target.id {
        bail!("Source and target must be different output devices (audio feedback prevention)");
    }
    info!(
        "Checking audio route from {} to {} without playback",
        source.friendly_name, target.friendly_name
    );
    let source_device = output_device_by_id(&source.id)?;
    let target_device = output_device_by_id(&target.id)?;
    let capture = open_capture(&source_device, false)
        .with_context(|| format!("failed to open loopback capture on {}", source.friendly_name))?;
    info!("Source mix format: {}", capture.format.describe());
    let target_client: IAudioClient = unsafe { target_device.Activate(CLSCTX_ALL, None) }?;
    let target_format = WaveFormatOwned::from_mix_format(&target_client)?;
    info!("Target mix format: {}", target_format.describe());
    let _render = open_render_client(&target_device, &capture.format)
        .with_context(|| format!("failed to open render client on {}", target.friendly_name))?;
    info!("Audio route initialization passed; no playback was started");
    Ok(())
}

pub fn run_controlled_audio_router(
    source_selector: &str,
    target_selector: &str,
    control: &RouteControl,
) -> Result<()> {
    let result = run_router(source_selector, target_selector, Some(control));
    control.set_state(RouteState::Stopped);
    result
}

fn run_router(
    source_selector: &str,
    target_selector: &str,
    control: Option<&RouteControl>,
) -> Result<()> {
    let _com = ComGuard::new()?;
    // Register only the buffer-servicing thread, never raise the entire GUI/process priority.
    let _scheduling = match AudioScheduling::new() {
        Ok(scheduling) => Some(scheduling),
        Err(error) => {
            warn!("Audio scheduling protection unavailable: {error:#}");
            None
        }
    };
    let watch = if source_selector.eq_ignore_ascii_case("default")
        || target_selector.eq_ignore_ascii_case("default")
    {
        Some(DefaultEndpointWatch::new()?)
    } else {
        None
    };
    let mut started_once = false;

    loop {
        if control.is_some_and(RouteControl::is_stopped) {
            return Ok(());
        }
        let generation = watch.as_ref().map_or(0, DefaultEndpointWatch::generation);
        match run_audio_session(
            source_selector,
            target_selector,
            &mut started_once,
            watch.as_ref(),
            control,
        ) {
            Ok(()) => info!("Default audio output changed; reconnecting to the current endpoints"),
            Err(error)
                if started_once
                    || watch.as_ref().is_some_and(|watch| watch.generation() != generation) =>
            {
                if let Some(control) = control {
                    control.set_state(RouteState::Recovering);
                }
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
    control: Option<&RouteControl>,
) -> Result<()> {
    // Snapshot before resolving either selector so changes during setup cannot be lost.
    let generation = watch.map_or(0, DefaultEndpointWatch::generation);
    let changed = || {
        watch.is_some_and(|watch| watch.generation() != generation)
            || control.is_some_and(RouteControl::is_stopped)
    };
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

    let capture_stream = open_capture(&source_device, control.is_some())
        .with_context(|| format!("failed to open loopback capture on {}", source.friendly_name))?;
    debug!("Source loopback mix format={}", capture_stream.format.describe());

    let render_stream = open_render_client(&target_device, &capture_stream.format)
        .with_context(|| format!("failed to open render client on {}", target.friendly_name))?;
    debug!("Target render initialized with format={}", render_stream.format.describe());

    if changed() {
        return Ok(());
    }
    render_stream.prime()?;
    unsafe { render_stream.client.Start() }.context("failed to start render client")?;
    unsafe { capture_stream.client.Start() }.context("failed to start capture client")?;
    debug!("Started source capture and target render streams");
    if *started_once {
        info!("Audio routing resumed from {} to {}", source.friendly_name, target.friendly_name);
    } else {
        *started_once = true;
    }

    if let Some(control) = control {
        control.set_state(RouteState::Running);
    }
    let mut gain =
        control.map(|c| GainRamp::new(capture_stream.format.sample_rate(), c.target_gain()));

    loop {
        if pump_audio(&capture_stream, &render_stream, &changed, control, gain.as_mut())? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(3));
    }
}

fn pump_audio(
    capture: &AudioCaptureStream,
    render: &AudioRenderStream,
    changed: &impl Fn() -> bool,
    control: Option<&RouteControl>,
    mut gain: Option<&mut GainRamp>,
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
                    if let (Some(control), Some(gain)) = (control, gain.as_mut()) {
                        let samples = slice::from_raw_parts_mut(render_ptr, bytes);
                        gain.process_f32_bytes(
                            samples,
                            render.format.channels(),
                            control.target_gain(),
                        );
                    }
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

impl AudioRenderStream {
    fn prime(&self) -> Result<()> {
        // Actual queued frames, not merely a larger empty capacity, absorb scheduler jitter.
        let frames = (self.format.sample_rate() / 1000 * ROUTE_PREROLL_MS).min(self.buffer_frames);
        if frames != 0 {
            unsafe {
                self.render.GetBuffer(frames)?;
                self.render.ReleaseBuffer(frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
            }
        }
        Ok(())
    }
}

struct AudioCaptureStream {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    format: WaveFormatOwned,
}

#[cfg(test)]
fn open_loopback_capture(device: &IMMDevice) -> Result<AudioCaptureStream> {
    open_capture(device, false)
}

fn open_capture(device: &IMMDevice, float_samples: bool) -> Result<AudioCaptureStream> {
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }?;
    let mut format = WaveFormatOwned::from_mix_format(&client)?;
    if float_samples {
        format = format.float32()?;
    }

    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            CAPTURE_STREAM_FLAGS,
            ROUTE_BUFFER_DURATION,
            0,
            format.as_ptr(),
            None,
        )
    }
    .map_err(|error| stream_initialization_error(error, "capture", &format))?;

    let capture = unsafe { client.GetService::<IAudioCaptureClient>() }?;
    Ok(AudioCaptureStream { client, capture, format })
}

fn open_render_client(device: &IMMDevice, format: &WaveFormatOwned) -> Result<AudioRenderStream> {
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }?;

    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            RENDER_STREAM_FLAGS,
            ROUTE_BUFFER_DURATION,
            0,
            format.as_ptr(),
            None,
        )
    }
    .map_err(|error| stream_initialization_error(error, "render", format))?;

    let buffer_frames = unsafe { client.GetBufferSize() }?;
    let render = unsafe { client.GetService::<IAudioRenderClient>() }?;
    Ok(AudioRenderStream { client, render, buffer_frames, format: format.clone() })
}

fn stream_initialization_error(
    error: windows::core::Error,
    stream: &str,
    format: &WaveFormatOwned,
) -> anyhow::Error {
    let message = if error.code() == AUDCLNT_E_DEVICE_IN_USE {
        format!(
            "{stream} Initialize: 오디오 장치가 독점 사용 중입니다 (AUDCLNT_E_DEVICE_IN_USE, 0x8889000A). 장치를 사용하는 앱을 공유 모드로 전환하거나 해당 앱의 장치 사용을 중지한 뒤 다시 시작하세요. format {}",
            format.describe()
        )
    } else {
        format!("{stream} Initialize failed with format {}", format.describe())
    };
    anyhow::Error::new(error).context(message)
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
    fn channels(&self) -> usize {
        usize::from(u16::from_le_bytes([self.bytes[2], self.bytes[3]]))
    }
    fn sample_rate(&self) -> u32 {
        u32::from_le_bytes([self.bytes[4], self.bytes[5], self.bytes[6], self.bytes[7]])
    }
    fn float32(&self) -> Result<Self> {
        let channels = u16::try_from(self.channels())?;
        let rate = self.sample_rate();
        if channels == 0 || rate == 0 {
            bail!("Invalid audio mix format");
        }
        let align = channels.checked_mul(4).context("Too many audio channels")?;
        let byte_rate = rate.checked_mul(u32::from(align)).context("Audio rate overflow")?;
        let mut bytes = Vec::with_capacity(40);
        bytes.extend(0xfffe_u16.to_le_bytes());
        bytes.extend(channels.to_le_bytes());
        bytes.extend(rate.to_le_bytes());
        bytes.extend(byte_rate.to_le_bytes());
        bytes.extend(align.to_le_bytes());
        bytes.extend(32_u16.to_le_bytes());
        bytes.extend(22_u16.to_le_bytes());
        bytes.extend(32_u16.to_le_bytes());
        let mask = if self.bytes.len() >= 40 && self.bytes[..2] == [0xfe, 0xff] {
            &self.bytes[20..24]
        } else {
            &[0_u8; 4]
        };
        bytes.extend(mask);
        bytes.extend([3, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);
        Ok(Self { bytes, block_align: usize::from(align) })
    }
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

// COM initialization and its matching teardown must stay on the same thread.
struct ComGuard {
    _thread: PhantomData<Rc<()>>,
}

impl ComGuard {
    fn new() -> Result<Self> {
        Ok(Self::initialize(COINIT_MULTITHREADED)?)
    }

    fn for_enumeration() -> Result<Self> {
        // Winit initializes OLE/STA for GUI drag-and-drop. Device enumeration
        // supports STA too; routing workers still require MTA through new().
        match Self::initialize(COINIT_MULTITHREADED) {
            Err(error) if error.code() == RPC_E_CHANGED_MODE => {
                Ok(Self::initialize(COINIT_APARTMENTTHREADED)?)
            }
            result => Ok(result?),
        }
    }

    fn initialize(mode: COINIT) -> windows::core::Result<Self> {
        // SAFETY: both S_OK and S_FALSE acquire one initialization reference.
        // A failed call creates no guard and is never followed by CoUninitialize.
        unsafe { CoInitializeEx(None, mode) }.ok()?;
        Ok(Self { _thread: PhantomData })
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY: the guard cannot move to another thread and owns one reference.
        unsafe { CoUninitialize() };
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AudioOutputDevice, ComGuard, DefaultEndpointNotification, WindowsAudioBackend,
        list_output_devices, resolve_output_device,
    };
    use anyhow::Result;
    use audio_cable_core::AudioBackend;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::Media::Audio::{
        DEVICE_STATE_ACTIVE, IMMNotificationClient, eCapture, eCommunications, eConsole,
        eMultimedia, eRender,
    };
    use windows::Win32::System::Com::{
        APTTYPE, APTTYPE_MAINSTA, APTTYPE_MTA, APTTYPE_STA, APTTYPEQUALIFIER,
        COINIT_APARTMENTTHREADED, CoGetApartmentType,
    };
    use windows::core::PCWSTR;

    fn apartment_type() -> windows::core::Result<APTTYPE> {
        let mut apartment = APTTYPE::default();
        let mut qualifier = APTTYPEQUALIFIER::default();
        // SAFETY: both output pointers refer to valid, writable values.
        unsafe { CoGetApartmentType(&raw mut apartment, &raw mut qualifier) }?;
        Ok(apartment)
    }

    fn on_com_thread(test: impl FnOnce() -> Result<()> + Send + 'static) -> Result<()> {
        std::thread::spawn(test)
            .join()
            .map_err(|_| anyhow::anyhow!("COM regression test thread panicked"))?
    }

    #[test]
    fn enumeration_preserves_gui_sta_and_balances_initialization() -> Result<()> {
        on_com_thread(|| {
            let gui = ComGuard::initialize(COINIT_APARTMENTTHREADED)?;
            let gui_apartment = apartment_type()?;
            assert!(matches!(gui_apartment, APTTYPE_STA | APTTYPE_MAINSTA));
            // Reproduce the original failure before exercising the GUI query paths.
            let error = ComGuard::new().err().ok_or_else(|| {
                anyhow::anyhow!("MTA initialization unexpectedly succeeded on an STA thread")
            })?;
            assert_eq!(
                error.downcast_ref::<windows::core::Error>().map(windows::core::Error::code),
                Some(RPC_E_CHANGED_MODE)
            );
            for _ in 0..2 {
                let _outputs = list_output_devices()?;
                let _endpoints = WindowsAudioBackend.endpoints()?;
                assert_eq!(apartment_type()?, gui_apartment);
            }
            drop(gui);
            // Switching modes proves no initialization references were leaked.
            let _worker = ComGuard::new()?;
            assert_eq!(apartment_type()?, APTTYPE_MTA);
            Ok(())
        })
    }

    #[test]
    fn enumeration_preserves_worker_mta_and_balances_nested_initialization() -> Result<()> {
        on_com_thread(|| {
            let worker = ComGuard::new()?;
            for _ in 0..2 {
                let _outputs = list_output_devices()?;
                let _endpoints = WindowsAudioBackend.endpoints()?;
                assert_eq!(apartment_type()?, APTTYPE_MTA);
            }
            drop(worker);
            let _gui = ComGuard::initialize(COINIT_APARTMENTTHREADED)?;
            assert!(matches!(apartment_type()?, APTTYPE_STA | APTTYPE_MAINSTA));
            Ok(())
        })
    }

    #[test]
    fn enumeration_initializes_and_releases_com_on_a_fresh_thread() -> Result<()> {
        on_com_thread(|| {
            let enumeration = ComGuard::for_enumeration()?;
            assert_eq!(apartment_type()?, APTTYPE_MTA);
            drop(enumeration);
            let _gui = ComGuard::initialize(COINIT_APARTMENTTHREADED)?;
            assert!(matches!(apartment_type()?, APTTYPE_STA | APTTYPE_MAINSTA));
            Ok(())
        })
    }

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
