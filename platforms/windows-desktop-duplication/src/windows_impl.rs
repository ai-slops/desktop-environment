use anyhow::{Context, Result, anyhow, bail};
use display_relay_core::{DisplayArea, VirtualDesktop};
use std::mem::MaybeUninit;
use windows::Win32::Foundation::{E_ACCESSDENIED, HMODULE, RECT};
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
    DXGI_OUTPUT_DESC, IDXGIAdapter, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1, IDXGIOutput,
    IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
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
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }?;
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

            let description = unsafe { output.GetDesc() }?;
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
    duplication: IDXGIOutputDuplication,
    staging_texture: ID3D11Texture2D,
    frame_buffer: Vec<u8>,
}

/// A freshly created D3D11 device bound to the desktop duplication output for
/// the requested display name.
struct ResolvedDisplay {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    display: DisplayInfo,
    duplication: IDXGIOutputDuplication,
}

/// Resolves `display_name` against the current desktop topology and binds a
/// brand-new D3D11 device to it. Always starting from a fresh device (rather
/// than reusing one obtained before a disconnect) matters: an existing
/// device's adapter can keep reporting a stale output list after the display
/// is unplugged and replugged, so retrying with it never finds the display
/// again even though it is back. Re-resolving from scratch, keyed on the
/// originally requested `display_name`, mirrors what a full process restart
/// does and lets recovery actually succeed once the display returns.
fn resolve_display(display_name: &str) -> Result<ResolvedDisplay> {
    let (device, context) = create_device()?;
    let display = enumerate_displays()?
        .into_iter()
        .find(|display| display.name.eq_ignore_ascii_case(display_name))
        .with_context(|| format!("Display '{display_name}' was not found"))?;

    let dxgi_device: IDXGIDevice = device.cast()?;
    let adapter = unsafe { dxgi_device.GetAdapter() }?;
    let output = find_output(&adapter, &display.name)?
        .ok_or_else(|| anyhow!("Display '{display_name}' is no longer available"))?;
    let output1: IDXGIOutput1 = output.cast()?;
    let duplication = unsafe { output1.DuplicateOutput(&device) }.map_err(|error| {
        if error.code() == E_ACCESSDENIED {
            anyhow!(
                "Desktop Duplication access was denied. Run from the interactive user session on the GPU that owns the target display"
            )
        } else {
            anyhow!(error)
        }
    })?;

    Ok(ResolvedDisplay { device, context, display, duplication })
}

impl DesktopDuplicator {
    pub fn new(display_name: &str) -> Result<Self> {
        let resolved = resolve_display(display_name)?;

        let staging_texture = create_staging_texture(
            &resolved.device,
            resolved.display.area.width,
            resolved.display.area.height,
        )?;
        let frame_buffer = vec![
            0_u8;
            resolved.display.area.width as usize
                * resolved.display.area.height as usize
                * 4
        ];

        Ok(Self {
            display_name: display_name.to_string(),
            display: resolved.display,
            device: resolved.device,
            context: resolved.context,
            duplication: resolved.duplication,
            staging_texture,
            frame_buffer,
        })
    }

    #[must_use]
    pub fn display_info(&self) -> &DisplayInfo {
        &self.display
    }

    /// Re-acquires desktop duplication for the display originally requested by
    /// name (not whatever was last resolved) after access was lost, e.g. a
    /// display or GPU reconfiguration. Rebuilds the D3D11 device from scratch;
    /// see [`resolve_display`] for why that is necessary for recovery to work.
    pub fn recreate(&mut self) -> Result<()> {
        let resolved = resolve_display(&self.display_name)?;

        if resolved.display.area.width != self.display.area.width
            || resolved.display.area.height != self.display.area.height
        {
            self.staging_texture = create_staging_texture(
                &resolved.device,
                resolved.display.area.width,
                resolved.display.area.height,
            )?;
            self.frame_buffer = vec![
                0_u8;
                resolved.display.area.width as usize
                    * resolved.display.area.height as usize
                    * 4
            ];
        }

        self.display = resolved.display;
        self.device = resolved.device;
        self.context = resolved.context;
        self.duplication = resolved.duplication;
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
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None::<IDXGIResource>;

        let acquire_result = unsafe {
            self.duplication.AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource)
        };

        match acquire_result {
            Ok(()) => {}
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(false),
            Err(error) if error.code() == DXGI_ERROR_ACCESS_LOST => {
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
            self.duplication.ReleaseFrame()?;
        }

        Ok(true)
    }

    pub fn capture_frame<'a>(&'a mut self, timeout_ms: u32) -> Result<CaptureFrameView<'a>> {
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None::<IDXGIResource>;

        let acquire_result = unsafe {
            self.duplication.AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource)
        };

        match acquire_result {
            Ok(()) => {}
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                bail!("Timed out waiting for the next frame")
            }
            Err(error) if error.code() == DXGI_ERROR_ACCESS_LOST => {
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
            self.duplication.ReleaseFrame()?;
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
        let _ = unsafe { self.duplication.ReleaseFrame() };
        let _ = &self.device;
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
    }?;

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

fn find_output(adapter: &IDXGIAdapter, display_name: &str) -> Result<Option<IDXGIOutput>> {
    let mut output_index = 0;
    loop {
        let output = match unsafe { adapter.EnumOutputs(output_index) } {
            Ok(output) => output,
            Err(_) => return Ok(None),
        };
        output_index += 1;

        let description = unsafe { output.GetDesc() }?;
        let candidate = utf16_to_string(&description.DeviceName);
        if candidate.eq_ignore_ascii_case(display_name) {
            return Ok(Some(output));
        }
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
