use anyhow::{Context, Result, bail};
use display_relay_core::RelayConfig;
use std::ffi::c_void;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::error;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BUFFER_DESC, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
    D3D11_SAMPLER_DESC, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_DEFAULT, D3D11_VIEWPORT, ID3D11Buffer,
    ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader, ID3D11RenderTargetView,
    ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_UNSPECIFIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2,
    IDXGISwapChain1,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, DefWindowProcW, GWLP_WNDPROC, GetClientRect, GetPropW, GetWindowRect,
    RemovePropW, SetPropW, SetWindowLongPtrW, WINDOW_LONG_PTR_INDEX, WM_ENTERSIZEMOVE,
    WM_EXITSIZEMOVE, WM_NCDESTROY, WM_SIZING, WMSZ_BOTTOM, WMSZ_BOTTOMLEFT, WMSZ_BOTTOMRIGHT,
    WMSZ_LEFT, WMSZ_RIGHT, WMSZ_TOP, WMSZ_TOPLEFT, WMSZ_TOPRIGHT,
};
use windows::core::{Interface, PCSTR, w};
use windows_desktop_duplication::{DesktopDuplicator, enumerate_displays};
use windows_input::RemoteInputController;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,wgpu_core=warn".into()),
        )
        .with_target(false)
        .init();

    let command = Command::from_env()?;
    match command {
        Command::List => list_displays(),
        Command::Mirror(config) => run_relay(config),
    }
}

fn list_displays() -> Result<()> {
    for display in enumerate_displays()? {
        println!(
            "{}\t{}\t({}, {}) {}x{}",
            display.name,
            display.friendly_name,
            display.area.left,
            display.area.top,
            display.area.width,
            display.area.height
        );
    }

    Ok(())
}

fn run_relay(config: RelayConfig) -> Result<()> {
    let event_loop = EventLoop::new()?;
    let mut app = RelayApp::new(config)?;
    event_loop.run_app(&mut app)?;
    Ok(())
}

enum Command {
    List,
    Mirror(RelayConfig),
}

impl Command {
    fn from_env() -> Result<Self> {
        let mut args = std::env::args().skip(1);
        let Some(first) = args.next() else {
            bail!(
                "Usage: display-relay list | mirror <DISPLAY_NAME> [--fullscreen] [--timeout-ms N] [--fps N]"
            )
        };

        match first.as_str() {
            "list" => Ok(Self::List),
            "mirror" => {
                let display_name = args
                    .next()
                    .context("mirror requires a display name such as \\\\.\\DISPLAY3")?;
                let mut config = RelayConfig::default();
                config.target.display_name = display_name;

                while let Some(arg) = args.next() {
                    match arg.as_str() {
                        "--fullscreen" => config.mirror_fullscreen = true,
                        "--timeout-ms" => {
                            let value = args.next().context("--timeout-ms expects a number")?;
                            config.capture_timeout_ms = value.parse()?;
                        }
                        "--fps" => {
                            let value = args.next().context("--fps expects a number")?;
                            config.target_fps = value.parse()?;
                            if config.target_fps == 0 {
                                bail!("--fps must be greater than 0");
                            }
                        }
                        other => bail!("Unknown argument: {other}"),
                    }
                }

                Ok(Self::Mirror(config))
            }
            other => bail!("Unknown command: {other}"),
        }
    }
}

struct RelayApp {
    config: RelayConfig,
    duplicator: DesktopDuplicator,
    input: RemoteInputController,
    window: Option<Arc<Window>>,
    window_id: Option<WindowId>,
    renderer: Option<FastRenderer>,
    last_window_size: Option<PhysicalSize<u32>>,
    frame_interval: Duration,
    next_frame_deadline: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResizeAxis {
    Width,
    Height,
}

struct AspectRatioHook {
    aspect_width: u32,
    aspect_height: u32,
    old_wndproc: isize,
}

impl RelayApp {
    fn new(config: RelayConfig) -> Result<Self> {
        let duplicator = DesktopDuplicator::new(&config.target.display_name)?;
        let input = RemoteInputController::new(duplicator.display_info().virtual_desktop);
        let frame_interval = Duration::from_secs_f64(1.0 / f64::from(config.target_fps.max(1)));

        Ok(Self {
            config,
            duplicator,
            input,
            window: None,
            window_id: None,
            renderer: None,
            last_window_size: None,
            frame_interval,
            next_frame_deadline: Instant::now(),
        })
    }

    fn redraw(&mut self) -> Result<()> {
        let cursor_overlay = self.cursor_overlay()?;
        let capture_timeout_ms = self.effective_capture_timeout_ms();
        let renderer = self.renderer.as_mut().context("renderer not ready")?;
        let updated =
            self.duplicator.copy_latest_frame_to(renderer.capture_texture(), capture_timeout_ms)?;

        renderer.render(cursor_overlay, updated)?;
        Ok(())
    }

    fn effective_capture_timeout_ms(&self) -> u32 {
        let frame_interval_ms =
            self.frame_interval.as_millis().clamp(1, u128::from(u32::MAX)) as u32;
        self.config.capture_timeout_ms.min(frame_interval_ms).max(1)
    }

    fn cursor_overlay(&self) -> Result<CursorOverlay> {
        let display = self.duplicator.display_info().area;
        let (cursor_x, cursor_y) = self.input.cursor_position()?;

        if !display.contains(cursor_x, cursor_y) {
            return Ok(CursorOverlay::hidden());
        }

        let x = (cursor_x - display.left) as f32 / display.width.max(1) as f32;
        let y = (cursor_y - display.top) as f32 / display.height.max(1) as f32;
        Ok(CursorOverlay {
            position: [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)],
            visible: 1.0,
            radius_px: 14.0,
        })
    }
}

impl ApplicationHandler for RelayApp {
    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        if !matches!(cause, StartCause::Init | StartCause::ResumeTimeReached { .. }) {
            return;
        }

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let display = self.duplicator.display_info();
        let (resize_step_width, resize_step_height) =
            reduce_ratio(display.area.width.max(1), display.area.height.max(1));
        let mut attributes = WindowAttributes::default()
            .with_title(format!("Relay {} (view only)", display.name))
            .with_inner_size(LogicalSize::new(
                f64::from(display.area.width),
                f64::from(display.area.height),
            ))
            .with_resize_increments(LogicalSize::new(
                f64::from(resize_step_width),
                f64::from(resize_step_height),
            ));

        if self.config.mirror_fullscreen {
            attributes = attributes.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }

        let window = match event_loop.create_window(attributes) {
            Ok(window) => window,
            Err(error) => {
                error!("Failed to create relay window: {error}");
                event_loop.exit();
                return;
            }
        };

        let window = Arc::new(window);
        if let Err(error) = install_aspect_ratio_hook(
            &window,
            display.area.width.max(1),
            display.area.height.max(1),
        ) {
            error!("Failed to install aspect ratio hook: {error:#}");
            event_loop.exit();
            return;
        }
        let renderer = match FastRenderer::new(Arc::clone(&window), &self.duplicator) {
            Ok(renderer) => renderer,
            Err(error) => {
                error!("Failed to create fast renderer: {error:#}");
                event_loop.exit();
                return;
            }
        };

        self.window_id = Some(window.id());
        self.last_window_size = Some(window.inner_size());
        self.renderer = Some(renderer);
        self.window = Some(window);
        self.next_frame_deadline = Instant::now();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if Some(window_id) != self.window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.last_window_size = Some(size);
                if let Some(renderer) = self.renderer.as_mut() {
                    if let Err(error) = renderer.resize(size.width, size.height) {
                        error!("Failed to resize relay surface: {error:#}");
                        event_loop.exit();
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                    && event.state == ElementState::Pressed
                {
                    event_loop.exit();
                }
            }
            WindowEvent::RedrawRequested => {
                self.next_frame_deadline = Instant::now() + self.frame_interval;
                if let Err(error) = self.redraw() {
                    error!("Relay redraw failed: {error:#}");
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame_deadline));
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            let _ = uninstall_aspect_ratio_hook(window);
        }
    }
}

struct FastRenderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain1,
    render_target_view: Option<ID3D11RenderTargetView>,
    capture_texture: ID3D11Texture2D,
    capture_srv: ID3D11ShaderResourceView,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    cursor_buffer: ID3D11Buffer,
    source_size: PhysicalSize<u32>,
    viewport: D3D11_VIEWPORT,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct CursorOverlay {
    position: [f32; 2],
    visible: f32,
    radius_px: f32,
}

impl CursorOverlay {
    const fn hidden() -> Self {
        Self { position: [0.0, 0.0], visible: 0.0, radius_px: 0.0 }
    }
}

impl FastRenderer {
    fn new(window: Arc<Window>, duplicator: &DesktopDuplicator) -> Result<Self> {
        let device = duplicator.device();
        let context = duplicator.context();
        let hwnd = hwnd_from_window(&window)?;
        let swap_chain = create_swap_chain(&device, hwnd, window.inner_size())?;
        let render_target_view = create_render_target_view(&device, &swap_chain)?;
        let capture_texture = duplicator.create_gpu_texture()?;
        let capture_srv = create_shader_resource_view(&device, &capture_texture)?;
        let vertex_shader = compile_vertex_shader(&device)?;
        let pixel_shader = compile_pixel_shader(&device)?;
        let sampler = create_sampler(&device)?;
        let cursor_buffer = create_cursor_buffer(&device)?;
        let source_size = PhysicalSize::new(
            duplicator.display_info().area.width,
            duplicator.display_info().area.height,
        );
        let viewport = fitted_viewport(window.inner_size(), source_size);

        Ok(Self {
            device,
            context,
            swap_chain,
            render_target_view: Some(render_target_view),
            capture_texture,
            capture_srv,
            vertex_shader,
            pixel_shader,
            sampler,
            cursor_buffer,
            source_size,
            viewport,
        })
    }

    fn capture_texture(&self) -> &ID3D11Texture2D {
        &self.capture_texture
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }

        unsafe {
            self.context.OMSetRenderTargets(Some(&[]), None);
            self.context.ClearState();
            self.context.Flush();
        }
        let old_rtv = self.render_target_view.take();
        drop(old_rtv);
        self.render_target_view =
            Some(recreate_swap_chain_target(&self.device, &self.swap_chain, width, height)?);
        self.viewport = fitted_viewport(PhysicalSize::new(width, height), self.source_size);
        Ok(())
    }

    fn render(&mut self, cursor: CursorOverlay, _updated: bool) -> Result<()> {
        update_cursor_buffer(&self.context, &self.cursor_buffer, cursor);
        let render_target_view =
            self.render_target_view.as_ref().context("render target view not ready")?;

        let clear = [0.0_f32, 0.0, 0.0, 1.0];
        unsafe {
            self.context.ClearRenderTargetView(render_target_view, &clear);
            self.context.OMSetRenderTargets(Some(&[Some(render_target_view.clone())]), None);
            self.context.RSSetViewports(Some(&[self.viewport]));
            self.context.VSSetShader(&self.vertex_shader, None);
            self.context.PSSetShader(&self.pixel_shader, None);
            self.context.PSSetShaderResources(0, Some(&[Some(self.capture_srv.clone())]));
            self.context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context.PSSetConstantBuffers(0, Some(&[Some(self.cursor_buffer.clone())]));
            self.context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.Draw(3, 0);
        }

        unsafe { self.swap_chain.Present(0, DXGI_PRESENT(0)) }.ok()?;
        Ok(())
    }
}

fn hwnd_from_window(window: &Window) -> Result<HWND> {
    let handle = window.window_handle()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Ok(HWND(win32.hwnd.get() as *mut c_void)),
        _ => bail!("display-relay requires a Win32 window handle"),
    }
}

fn install_aspect_ratio_hook(window: &Window, aspect_width: u32, aspect_height: u32) -> Result<()> {
    let hwnd = hwnd_from_window(window)?;
    let hook = Box::new(AspectRatioHook { aspect_width, aspect_height, old_wndproc: 0 });
    let hook_ptr = Box::into_raw(hook);

    unsafe {
        SetPropW(
            hwnd,
            w!("DisplayRelayAspectHook"),
            Some(windows::Win32::Foundation::HANDLE(hook_ptr.cast::<core::ffi::c_void>())),
        )?;
        let old_wndproc = SetWindowLongPtrW(
            hwnd,
            WINDOW_LONG_PTR_INDEX(GWLP_WNDPROC.0),
            aspect_ratio_wndproc as *const () as usize as isize,
        );
        (*hook_ptr).old_wndproc = old_wndproc;
    }

    Ok(())
}

fn uninstall_aspect_ratio_hook(window: &Window) -> Result<()> {
    let hwnd = hwnd_from_window(window)?;
    unsafe {
        let handle = RemovePropW(hwnd, w!("DisplayRelayAspectHook"))?;
        let hook_ptr = handle.0 as *mut AspectRatioHook;
        if !hook_ptr.is_null() {
            let old_wndproc = (*hook_ptr).old_wndproc;
            SetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(GWLP_WNDPROC.0), old_wndproc);
            drop(Box::from_raw(hook_ptr));
        }
    }
    Ok(())
}

unsafe extern "system" fn aspect_ratio_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let hook_handle = unsafe { GetPropW(hwnd, w!("DisplayRelayAspectHook")) };
    if hook_handle.0.is_null() {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }

    let hook = unsafe { &mut *(hook_handle.0 as *mut AspectRatioHook) };

    match message {
        WM_ENTERSIZEMOVE | WM_EXITSIZEMOVE => {}
        WM_SIZING => {
            let rect_ptr = lparam.0 as *mut RECT;
            if !rect_ptr.is_null() {
                unsafe { apply_aspect_ratio_to_sizing_rect(hwnd, hook, wparam, &mut *rect_ptr) };
                return LRESULT(1);
            }
        }
        WM_NCDESTROY => {
            let old_wndproc = hook.old_wndproc;
            let hook_ptr = hook as *mut AspectRatioHook;
            unsafe {
                let _ = RemovePropW(hwnd, w!("DisplayRelayAspectHook"));
                SetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(GWLP_WNDPROC.0), old_wndproc);
            }
            let result = call_original_wndproc(old_wndproc, hwnd, message, wparam, lparam);
            unsafe {
                drop(Box::from_raw(hook_ptr));
            }
            return result;
        }
        _ => {}
    }

    call_original_wndproc(hook.old_wndproc, hwnd, message, wparam, lparam)
}

unsafe fn apply_aspect_ratio_to_sizing_rect(
    hwnd: HWND,
    hook: &mut AspectRatioHook,
    edge: WPARAM,
    rect: &mut RECT,
) {
    let mut current_window_rect = RECT::default();
    let mut current_client_rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut current_window_rect) }.is_err()
        || unsafe { GetClientRect(hwnd, &mut current_client_rect) }.is_err()
    {
        return;
    }

    let current_client_width = rect_width(&current_client_rect).max(1);
    let current_client_height = rect_height(&current_client_rect).max(1);
    let frame_width = (rect_width(&current_window_rect) - current_client_width).max(0);
    let frame_height = (rect_height(&current_window_rect) - current_client_height).max(0);

    let requested_client_width = (rect_width(rect) - frame_width).max(1);
    let requested_client_height = (rect_height(rect) - frame_height).max(1);
    let axis = choose_resize_axis(
        edge,
        requested_client_width,
        requested_client_height,
        hook.aspect_width,
        hook.aspect_height,
    );

    let target_client_width;
    let target_client_height;
    match axis {
        ResizeAxis::Width => {
            target_client_width = requested_client_width.max(1);
            target_client_height = ((i64::from(target_client_width)
                * i64::from(hook.aspect_height))
                / i64::from(hook.aspect_width))
            .max(1) as i32;
        }
        ResizeAxis::Height => {
            target_client_height = requested_client_height.max(1);
            target_client_width = ((i64::from(target_client_height) * i64::from(hook.aspect_width))
                / i64::from(hook.aspect_height))
            .max(1) as i32;
        }
    }

    let target_outer_width = target_client_width + frame_width;
    let target_outer_height = target_client_height + frame_height;
    apply_target_outer_size(rect, edge, target_outer_width, target_outer_height);
}

fn apply_target_outer_size(rect: &mut RECT, edge: WPARAM, width: i32, height: i32) {
    match edge.0 as u32 {
        WMSZ_LEFT => rect.left = rect.right - width,
        WMSZ_RIGHT => rect.right = rect.left + width,
        WMSZ_TOP => rect.top = rect.bottom - height,
        WMSZ_BOTTOM => rect.bottom = rect.top + height,
        WMSZ_TOPLEFT => {
            rect.left = rect.right - width;
            rect.top = rect.bottom - height;
        }
        WMSZ_TOPRIGHT => {
            rect.right = rect.left + width;
            rect.top = rect.bottom - height;
        }
        WMSZ_BOTTOMLEFT => {
            rect.left = rect.right - width;
            rect.bottom = rect.top + height;
        }
        WMSZ_BOTTOMRIGHT => {
            rect.right = rect.left + width;
            rect.bottom = rect.top + height;
        }
        _ => {}
    }
}

fn is_horizontal_edge_only(edge: WPARAM) -> bool {
    matches!(edge.0 as u32, WMSZ_LEFT | WMSZ_RIGHT)
}

fn is_vertical_edge_only(edge: WPARAM) -> bool {
    matches!(edge.0 as u32, WMSZ_TOP | WMSZ_BOTTOM)
}

fn choose_resize_axis(
    edge: WPARAM,
    requested_width: i32,
    requested_height: i32,
    aspect_width: u32,
    aspect_height: u32,
) -> ResizeAxis {
    if is_vertical_edge_only(edge) {
        return ResizeAxis::Height;
    }
    if is_horizontal_edge_only(edge) {
        return ResizeAxis::Width;
    }

    let width_scale = i64::from(requested_width.max(1)) * i64::from(aspect_height.max(1));
    let height_scale = i64::from(requested_height.max(1)) * i64::from(aspect_width.max(1));

    if width_scale <= height_scale { ResizeAxis::Width } else { ResizeAxis::Height }
}

fn rect_width(rect: &RECT) -> i32 {
    rect.right - rect.left
}

fn rect_height(rect: &RECT) -> i32 {
    rect.bottom - rect.top
}

fn call_original_wndproc(
    old_wndproc: isize,
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let proc = unsafe {
        std::mem::transmute::<
            isize,
            Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>,
        >(old_wndproc)
    };
    unsafe { CallWindowProcW(proc, hwnd, message, wparam, lparam) }
}

fn create_swap_chain(
    device: &ID3D11Device,
    hwnd: HWND,
    size: PhysicalSize<u32>,
) -> Result<IDXGISwapChain1> {
    let dxgi_device: IDXGIDevice = device.cast()?;
    let adapter = unsafe { dxgi_device.GetAdapter() }?;
    let factory: IDXGIFactory2 = unsafe { adapter.GetParent() }?;
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: size.width.max(1),
        Height: size.height.max(1),
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        Scaling: DXGI_SCALING_STRETCH,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
        Flags: 0,
    };

    unsafe { factory.CreateSwapChainForHwnd(device, hwnd, &desc, None, None) }.map_err(Into::into)
}

fn create_render_target_view(
    device: &ID3D11Device,
    swap_chain: &IDXGISwapChain1,
) -> Result<ID3D11RenderTargetView> {
    let back_buffer: ID3D11Texture2D = unsafe { swap_chain.GetBuffer(0) }?;
    let mut view = None;
    unsafe {
        device.CreateRenderTargetView(&back_buffer, None, Some(&mut view))?;
    }
    view.context("CreateRenderTargetView returned no view")
}

fn recreate_swap_chain_target(
    device: &ID3D11Device,
    swap_chain: &IDXGISwapChain1,
    width: u32,
    height: u32,
) -> Result<ID3D11RenderTargetView> {
    unsafe {
        swap_chain.ResizeBuffers(
            0,
            width.max(1),
            height.max(1),
            DXGI_FORMAT_B8G8R8A8_UNORM,
            DXGI_SWAP_CHAIN_FLAG(0),
        )
    }?;
    create_render_target_view(device, swap_chain)
}

fn create_shader_resource_view(
    device: &ID3D11Device,
    texture: &ID3D11Texture2D,
) -> Result<ID3D11ShaderResourceView> {
    let mut srv = None;
    unsafe {
        device.CreateShaderResourceView(texture, None, Some(&mut srv))?;
    }
    srv.context("CreateShaderResourceView returned no view")
}

fn compile_vertex_shader(device: &ID3D11Device) -> Result<ID3D11VertexShader> {
    let source = br#"
struct VSOut {
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
};

VSOut main(uint vertex_id : SV_VertexID) {
    float2 positions[3] = {
        float2(-1.0, -3.0),
        float2(-1.0,  1.0),
        float2( 3.0,  1.0)
    };
    float2 uvs[3] = {
        float2(0.0, 2.0),
        float2(0.0, 0.0),
        float2(2.0, 0.0)
    };
    VSOut outv;
    outv.position = float4(positions[vertex_id], 0.0, 1.0);
    outv.uv = uvs[vertex_id];
    return outv;
}
"#;
    let blob = compile_shader_blob(source, b"main\0", b"vs_5_0\0")?;
    let mut shader = None;
    unsafe {
        device.CreateVertexShader(shader_bytes(&blob), None, Some(&mut shader))?;
    }
    shader.context("CreateVertexShader returned no shader")
}

fn compile_pixel_shader(device: &ID3D11Device) -> Result<ID3D11PixelShader> {
    let source = br#"
Texture2D relay_texture : register(t0);
SamplerState relay_sampler : register(s0);

cbuffer CursorOverlay : register(b0) {
    float2 cursor_position;
    float cursor_visible;
    float cursor_radius_px;
};

struct VSOut {
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
};

float4 main(VSOut input) : SV_TARGET {
    float4 base = relay_texture.Sample(relay_sampler, input.uv);
    if (cursor_visible < 0.5) {
        return base;
    }

    uint width;
    uint height;
    relay_texture.GetDimensions(width, height);
    float2 delta_px = (input.uv - cursor_position) * float2(width, height);
    float distance_px = length(delta_px);
    float ring_outer = smoothstep(cursor_radius_px + 3.0, cursor_radius_px - 3.0, distance_px);
    float ring_inner = smoothstep(cursor_radius_px - 5.0, cursor_radius_px - 9.0, distance_px);
    float cross_x = smoothstep(2.0, 0.0, abs(delta_px.x)) * smoothstep(cursor_radius_px + 6.0, cursor_radius_px - 6.0, abs(delta_px.y));
    float cross_y = smoothstep(2.0, 0.0, abs(delta_px.y)) * smoothstep(cursor_radius_px + 6.0, cursor_radius_px - 6.0, abs(delta_px.x));
    float marker = saturate((ring_outer - ring_inner) + cross_x + cross_y);
    float3 mixed = lerp(base.rgb, float3(1.0, 0.12, 0.12), marker * 0.9);
    return float4(mixed, base.a);
}
"#;
    let blob = compile_shader_blob(source, b"main\0", b"ps_5_0\0")?;
    let mut shader = None;
    unsafe {
        device.CreatePixelShader(shader_bytes(&blob), None, Some(&mut shader))?;
    }
    shader.context("CreatePixelShader returned no shader")
}

fn compile_shader_blob(source: &[u8], entry: &[u8], target: &[u8]) -> Result<ID3DBlob> {
    let mut code = None;
    let mut errors = None;
    let result = unsafe {
        D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            PCSTR::null(),
            None,
            None,
            PCSTR(entry.as_ptr()),
            PCSTR(target.as_ptr()),
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };

    match result {
        Ok(()) => code.context("D3DCompile returned no shader blob"),
        Err(error) => {
            let message = errors
                .as_ref()
                .map(|blob| String::from_utf8_lossy(shader_bytes(blob)).into_owned())
                .unwrap_or_else(|| error.to_string());
            bail!("Shader compilation failed: {message}")
        }
    }
}

fn shader_bytes(blob: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(blob.GetBufferPointer().cast(), blob.GetBufferSize()) }
}

fn create_sampler(device: &ID3D11Device) -> Result<ID3D11SamplerState> {
    let desc = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE_ADDRESS_CLAMP,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: windows::Win32::Graphics::Direct3D11::D3D11_COMPARISON_NEVER,
        BorderColor: [0.0, 0.0, 0.0, 0.0],
        MinLOD: 0.0,
        MaxLOD: f32::MAX,
    };
    let mut sampler = None;
    unsafe {
        device.CreateSamplerState(&desc, Some(&mut sampler))?;
    }
    sampler.context("CreateSamplerState returned no sampler")
}

fn create_cursor_buffer(device: &ID3D11Device) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: std::mem::size_of::<CursorOverlay>() as u32,
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: (&CursorOverlay::hidden() as *const CursorOverlay).cast(),
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut buffer = None;
    unsafe {
        device.CreateBuffer(&desc, Some(&data), Some(&mut buffer))?;
    }
    buffer.context("CreateBuffer returned no cursor buffer")
}

fn update_cursor_buffer(
    context: &ID3D11DeviceContext,
    buffer: &ID3D11Buffer,
    cursor: CursorOverlay,
) {
    unsafe {
        context.UpdateSubresource(buffer, 0, None, (&cursor as *const CursorOverlay).cast(), 0, 0);
    }
}

fn fitted_viewport(
    window_size: PhysicalSize<u32>,
    content_size: PhysicalSize<u32>,
) -> D3D11_VIEWPORT {
    let window_width = window_size.width.max(1) as f32;
    let window_height = window_size.height.max(1) as f32;
    let content_width = content_size.width.max(1) as f32;
    let content_height = content_size.height.max(1) as f32;
    let scale = (window_width / content_width).min(window_height / content_height);
    let fitted_width = (content_width * scale).max(1.0);
    let fitted_height = (content_height * scale).max(1.0);

    D3D11_VIEWPORT {
        TopLeftX: ((window_width - fitted_width) * 0.5).max(0.0),
        TopLeftY: ((window_height - fitted_height) * 0.5).max(0.0),
        Width: fitted_width,
        Height: fitted_height,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    }
}

fn reduce_ratio(width: u32, height: u32) -> (u32, u32) {
    let divisor = gcd(width.max(1), height.max(1));
    (width / divisor, height / divisor)
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }

    a.max(1)
}
