use anyhow::{Result, anyhow, bail};
use display_relay_core::VirtualDesktop;
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

#[derive(Debug, Clone, Copy)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy)]
pub struct InjectedKeyEvent {
    pub scan_code: u16,
    pub pressed: bool,
}

pub struct RemoteInputController {
    desktop: VirtualDesktop,
}

impl RemoteInputController {
    #[must_use]
    pub fn new(desktop: VirtualDesktop) -> Self {
        Self { desktop }
    }

    pub fn cursor_position(&self) -> Result<(i32, i32)> {
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point) }?;
        Ok((point.x, point.y))
    }

    pub fn move_mouse(&self, x: i32, y: i32) -> Result<()> {
        let (absolute_x, absolute_y) = self
            .desktop
            .absolute_mouse(x, y)
            .ok_or_else(|| anyhow!("Point ({x}, {y}) is outside the virtual desktop"))?;

        self.send_input(&[INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: absolute_x,
                    dy: absolute_y,
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }])
    }

    pub fn click(&self, button: MouseButton, pressed: bool) -> Result<()> {
        let flag = match (button, pressed) {
            (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
            (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
            (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
            (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
            (MouseButton::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
            (MouseButton::Middle, false) => MOUSEEVENTF_MIDDLEUP,
        };

        self.send_input(&[INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: flag,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }])
    }

    pub fn send_key(&self, event: InjectedKeyEvent) -> Result<()> {
        let mut flags = KEYEVENTF_SCANCODE;
        if !event.pressed {
            flags |= KEYEVENTF_KEYUP;
        }

        self.send_input(&[INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: event.scan_code,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }])
    }

    fn send_input(&self, inputs: &[INPUT]) -> Result<()> {
        let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent == inputs.len() as u32 {
            Ok(())
        } else if sent == 0 {
            bail!("SendInput failed")
        } else {
            bail!("Only sent {sent} of {} requested input events", inputs.len())
        }
    }
}
