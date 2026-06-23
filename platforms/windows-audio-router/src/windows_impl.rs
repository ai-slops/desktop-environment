use anyhow::{Context, Result, bail};
use std::slice;
use std::thread;
use std::time::Duration;
use tracing::{debug, info};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, DEVICE_STATE_ACTIVE,
    IAudioCaptureClient, IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceCollection,
    IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::PWSTR;

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
    let default_id =
        endpoint_id(&unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }?)?;
    let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }?;
    read_output_devices(&collection, &default_id)
}

pub fn run_output_audio_router(source_selector: &str, target_selector: &str) -> Result<()> {
    let _com = ComGuard::new()?;
    let source = select_output_device(source_selector)
        .with_context(|| format!("failed to resolve source device '{source_selector}'"))?;
    let target = select_output_device(target_selector)
        .with_context(|| format!("failed to resolve target device '{target_selector}'"))?;

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

    unsafe { render_stream.client.Start() }.context("failed to start render client")?;
    unsafe { capture_stream.client.Start() }.context("failed to start capture client")?;
    debug!("Started source capture and target render streams");

    loop {
        pump_audio(&capture_stream, &render_stream)?;
        thread::sleep(Duration::from_millis(3));
    }
}

fn pump_audio(capture: &AudioCaptureStream, render: &AudioRenderStream) -> Result<()> {
    loop {
        let packet_frames = unsafe { capture.capture.GetNextPacketSize() }?;
        if packet_frames == 0 {
            return Ok(());
        }

        let mut data = std::ptr::null_mut();
        let mut frames = 0;
        let mut flags = 0;
        unsafe {
            capture.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
        }

        let silent = (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
        let mut frame_offset = 0_u32;
        while frame_offset < frames {
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
                        .ReleaseBuffer(frames_to_write, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct WaveFormatOwned {
    bytes: Vec<u8>,
    block_align: usize,
}

impl WaveFormatOwned {
    fn from_mix_format(client: &IAudioClient) -> Result<Self> {
        let raw = unsafe { client.GetMixFormat() }?;
        let format = unsafe { wave_format_from_ptr(raw) }?;
        unsafe { CoTaskMemFree(Some(raw.cast())) };
        Ok(format)
    }

    fn as_ptr(&self) -> *const windows::Win32::Media::Audio::WAVEFORMATEX {
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
            "tag={} channels={} rate={} bits={} block_align={} extra={}",
            format_tag, channels, rate, bits, block_align, extra
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
    let devices = list_output_devices()?;
    if selector.eq_ignore_ascii_case("default") {
        return devices
            .into_iter()
            .find(|device| device.is_default)
            .context("no default audio output device found");
    }

    let selector_lower = selector.to_ascii_lowercase();
    devices
        .into_iter()
        .find(|device| {
            device.id.eq_ignore_ascii_case(selector)
                || device.friendly_name.to_ascii_lowercase().contains(&selector_lower)
                || device.id.to_ascii_lowercase().contains(&selector_lower)
        })
        .with_context(|| format!("no audio output matched '{selector}'"))
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
    default_id: &str,
) -> Result<Vec<AudioOutputDevice>> {
    let count = unsafe { collection.GetCount() }?;
    let mut devices = Vec::with_capacity(count as usize);
    for index in 0..count {
        let device = unsafe { collection.Item(index) }?;
        let id = endpoint_id(&device)?;
        devices.push(AudioOutputDevice {
            friendly_name: device_friendly_name(&device)?,
            is_default: id == default_id,
            id,
        });
    }
    Ok(devices)
}

fn endpoint_id(device: &IMMDevice) -> Result<String> {
    let id = unsafe { device.GetId() }?;
    let string = pwstr_to_string(id)?;
    unsafe { CoTaskMemFree(Some(id.0.cast())) };
    Ok(string)
}

fn device_friendly_name(device: &IMMDevice) -> Result<String> {
    let store: IPropertyStore = unsafe { device.OpenPropertyStore(STGM_READ) }?;
    let mut value = unsafe { store.GetValue(&PKEY_Device_FriendlyName as *const _) }?;
    let text = unsafe { PropVariantToStringAlloc(&value) }?;
    let friendly_name = pwstr_to_string(text)?;
    unsafe {
        CoTaskMemFree(Some(text.0.cast()));
        PropVariantClear(&mut value)?;
    }
    Ok(friendly_name)
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
