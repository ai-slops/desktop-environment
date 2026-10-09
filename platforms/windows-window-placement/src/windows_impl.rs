use anyhow::{Context, Result, bail};
use desktop_presets::WindowPlacement;
use raw_window_handle::{RawWindowHandle, WindowHandle};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowPlacement, IsIconic, SW_SHOWMAXIMIZED, SW_SHOWNORMAL, SetWindowPlacement,
    WINDOWPLACEMENT,
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
    };
    placement.validate()?;
    Ok(Some(placement))
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
        let placement = WindowPlacement { normal_rect: [100, 120, 760, 640], maximized: false };
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
        let invalid = WindowPlacement { normal_rect: [50, 50, 40, 40], maximized: false };
        assert!(restore(window.borrowed()?, invalid).is_err());
        assert_eq!(capture(window.borrowed()?)?, original);
        Ok(())
    }
}
