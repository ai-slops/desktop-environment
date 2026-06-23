use anyhow::{Context, Result, bail};
use display_relay_core::{PointerSample, RelayConfig};
use raw_window_handle::{DisplayHandle, HasDisplayHandle};
use softbuffer::{Context as SoftbufferContext, Surface};
use std::num::NonZeroU32;
use std::sync::Arc;
use tracing::{error, warn};
use windows_desktop_duplication::{CapturedFrame, DesktopDuplicator, enumerate_displays};
use windows_input::{InjectedKeyEvent, MouseButton, RemoteInputController};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
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
    softbuffer_context: Option<SoftbufferContext<DisplayHandle<'static>>>,
    surface: Option<Surface<DisplayHandle<'static>, Arc<Window>>>,
    last_pointer_position: Option<(f32, f32)>,
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
            softbuffer_context: None,
            surface: None,
            last_pointer_position: None,
        })
    }

    fn redraw(&mut self) -> Result<()> {
        let frame = self.duplicator.capture_frame(self.config.capture_timeout_ms);
        let surface = self.surface.as_mut().context("surface not ready")?;
        let window = self.window.as_ref().context("window not ready")?;

        let captured = match frame {
            Ok(frame) => frame,
            Err(error) if error.to_string().contains("Timed out waiting for the next frame") => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };

        render_frame(surface, window, captured)?;
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
        let mut attributes = WindowAttributes::default()
            .with_title(format!("Relay {}", display.name))
            .with_inner_size(LogicalSize::new(
                f64::from(display.area.width),
                f64::from(display.area.height),
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

        let display_handle = match event_loop.display_handle() {
            Ok(handle) => handle,
            Err(error) => {
                error!("Failed to fetch display handle: {error}");
                event_loop.exit();
                return;
            }
        };

        let display_handle = unsafe {
            std::mem::transmute::<DisplayHandle<'_>, DisplayHandle<'static>>(display_handle)
        };

        let context = match SoftbufferContext::new(display_handle) {
            Ok(context) => context,
            Err(error) => {
                error!("Failed to create softbuffer context: {error}");
                event_loop.exit();
                return;
            }
        };

        let window = Arc::new(window);

        let mut surface = match Surface::new(&context, Arc::clone(&window)) {
            Ok(surface) => surface,
            Err(error) => {
                error!("Failed to create softbuffer surface: {error}");
                event_loop.exit();
                return;
            }
        };

        let size = window.inner_size();
        if let Err(error) = surface.resize(
            NonZeroU32::new(size.width.max(1)).expect("non-zero width"),
            NonZeroU32::new(size.height.max(1)).expect("non-zero height"),
        ) {
            error!("Failed to resize softbuffer surface: {error}");
            event_loop.exit();
            return;
        }

        self.window_id = Some(window.id());
        self.softbuffer_context = Some(context);
        self.surface = Some(surface);
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
                if let Some(surface) = self.surface.as_mut() {
                    if let Err(error) = surface.resize(
                        NonZeroU32::new(size.width.max(1)).expect("non-zero width"),
                        NonZeroU32::new(size.height.max(1)).expect("non-zero height"),
                    ) {
                        error!("Failed to resize relay surface: {error}");
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

fn render_frame(
    surface: &mut Surface<DisplayHandle<'static>, Arc<Window>>,
    window: &Window,
    frame: CapturedFrame,
) -> Result<()> {
    let width = frame.width as usize;
    let height = frame.height as usize;
    let mut buffer = surface.buffer_mut().map_err(|error| anyhow::anyhow!("{error}"))?;
    let output_size = window.inner_size();
    let target_width = output_size.width as usize;
    let target_height = output_size.height as usize;

    if target_width == 0 || target_height == 0 {
        return Ok(());
    }

    for y in 0..target_height {
        let source_y = (y * height) / target_height;
        for x in 0..target_width {
            let source_x = (x * width) / target_width;
            let source_offset = ((source_y * width) + source_x) * 4;
            let b = frame.pixels_bgra[source_offset];
            let g = frame.pixels_bgra[source_offset + 1];
            let r = frame.pixels_bgra[source_offset + 2];
            buffer[(y * target_width) + x] = u32::from_be_bytes([0, r, g, b]);
        }
    }

    buffer.present().map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(())
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
