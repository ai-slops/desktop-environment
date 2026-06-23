use anyhow::{Context, Result, bail};
use display_relay_core::{PointerSample, RelayConfig};
use std::sync::Arc;
use tracing::{error, warn};
use windows_desktop_duplication::{CaptureFrameView, DesktopDuplicator, enumerate_displays};
use windows_input::{InjectedKeyEvent, MouseButton, RemoteInputController};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton as WinitMouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
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
                "Usage: display-relay list | mirror <DISPLAY_NAME> [--fullscreen] [--timeout-ms N]"
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
    renderer: Option<GpuRenderer>,
    last_pointer_position: Option<(f32, f32)>,
    last_window_size: Option<PhysicalSize<u32>>,
}

impl RelayApp {
    fn new(config: RelayConfig) -> Result<Self> {
        let duplicator = DesktopDuplicator::new(&config.target.display_name)?;
        let input = RemoteInputController::new(duplicator.display_info().virtual_desktop);

        Ok(Self {
            config,
            duplicator,
            input,
            window: None,
            window_id: None,
            renderer: None,
            last_pointer_position: None,
            last_window_size: None,
        })
    }

    fn constrained_window_size(&self, requested: PhysicalSize<u32>) -> PhysicalSize<u32> {
        let source_area = self.duplicator.display_info().area;
        let aspect_ratio = f64::from(source_area.width) / f64::from(source_area.height);

        let requested_width = requested.width.max(1);
        let requested_height = requested.height.max(1);
        let height_from_width = ((f64::from(requested_width) / aspect_ratio).round() as u32).max(1);
        let width_from_height =
            ((f64::from(requested_height) * aspect_ratio).round() as u32).max(1);

        let previous = self.last_window_size.unwrap_or(requested);
        let width_delta = requested_width.abs_diff(previous.width);
        let height_delta = requested_height.abs_diff(previous.height);

        if width_delta >= height_delta {
            PhysicalSize::new(requested_width, height_from_width)
        } else {
            PhysicalSize::new(width_from_height, requested_height)
        }
    }

    fn redraw(&mut self) -> Result<()> {
        let frame = self.duplicator.capture_frame(self.config.capture_timeout_ms);
        let renderer = self.renderer.as_mut().context("renderer not ready")?;
        let window = self.window.as_ref().context("window not ready")?;

        let captured = match frame {
            Ok(frame) => frame,
            Err(error) if error.to_string().contains("Timed out waiting for the next frame") => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };

        renderer.render_frame(&captured)?;
        window.request_redraw();
        Ok(())
    }

    fn handle_pointer_move(&mut self, x: f32, y: f32) {
        self.last_pointer_position = Some((x, y));

        let Some(window) = self.window.as_ref() else {
            return;
        };
        let size = window.inner_size();

        let mapped = self.duplicator.display_info().virtual_desktop.map_window_pointer(
            self.duplicator.display_info().area,
            (size.width, size.height),
            PointerSample { x, y },
        );

        if let Some((desktop_x, desktop_y)) = mapped {
            if let Err(error) = self.input.move_mouse(desktop_x, desktop_y) {
                warn!("Failed to move remote pointer: {error:#}");
            }
        }
    }

    fn handle_mouse_button(&mut self, button: WinitMouseButton, state: ElementState) {
        let Some((x, y)) = self.last_pointer_position else {
            return;
        };
        self.handle_pointer_move(x, y);

        let button = match button {
            WinitMouseButton::Left => MouseButton::Left,
            WinitMouseButton::Right => MouseButton::Right,
            WinitMouseButton::Middle => MouseButton::Middle,
            _ => return,
        };

        if let Err(error) = self.input.click(button, state == ElementState::Pressed) {
            warn!("Failed to forward mouse button: {error:#}");
        }
    }

    fn handle_key(&self, event: &KeyEvent) {
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };

        if code == KeyCode::Escape && event.state == ElementState::Pressed {
            return;
        }

        let Some(scan_code) = keycode_to_set1_scancode(code) else {
            return;
        };

        if let Err(error) = self
            .input
            .send_key(InjectedKeyEvent { scan_code, pressed: event.state == ElementState::Pressed })
        {
            warn!("Failed to forward keyboard input: {error:#}");
        }
    }
}

impl ApplicationHandler for RelayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let display = self.duplicator.display_info();
        let (resize_step_width, resize_step_height) =
            reduce_ratio(display.area.width.max(1), display.area.height.max(1));
        let mut attributes = WindowAttributes::default()
            .with_title(format!("Relay {}", display.name))
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
        let renderer = match pollster::block_on(GpuRenderer::new(
            Arc::clone(&window),
            self.duplicator.display_info().area.width,
            self.duplicator.display_info().area.height,
        )) {
            Ok(renderer) => renderer,
            Err(error) => {
                error!("Failed to create GPU renderer: {error:#}");
                event_loop.exit();
                return;
            }
        };

        if let Err(error) = renderer.configure_surface_for_current_size() {
            error!("Failed to configure GPU surface: {error:#}");
            event_loop.exit();
            return;
        }

        self.window_id = Some(window.id());
        self.last_window_size = Some(window.inner_size());
        self.renderer = Some(renderer);
        self.window = Some(window);
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
                let constrained = self.constrained_window_size(size);
                if constrained != size {
                    if let Some(window) = self.window.as_ref() {
                        let _ = window.request_inner_size(constrained);
                    }
                    return;
                }

                self.last_window_size = Some(size);
                if let Some(renderer) = self.renderer.as_mut() {
                    if let Err(error) = renderer.resize(size.width, size.height) {
                        error!("Failed to resize relay surface: {error:#}");
                        event_loop.exit();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.handle_pointer_move(position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.handle_mouse_button(button, state);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                    && event.state == ElementState::Pressed
                {
                    event_loop.exit();
                } else {
                    self.handle_key(&event);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.redraw() {
                    error!("Relay redraw failed: {error:#}");
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

struct GpuRenderer {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    capture_texture: wgpu::Texture,
    capture_texture_size: wgpu::Extent3d,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl GpuRenderer {
    async fn new(window: Arc<Window>, capture_width: u32, capture_height: u32) -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(Arc::clone(&window))?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .context("No suitable GPU adapter found for the relay window")?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("display-relay-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(surface_caps.formats[0]);
        let present_mode = surface_caps
            .present_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::PresentMode::AutoNoVsync)
            .unwrap_or(surface_caps.present_modes[0]);
        let alpha_mode = surface_caps.alpha_modes[0];

        let window_size = window.inner_size();
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: window_size.width.max(1),
            height: window_size.height.max(1),
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        let capture_texture_size = wgpu::Extent3d {
            width: capture_width.max(1),
            height: capture_height.max(1),
            depth_or_array_layers: 1,
        };
        let capture_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("display-relay-capture-texture"),
            size: capture_texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let capture_texture_view =
            capture_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("display-relay-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("display-relay-shader"),
            source: wgpu::ShaderSource::Wgsl(RELAY_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("display-relay-bind-group-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("display-relay-bind-group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&capture_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("display-relay-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("display-relay-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Ok(Self {
            window,
            surface,
            device,
            queue,
            surface_config,
            capture_texture,
            capture_texture_size,
            bind_group,
            pipeline,
        })
    }

    fn configure_surface_for_current_size(&self) -> Result<()> {
        if self.surface_config.width == 0 || self.surface_config.height == 0 {
            bail!("Relay window surface size is zero");
        }

        self.surface.configure(&self.device, &self.surface_config);
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }

        self.surface_config.width = width;
        self.surface_config.height = height;
        self.configure_surface_for_current_size()
    }

    fn render_frame(&mut self, frame: &CaptureFrameView<'_>) -> Result<()> {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.capture_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            frame.pixels_bgra,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.width * 4),
                rows_per_image: Some(frame.height),
            },
            self.capture_texture_size,
        );

        let output = match self.surface.get_current_texture() {
            Ok(output) => output,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.resize(self.window.inner_size().width, self.window.inner_size().height)?;
                return Ok(());
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(wgpu::SurfaceError::OutOfMemory) => bail!("GPU surface ran out of memory"),
            Err(other) => {
                return Err(anyhow::anyhow!("Failed to acquire surface texture: {other}"));
            }
        };

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("display-relay-encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("display-relay-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }

        self.queue.submit([encoder.finish()]);
        output.present();
        Ok(())
    }
}

const RELAY_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 2.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(2.0, 0.0),
    );
    let position = positions[vertex_index];
    var out: VertexOutput;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.uv = uvs[vertex_index];
    return out;
}

@group(0) @binding(0)
var relay_texture: texture_2d<f32>;

@group(0) @binding(1)
var relay_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(relay_texture, relay_sampler, in.uv);
}
"#;

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

fn keycode_to_set1_scancode(code: KeyCode) -> Option<u16> {
    Some(match code {
        KeyCode::KeyA => 0x1E,
        KeyCode::KeyB => 0x30,
        KeyCode::KeyC => 0x2E,
        KeyCode::KeyD => 0x20,
        KeyCode::KeyE => 0x12,
        KeyCode::KeyF => 0x21,
        KeyCode::KeyG => 0x22,
        KeyCode::KeyH => 0x23,
        KeyCode::KeyI => 0x17,
        KeyCode::KeyJ => 0x24,
        KeyCode::KeyK => 0x25,
        KeyCode::KeyL => 0x26,
        KeyCode::KeyM => 0x32,
        KeyCode::KeyN => 0x31,
        KeyCode::KeyO => 0x18,
        KeyCode::KeyP => 0x19,
        KeyCode::KeyQ => 0x10,
        KeyCode::KeyR => 0x13,
        KeyCode::KeyS => 0x1F,
        KeyCode::KeyT => 0x14,
        KeyCode::KeyU => 0x16,
        KeyCode::KeyV => 0x2F,
        KeyCode::KeyW => 0x11,
        KeyCode::KeyX => 0x2D,
        KeyCode::KeyY => 0x15,
        KeyCode::KeyZ => 0x2C,
        KeyCode::Digit0 => 0x0B,
        KeyCode::Digit1 => 0x02,
        KeyCode::Digit2 => 0x03,
        KeyCode::Digit3 => 0x04,
        KeyCode::Digit4 => 0x05,
        KeyCode::Digit5 => 0x06,
        KeyCode::Digit6 => 0x07,
        KeyCode::Digit7 => 0x08,
        KeyCode::Digit8 => 0x09,
        KeyCode::Digit9 => 0x0A,
        KeyCode::Space => 0x39,
        KeyCode::Enter => 0x1C,
        KeyCode::Tab => 0x0F,
        KeyCode::Backspace => 0x0E,
        KeyCode::ArrowUp => 0x48,
        KeyCode::ArrowDown => 0x50,
        KeyCode::ArrowLeft => 0x4B,
        KeyCode::ArrowRight => 0x4D,
        KeyCode::ShiftLeft => 0x2A,
        KeyCode::ShiftRight => 0x36,
        KeyCode::ControlLeft => 0x1D,
        KeyCode::AltLeft => 0x38,
        KeyCode::Escape => 0x01,
        _ => return None,
    })
}
