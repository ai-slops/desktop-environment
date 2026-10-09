use anyhow::{Context, Result, bail};
use desktop_presets::WindowPlacement;
use raw_window_handle::{RawWindowHandle, WindowHandle};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetMenu, GetWindowLongPtrW, GetWindowPlacement, IsIconic,
    SW_SHOWMAXIMIZED, SW_SHOWNORMAL, SetWindowPlacement, WINDOW_EX_STYLE, WINDOW_STYLE,
    WINDOWPLACEMENT, WS_MAXIMIZE, WS_MINIMIZE,
};

fn hwnd(window: WindowHandle<'_>) -> Result<HWND> {
    match window.as_raw() {
        RawWindowHandle::Win32(handle) => Ok(HWND(handle.hwnd.get() as *mut std::ffi::c_void)),
        _ => bail!("Windows 창 핸들이 필요합니다."),
    }
}

fn native_placement() -> Result<WINDOWPLACEMENT> {
    Ok(WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>().try_into()?,
        ..Default::default()
    })
}

pub fn capture(window: WindowHandle<'_>) -> Result<Option<WindowPlacement>> {
    let hwnd = hwnd(window)?;
    // SAFETY: the borrowed WindowHandle guarantees a live window for this call.
    if unsafe { IsIconic(hwnd).as_bool() } {
        return Ok(None);
    }
    let mut native = native_placement()?;
    // SAFETY: native is a writable, correctly sized WINDOWPLACEMENT; hwnd stays live.
    unsafe { GetWindowPlacement(hwnd, &raw mut native) }
        .context("창 크기와 위치를 읽을 수 없습니다.")?;
    let rect = native.rcNormalPosition;
    let placement = WindowPlacement {
        normal_rect: [rect.left, rect.top, rect.right, rect.bottom],
        maximized: native.showCmd == SW_SHOWMAXIMIZED.0.cast_unsigned(),
        normal_client_size: None,
    };
    placement.validate()?;
    Ok(Some(placement))
}

/// Capture the normal client size in physical pixels for video mirror windows.
pub fn capture_physical_client(window: WindowHandle<'_>) -> Result<Option<WindowPlacement>> {
    let Some(mut placement) = capture(window)? else { return Ok(None) };
    let frame = normal_frame_size(hwnd(window)?)?;
    let [left, top, right, bottom] = placement.normal_rect;
    placement.normal_client_size = Some([
        u32::try_from(i64::from(right) - i64::from(left) - frame[0])?,
        u32::try_from(i64::from(bottom) - i64::from(top) - frame[1])?,
    ]);
    placement.validate()?;
    Ok(Some(placement))
}

/// Restore the saved pixel client size after the window reaches its saved monitor.
pub fn restore_physical_client(window: WindowHandle<'_>, placement: WindowPlacement) -> Result<()> {
    restore(window, placement)?;
    let Some(size) = placement.normal_client_size else { return Ok(()) };
    // Moving during restore can change DPI. Query the target decorations afterwards.
    let frame = normal_frame_size(hwnd(window)?)?;
    let mut restored = capture(window)?.context("복원한 창이 최소화되어 있습니다.")?;
    let [left, top, _, _] = restored.normal_rect;
    restored.normal_rect = [
        left,
        top,
        i32::try_from(i64::from(left) + i64::from(size[0]) + frame[0])?,
        i32::try_from(i64::from(top) + i64::from(size[1]) + frame[1])?,
    ];
    restore(window, restored)
}

fn normal_frame_size(hwnd: HWND) -> Result<[i64; 2]> {
    // SAFETY: all queries use a borrowed, live HWND and a writable local rectangle.
    let (style, extended_style, menu, dpi) = unsafe {
        (
            GetWindowLongPtrW(hwnd, GWL_STYLE),
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE),
            GetMenu(hwnd),
            GetDpiForWindow(hwnd),
        )
    };
    let style = WINDOW_STYLE(u32::try_from(style)? & !(WS_MAXIMIZE.0 | WS_MINIMIZE.0));
    let extended_style = WINDOW_EX_STYLE(u32::try_from(extended_style)?);
    if dpi == 0 {
        bail!("창 DPI를 읽을 수 없습니다.");
    }
    let mut frame = RECT::default();
    unsafe {
        AdjustWindowRectExForDpi(&raw mut frame, style, !menu.0.is_null(), extended_style, dpi)
    }?;
    Ok([
        i64::from(frame.right) - i64::from(frame.left),
        i64::from(frame.bottom) - i64::from(frame.top),
    ])
}

pub fn restore(window: WindowHandle<'_>, placement: WindowPlacement) -> Result<()> {
    placement.validate()?;
    let hwnd = hwnd(window)?;
    let [left, top, right, bottom] = placement.normal_rect;
    let native = WINDOWPLACEMENT {
        showCmd: if placement.maximized { SW_SHOWMAXIMIZED } else { SW_SHOWNORMAL }
            .0
            .cast_unsigned(),
        rcNormalPosition: RECT { left, top, right, bottom },
        ..native_placement()?
    };
    // SAFETY: the handle is live, bounds were validated, and the structure has the
    // required length. SetWindowPlacement also recovers fully off-screen windows.
    unsafe { SetWindowPlacement(hwnd, &raw const native) }
        .context("저장한 창 크기와 위치를 복원할 수 없습니다.")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use raw_window_handle::Win32WindowHandle;
    use std::num::NonZeroIsize;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SHOW_WINDOW_CMD, SW_MINIMIZE, ShowWindow, WINDOW_EX_STYLE,
        WS_OVERLAPPEDWINDOW,
    };
    use windows::core::w;

    struct TestWindow(HWND);

    impl TestWindow {
        fn new() -> Result<Self> {
            // SAFETY: STATIC is a built-in class. No pointers or callback state escape.
            let handle = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    w!("Owned placement test"),
                    WS_OVERLAPPEDWINDOW,
                    80,
                    90,
                    500,
                    400,
                    None,
                    None,
                    None,
                    None,
                )
            }?;
            Ok(Self(handle))
        }

        fn borrowed(&self) -> Result<WindowHandle<'_>> {
            let handle = NonZeroIsize::new(self.0.0 as isize).context("null test handle")?;
            // SAFETY: self owns this window until Drop, so the returned borrow is bounded.
            Ok(unsafe {
                WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(handle)))
            })
        }

        fn show(&self, command: SHOW_WINDOW_CMD) {
            // SAFETY: the test owns this still-live window.
            let _ = unsafe { ShowWindow(self.0, command) };
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            // SAFETY: this window was created and is destroyed on this test thread.
            let _ = unsafe { DestroyWindow(self.0) };
        }
    }

    #[test]
    fn normal_and_maximized_geometry_round_trip_without_saving_minimized_bounds() -> Result<()> {
        let window = TestWindow::new()?;
        let placement = WindowPlacement {
            normal_rect: [100, 120, 760, 640],
            maximized: false,
            normal_client_size: None,
        };
        restore(window.borrowed()?, placement)?;
        assert_eq!(capture(window.borrowed()?)?, Some(placement));
        let maximized = WindowPlacement { maximized: true, ..placement };
        restore(window.borrowed()?, maximized)?;
        assert_eq!(capture(window.borrowed()?)?, Some(maximized));
        window.show(SW_MINIMIZE);
        assert_eq!(capture(window.borrowed()?)?, None);
        window.show(SW_SHOWNORMAL);
        restore(window.borrowed()?, placement)?;
        assert_eq!(capture(window.borrowed()?)?, Some(placement));
        Ok(())
    }

    #[test]
    fn invalid_geometry_is_rejected_before_windows_is_modified() -> Result<()> {
        let window = TestWindow::new()?;
        let original = capture(window.borrowed()?)?;
        let invalid = WindowPlacement {
            normal_rect: [50, 50, 40, 40],
            maximized: false,
            normal_client_size: None,
        };
        assert!(restore(window.borrowed()?, invalid).is_err());
        assert_eq!(capture(window.borrowed()?)?, original);
        Ok(())
    }

    #[test]
    fn physical_client_size_overrides_differently_scaled_outer_bounds() -> Result<()> {
        use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
        let window = TestWindow::new()?;
        // The saved outer size can have come from a different monitor's decorations.
        let placement = WindowPlacement {
            normal_rect: [100, 120, 1080, 720],
            maximized: false,
            normal_client_size: Some([960, 540]),
        };
        restore_physical_client(window.borrowed()?, placement)?;
        let mut rect = RECT::default();
        unsafe { GetClientRect(window.0, &raw mut rect) }?;
        assert_eq!((rect.right - rect.left, rect.bottom - rect.top), (960, 540));
        let saved = capture_physical_client(window.borrowed()?)?.context("Missing placement")?;
        assert_eq!(saved.normal_client_size, Some([960, 540]));
        assert_eq!(saved.normal_rect[..2], placement.normal_rect[..2]);
        let maximized = WindowPlacement { maximized: true, ..saved };
        restore_physical_client(window.borrowed()?, maximized)?;
        let maximized_saved =
            capture_physical_client(window.borrowed()?)?.context("Missing maximized placement")?;
        assert!(maximized_saved.maximized);
        assert_eq!(maximized_saved.normal_client_size, Some([960, 540]));
        window.show(SW_MINIMIZE);
        assert_eq!(capture_physical_client(window.borrowed()?)?, None);
        Ok(())
    }
}
