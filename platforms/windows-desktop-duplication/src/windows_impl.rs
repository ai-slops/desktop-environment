use anyhow::{Context, Result, anyhow, bail};
use display_relay_core::{DisplayArea, VirtualDesktop};
use std::mem::MaybeUninit;
use windows::Win32::Foundation::{E_ACCESSDENIED, HMODULE, LUID, RECT};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
    ID3D11Resource, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION_IDENTITY, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    DXGI_OUTPUT_DESC, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1, IDXGIOutput, IDXGIOutput1,
    IDXGIOutputDuplication, IDXGIResource,
};
use windows::core::Interface;

#[derive(Debug, Clone)]
pub struct DisplayInfo {
    pub name: String,
    pub friendly_name: String,
    pub area: DisplayArea,
    pub virtual_desktop: VirtualDesktop,
}

#[derive(Debug, Clone, Copy)]
pub struct CaptureFrameView<'a> {
    pub width: u32,
    pub height: u32,
    pub pixels_bgra: &'a [u8],
}

/// Signals that the OS revoked desktop duplication access (e.g. a display or GPU
/// reconfiguration). Callers should call [`DesktopDuplicator::recreate`] and retry.
#[derive(Debug)]
pub struct DuplicationAccessLost;

impl std::fmt::Display for DuplicationAccessLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Desktop duplication access was lost; recreate the relay session")
    }
}

impl std::error::Error for DuplicationAccessLost {}

pub fn enumerate_displays() -> Result<Vec<DisplayInfo>> {
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.context("CreateDXGIFactory1 failed")?;
    let virtual_bounds = read_virtual_desktop_bounds();
    let mut displays = Vec::new();

    let mut adapter_index = 0;
    loop {
        let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(adapter_index) } {
            Ok(adapter) => adapter,
            Err(_) => break,
        };
        adapter_index += 1;

        let mut output_index = 0;
        loop {
            let output: IDXGIOutput = match unsafe { adapter.EnumOutputs(output_index) } {
                Ok(output) => output,
                Err(_) => break,
            };
            output_index += 1;

            let description =
                unsafe { output.GetDesc() }.context("IDXGIOutput::GetDesc failed (enumerate)")?;
            if !description.AttachedToDesktop.as_bool() {
                continue;
            }

            displays.push(display_from_desc(description, virtual_bounds));
        }
    }

    if displays.is_empty() {
        bail!("No desktop-attached outputs were found")
    }

    Ok(displays)
}

pub struct DesktopDuplicator {
    display_name: String,
    display: DisplayInfo,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    // Identifies the physical GPU `device` was created on. Desktop Duplication requires the
    // device passed to DuplicateOutput to belong to the same adapter as the output, so a
    // freshly re-enumerated adapter is matched back to this device by LUID rather than by
    // creating a new device (see `recreate` for why the device must stay the same).
    adapter_luid: LUID,
    // `None` only ever exists transiently inside `recreate` between releasing the old
    // duplication and acquiring the new one, and right after an access-lost error is
    // reported from `copy_latest_frame_to`/`capture_frame`.
    duplication: Option<IDXGIOutputDuplication>,
    staging_texture: ID3D11Texture2D,
    frame_buffer: Vec<u8>,
}

/// Re-enumerates the current desktop topology from a brand-new DXGI factory — never reusing
/// a cached adapter, which can report a stale output list after a display is unplugged and
/// replugged — and duplicates `display_name`'s output using the given, already-created
/// `device`. Only the freshly-enumerated adapter that matches `adapter_luid` (the physical
/// GPU `device` belongs to) is searched, since DuplicateOutput requires the device and the
/// output to share an adapter anyway.
fn acquire_duplication(
    device: &ID3D11Device,
    adapter_luid: LUID,
    display_name: &str,
) -> Result<(DisplayInfo, IDXGIOutputDuplication)> {
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.context("CreateDXGIFactory1 failed")?;
    let virtual_bounds = read_virtual_desktop_bounds();

    let mut adapter_index = 0;
    let adapter: IDXGIAdapter1 = loop {
        let adapter: IDXGIAdapter1 = unsafe { factory.EnumAdapters1(adapter_index) }
            .with_context(|| format!("GPU adapter for display '{display_name}' was not found"))?;
        adapter_index += 1;

        let desc = unsafe { adapter.GetDesc1() }.context("IDXGIAdapter1::GetDesc1 failed")?;
        if desc.AdapterLuid == adapter_luid {
            break adapter;
        }
    };

    let mut output_index = 0;
    loop {
        let output: IDXGIOutput = unsafe { adapter.EnumOutputs(output_index) }
            .map_err(|_| anyhow!("Display '{display_name}' is no longer available"))?;
        output_index += 1;

        let description =
            unsafe { output.GetDesc() }.context("IDXGIOutput::GetDesc failed (acquire)")?;
        let name = utf16_to_string(&description.DeviceName);
        if !name.eq_ignore_ascii_case(display_name) {
            continue;
        }
        if !description.AttachedToDesktop.as_bool() {
            bail!("Display '{display_name}' is no longer attached to the desktop");
        }

        let display = display_from_desc(description, virtual_bounds);
        let output1: IDXGIOutput1 =
            output.cast().context("failed to query the IDXGIOutput1 interface")?;
        let duplication = unsafe { output1.DuplicateOutput(device) }.map_err(|error| {
            if error.code() == E_ACCESSDENIED {
                anyhow!(
                    "Desktop Duplication access was denied. Run from the interactive user session on the GPU that owns the target display"
                )
            } else {
                anyhow!(error)
            }
        })?;

        return Ok((display, duplication));
    }
}

fn adapter_luid_of(device: &ID3D11Device) -> Result<LUID> {
    let dxgi_device: IDXGIDevice =
        device.cast().context("failed to query the IDXGIDevice interface")?;
    let adapter = unsafe { dxgi_device.GetAdapter() }.context("IDXGIDevice::GetAdapter failed")?;
    let adapter1: IDXGIAdapter1 =
        adapter.cast().context("failed to query the IDXGIAdapter1 interface")?;
    Ok(unsafe { adapter1.GetDesc1() }.context("IDXGIAdapter1::GetDesc1 failed")?.AdapterLuid)
}

impl DesktopDuplicator {
    pub fn new(display_name: &str) -> Result<Self> {
        let (device, context) = create_device().context("failed to create the D3D11 device")?;
        let adapter_luid = adapter_luid_of(&device)?;
        let (display, duplication) = acquire_duplication(&device, adapter_luid, display_name)?;

        let staging_texture =
            create_staging_texture(&device, display.area.width, display.area.height)?;
        let frame_buffer =
            vec![0_u8; display.area.width as usize * display.area.height as usize * 4];

        Ok(Self {
            display_name: display_name.to_string(),
            display,
            device,
            context,
            adapter_luid,
            duplication: Some(duplication),
            staging_texture,
            frame_buffer,
        })
    }

    #[must_use]
    pub fn display_info(&self) -> &DisplayInfo {
        &self.display
    }

    #[must_use]
    pub fn has_duplication(&self) -> bool {
        self.duplication.is_some()
    }

    /// Re-acquires desktop duplication for the display originally requested by name (not
    /// whatever was last resolved) after access was lost, e.g. a display reconfiguration or
    /// the secure desktop (a UAC prompt) having been shown. This deliberately reuses the
    /// existing `device` rather than creating a new one: a D3D11 device is not what goes
    /// stale after a display is unplugged and replugged (a cached *adapter* is, which
    /// `acquire_duplication` avoids by always enumerating a fresh one), and recreating the
    /// device would require recreating the window's swap chain to match it. Windows only
    /// allows one flip-model swap chain per window ever, and D3D11 defers destroying the old
    /// one when it's replaced; tearing it down and immediately creating a new one for the
    /// same window was observed to fail with E_ACCESSDENIED indefinitely, well past the
    /// point the secure desktop was actually gone. Keeping the device (and therefore the
    /// window's swap chain) untouched across recovery avoids that entirely.
    pub fn recreate(&mut self) -> Result<()> {
        // Release the old (already-inaccessible) duplication interface before requesting a
        // new one. Desktop Duplication only permits one active duplication interface per
        // output at a time; while the previous interface object is still alive, every
        // subsequent DuplicateOutput call keeps failing with E_ACCESSDENIED forever, even
        // long after whatever caused the original access loss (e.g. a UAC prompt) is gone.
        self.duplication = None;

        let (display, duplication) =
            acquire_duplication(&self.device, self.adapter_luid, &self.display_name)?;

        if display.area.width != self.display.area.width
            || display.area.height != self.display.area.height
        {
            self.staging_texture =
                create_staging_texture(&self.device, display.area.width, display.area.height)?;
            self.frame_buffer =
                vec![0_u8; display.area.width as usize * display.area.height as usize * 4];
        }

        self.display = display;
        self.duplication = Some(duplication);
        Ok(())
    }

    #[must_use]
    pub fn device(&self) -> ID3D11Device {
        self.device.clone()
    }

    #[must_use]
    pub fn context(&self) -> ID3D11DeviceContext {
        self.context.clone()
    }

    pub fn create_gpu_texture(&self) -> Result<ID3D11Texture2D> {
        create_shader_texture(&self.device, self.display.area.width, self.display.area.height)
    }

    pub fn copy_latest_frame_to(
        &mut self,
        target_texture: &ID3D11Texture2D,
        timeout_ms: u32,
    ) -> Result<bool> {
        let duplication =
            self.duplication.as_ref().context("Desktop duplication session not ready")?;
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None::<IDXGIResource>;

        let acquire_result =
            unsafe { duplication.AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource) };

        match acquire_result {
            Ok(()) => {}
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(false),
            Err(error)
                if error.code() == DXGI_ERROR_ACCESS_LOST || error.code() == E_ACCESSDENIED =>
            {
                self.duplication = None;
                return Err(DuplicationAccessLost.into());
            }
            Err(error) => return Err(error.into()),
        }

        let resource = resource.context("Desktop duplication returned no frame resource")?;
        let texture: ID3D11Texture2D = resource.cast()?;
        let texture_resource: ID3D11Resource = texture.cast()?;
        let target_resource: ID3D11Resource = target_texture.cast()?;

        unsafe {
            self.context.CopyResource(&target_resource, &texture_resource);
        }
        if let Err(error) = release_frame(duplication) {
            self.duplication = None;
            return Err(error);
        }

        Ok(true)
    }

    pub fn capture_frame<'a>(&'a mut self, timeout_ms: u32) -> Result<CaptureFrameView<'a>> {
        let duplication =
            self.duplication.as_ref().context("Desktop duplication session not ready")?;
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None::<IDXGIResource>;

        let acquire_result =
            unsafe { duplication.AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource) };

        match acquire_result {
            Ok(()) => {}
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                bail!("Timed out waiting for the next frame")
            }
            Err(error)
                if error.code() == DXGI_ERROR_ACCESS_LOST || error.code() == E_ACCESSDENIED =>
            {
                self.duplication = None;
                return Err(DuplicationAccessLost.into());
            }
            Err(error) => return Err(error.into()),
        }

        let resource = resource.context("Desktop duplication returned no frame resource")?;
        let texture: ID3D11Texture2D = resource.cast()?;
        let texture_resource: ID3D11Resource = texture.cast()?;
        let staging_resource: ID3D11Resource = self.staging_texture.cast()?;

        unsafe {
            self.context.CopyResource(&staging_resource, &texture_resource);
        }

        let mapped = map_texture(&self.context, &self.staging_texture)?;
        let width = self.display.area.width as usize;
        let height = self.display.area.height as usize;
        let row_pitch = mapped.RowPitch as usize;
        let bytes_per_row = width * 4;
        let total_bytes = bytes_per_row * height;

        if self.frame_buffer.len() != total_bytes {
            self.frame_buffer.resize(total_bytes, 0);
        }

        unsafe {
            let src = mapped.pData.cast::<u8>();
            if row_pitch == bytes_per_row {
                std::ptr::copy_nonoverlapping(src, self.frame_buffer.as_mut_ptr(), total_bytes);
            } else {
                for row in 0..height {
                    let src_row = src.add(row * row_pitch);
                    let dst_offset = row * bytes_per_row;
                    std::ptr::copy_nonoverlapping(
                        src_row,
                        self.frame_buffer[dst_offset..].as_mut_ptr(),
                        bytes_per_row,
                    );
                }
            }
            self.context.Unmap(&self.staging_texture, 0);
        }
        if let Err(error) = release_frame(duplication) {
            self.duplication = None;
            return Err(error);
        }

        Ok(CaptureFrameView {
            width: self.display.area.width,
            height: self.display.area.height,
            pixels_bgra: &self.frame_buffer,
        })
    }
}

impl Drop for DesktopDuplicator {
    fn drop(&mut self) {
        if let Some(duplication) = &self.duplication {
            let _ = unsafe { duplication.ReleaseFrame() };
        }
    }
}

fn create_device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let feature_levels = [D3D_FEATURE_LEVEL_11_0];
    let mut device = None;
    let mut context = None;
    let mut created_level = D3D_FEATURE_LEVEL(0);

    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut created_level),
            Some(&mut context),
        )
    }
    .context("D3D11CreateDevice failed")?;

    let device = device.context("D3D11CreateDevice returned no device")?;
    let context = context.context("D3D11CreateDevice returned no device context")?;

    if created_level != D3D_FEATURE_LEVEL_11_0 {
        bail!("Desktop Duplication requires a D3D11-capable adapter")
    }

    Ok((device, context))
}

fn create_staging_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<ID3D11Texture2D> {
    let description = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };

    let mut texture = None;
    unsafe {
        device.CreateTexture2D(&description, None, Some(&mut texture))?;
    }
    texture.context("CreateTexture2D returned no staging texture")
}

fn create_shader_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<ID3D11Texture2D> {
    let description = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };

    let mut texture = None;
    unsafe {
        device.CreateTexture2D(&description, None, Some(&mut texture))?;
    }
    texture.context("CreateTexture2D returned no shader texture")
}

fn map_texture(
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
) -> Result<D3D11_MAPPED_SUBRESOURCE> {
    let mut mapped = MaybeUninit::<D3D11_MAPPED_SUBRESOURCE>::zeroed();

    unsafe {
        context.Map(texture, 0, D3D11_MAP_READ, 0, Some(mapped.as_mut_ptr().cast()))?;
        Ok(mapped.assume_init())
    }
}

/// Releases the frame acquired by `AcquireNextFrame`. The secure desktop
/// (UAC prompts, the lock screen, Ctrl+Alt+Del) can appear between a
/// successful acquire and this call, which then fails with
/// `DXGI_ERROR_ACCESS_LOST` or `E_ACCESSDENIED` instead of succeeding; treat
/// both the same as a failed acquire so the caller recovers instead of
/// treating it as a fatal error.
fn release_frame(duplication: &IDXGIOutputDuplication) -> Result<()> {
    match unsafe { duplication.ReleaseFrame() } {
        Ok(()) => Ok(()),
        Err(error) if error.code() == DXGI_ERROR_ACCESS_LOST || error.code() == E_ACCESSDENIED => {
            Err(DuplicationAccessLost.into())
        }
        Err(error) => Err(error.into()),
    }
}

fn display_from_desc(description: DXGI_OUTPUT_DESC, virtual_bounds: DisplayArea) -> DisplayInfo {
    let area = rect_to_area(description.DesktopCoordinates);
    let name = utf16_to_string(&description.DeviceName);
    let friendly_name = if description.Rotation == DXGI_MODE_ROTATION_IDENTITY {
        format!("{name} ({}x{})", area.width, area.height)
    } else {
        format!("{name} (rotated)")
    };

    DisplayInfo {
        name,
        friendly_name,
        area,
        virtual_desktop: VirtualDesktop { bounds: virtual_bounds },
    }
}

fn rect_to_area(rect: RECT) -> DisplayArea {
    DisplayArea {
        left: rect.left,
        top: rect.top,
        width: (rect.right - rect.left) as u32,
        height: (rect.bottom - rect.top) as u32,
    }
}

fn read_virtual_desktop_bounds() -> DisplayArea {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) as u32 };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) as u32 };

    DisplayArea { left, top, width, height }
}

fn utf16_to_string(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|value| *value == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}
