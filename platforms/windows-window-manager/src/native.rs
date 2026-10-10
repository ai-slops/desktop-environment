// Win32 pointer-sized handles/style bitfields require ABI casts; structure sizes
// and shortcut numbers are bounded by their native types and configuration validation.
#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap, clippy::cast_sign_loss)]
use crate::{Candidate, EventStream, Journal};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use window_manager_core::{
    Binding, Display, Error, ErrorCode, Id, Mutation, ObservedWindow, Rect, Result, ShowState,
    new_id,
};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    DISPLAY_DEVICEW, EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR,
    MONITOR_DEFAULTTONEAREST, MONITORINFOEXW, MonitorFromWindow,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, GetDpiForWindow,
    MDT_EFFECTIVE_DPI, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_DESTROY, EVENT_OBJECT_LOCATIONCHANGE, EnumWindows, GW_OWNER, GWL_EXSTYLE,
    GWL_STYLE, GetClassNameW, GetClientRect, GetMessageW, GetPropW, GetWindow, GetWindowLongPtrW,
    GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
    IsZoomed, MSG, SWP_ASYNCWINDOWPOS, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetForegroundWindow, SetPropW,
    SetWindowPos, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_HOTKEY, WS_EX_TOOLWINDOW,
    WS_THICKFRAME,
};
use windows::core::{BOOL, PCWSTR};

fn native_error(code: ErrorCode, object: &str, error: impl std::fmt::Display) -> Error {
    Error::new(code, error.to_string(), object)
}
const fn handle(value: u64) -> HWND {
    HWND(value as usize as *mut std::ffi::c_void)
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
#[must_use]
pub fn foreground_handle() -> u64 {
    // SAFETY: foreground query does not change activation.
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow().0 as usize as u64 }
}
/// Only this process's own manager surface is relocated. No managed app's geometry is involved.
fn control_handle() -> Result<HWND> {
    unsafe extern "system" fn find(hwnd: HWND, data: LPARAM) -> BOOL {
        let mut process = 0;
        // SAFETY: callback receives a live enumerated HWND and our writable output pointer.
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&raw mut process));
        }
        if process == std::process::id() {
            let mut title = [0; 256];
            let len = unsafe { GetWindowTextW(hwnd, &mut title) };
            if len > 0 && string(&title) == "Window Manager" {
                unsafe {
                    *(data.0 as *mut HWND) = hwnd;
                }
                return BOOL(0);
            }
        }
        BOOL(1)
    }
    let _dpi = DpiGuard::new();
    let mut hwnd = HWND::default();
    // SAFETY: callback output outlives the synchronous enumeration.
    let _ = unsafe { EnumWindows(Some(find), LPARAM((&raw mut hwnd) as isize)) };
    if hwnd.0.is_null() {
        return Err(Error::new(
            ErrorCode::TargetMissing,
            "Control window not available",
            "control",
        ));
    }
    Ok(hwnd)
}

#[must_use]
pub fn control_fits(area: Rect) -> bool {
    let _dpi = DpiGuard::new();
    let Ok(hwnd) = control_handle() else {
        return false;
    };
    let mut frame = RECT::default();
    // SAFETY: query only for this process's own control surface.
    unsafe { GetWindowRect(hwnd, &raw mut frame) }.is_ok() && area.contains(rect(frame))
}

pub fn position_control(area: Rect) -> Result<()> {
    area.validate()?;
    let _dpi = DpiGuard::new();
    let hwnd = control_handle()?;
    let mut frame = RECT::default();
    unsafe { GetWindowRect(hwnd, &raw mut frame) }
        .map_err(|error| native_error(ErrorCode::UnsupportedOperation, "control", error))?;
    let frame = rect(frame);
    let initial_dpi = unsafe { GetDpiForWindow(hwnd) };
    place_control(hwnd, area, frame)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(900);
    let mut corrected = false;
    let mut stable = None;
    loop {
        let mut actual = RECT::default();
        // SAFETY: only the previously located window in this manager process is observed.
        if unsafe { GetWindowRect(hwnd, &raw mut actual) }.is_ok() {
            let actual = rect(actual);
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            if area.contains(actual) {
                match stable {
                    Some((previous, previous_dpi, started))
                        if actual == previous && dpi == previous_dpi =>
                    {
                        if std::time::Instant::now().duration_since(started)
                            >= std::time::Duration::from_millis(80)
                        {
                            return Ok(());
                        }
                    }
                    _ => stable = Some((actual, dpi, std::time::Instant::now())),
                }
            } else {
                stable = None;
                // WM_DPICHANGED can expand the own control window after its first asynchronous move.
                if !corrected
                    && dpi != initial_dpi
                    && (actual.width != frame.width || actual.height != frame.height)
                {
                    corrected = true;
                    place_control(hwnd, area, actual)?;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::new(
                ErrorCode::ApplicationTimeout,
                "Control could not fit private-designated area; content stays redacted",
                "control",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn place_control(hwnd: HWND, area: Rect, frame: Rect) -> Result<()> {
    let width = frame.width.min(area.width);
    let height = frame.height.min(area.height);
    let x = area.x + (area.width - width) / 2;
    let y = area.y + (area.height - height) / 2;
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS,
        )
    }
    .map_err(|error| native_error(ErrorCode::UnsupportedOperation, "control", error))
}
fn string(value: &[u16]) -> String {
    String::from_utf16_lossy(
        &value[..value.iter().position(|unit| *unit == 0).unwrap_or(value.len())],
    )
}
const fn rect(value: RECT) -> Rect {
    Rect {
        x: value.left,
        y: value.top,
        width: value.right - value.left,
        height: value.bottom - value.top,
    }
}

struct DpiGuard(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT);
impl DpiGuard {
    fn new() -> Self {
        // SAFETY: changes only this adapter thread's DPI context, restored by Drop.
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}
impl Drop for DpiGuard {
    fn drop(&mut self) {
        // SAFETY: this value was returned by SetThreadDpiAwarenessContext on the same thread.
        if !self.0.0.is_null() {
            unsafe { SetThreadDpiAwarenessContext(self.0) };
        }
    }
}

fn process_identity(pid: u32) -> Result<(u64, u32)> {
    // SAFETY: least-privileged process query, no process execution or memory access.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|error| native_error(ErrorCode::PermissionDenied, "process", error))?;
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let mut session = 0;
    // SAFETY: writable local FILETIME/session buffers, valid process handle.
    let times = unsafe {
        GetProcessTimes(process, &raw mut creation, &raw mut exit, &raw mut kernel, &raw mut user)
    };
    let session_result = unsafe { ProcessIdToSessionId(pid, &raw mut session) };
    let _ = unsafe { CloseHandle(process) };
    times.map_err(|error| native_error(ErrorCode::PermissionDenied, "process", error))?;
    session_result.map_err(|error| native_error(ErrorCode::PermissionDenied, "session", error))?;
    Ok(((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime), session))
}

pub fn bind(candidate: &Candidate, allow_hide: bool) -> Result<ObservedWindow> {
    bind_internal(candidate, allow_hide, None)
}

pub fn bind_mapped(
    candidate: &Candidate,
    allow_hide: bool,
    mappings: &crate::MonitorMappings,
) -> Result<ObservedWindow> {
    bind_internal(candidate, allow_hide, Some(mappings))
}

fn bind_internal(
    candidate: &Candidate,
    allow_hide: bool,
    mappings: Option<&crate::MonitorMappings>,
) -> Result<ObservedWindow> {
    static GENERATION: AtomicU64 = AtomicU64::new(1);
    let hwnd = handle(candidate.handle);
    let mut pid = 0;
    // SAFETY: query-only; the candidate is revalidated before registering a lifetime token.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
    if pid != candidate.process || pid == 0 {
        return Err(Error::new(ErrorCode::StaleBinding, "Candidate window was replaced", "window"));
    }
    let (started, session) = process_identity(pid)?;
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
    let property = format!("DesktopEnvironment.WindowManager.{}", new_id("lifetime"));
    let property_wide = wide(&property);
    // SAFETY: an opaque non-pointer token is copied into the native property list.
    // No dereference is performed; destroying the window removes its property list.
    unsafe {
        SetPropW(
            hwnd,
            PCWSTR(property_wide.as_ptr()),
            Some(HANDLE(generation as usize as *mut std::ffi::c_void)),
        )
    }
    .map_err(|error| native_error(ErrorCode::PermissionDenied, "window", error))?;
    let binding = Binding {
        handle: candidate.handle,
        process: pid,
        process_started: started,
        session,
        generation,
        token_property: property,
    };
    observe_internal(&binding, allow_hide, mappings)
}

pub fn validate_binding(binding: &Binding) -> Result<()> {
    let hwnd = handle(binding.handle);
    let property = wide(&binding.token_property);
    let mut pid = 0;
    // SAFETY: all calls are bounded queries; the property buffer is NUL terminated.
    let token = unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return Err(Error::new(ErrorCode::StaleBinding, "Window destroyed", "binding"));
        }
        GetWindowThreadProcessId(hwnd, Some(&raw mut pid));
        GetPropW(hwnd, PCWSTR(property.as_ptr()))
    };
    if pid != binding.process
        || token.0 as usize as u64 != binding.generation
        || process_identity(pid)? != (binding.process_started, binding.session)
    {
        return Err(Error::new(
            ErrorCode::StaleBinding,
            "Window lifetime or process/session no longer matches",
            "binding",
        ));
    }
    Ok(())
}

fn raw_monitor_info(monitor: HMONITOR) -> Result<crate::MonitorIdentity> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: correctly sized MONITORINFOEXW begins with MONITORINFO.
    if !unsafe { GetMonitorInfoW(monitor, &raw mut info.monitorInfo) }.as_bool() {
        return Err(native_error(
            ErrorCode::TargetMissing,
            "display",
            windows::core::Error::from_win32(),
        ));
    }
    let name = string(&info.szDevice);
    let name_wide = wide(&name);
    let mut device =
        DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
    // SAFETY: writable sized display structure and NUL-terminated adapter name.
    let enumerated =
        unsafe { EnumDisplayDevicesW(PCWSTR(name_wide.as_ptr()), 0, &raw mut device, 0) }.as_bool();
    let hardware =
        (enumerated && !string(&device.DeviceID).is_empty()).then(|| string(&device.DeviceID));
    let mut dpi_x = 96;
    let mut dpi_y = 96;
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &raw mut dpi_x, &raw mut dpi_y) };
    Ok(crate::MonitorIdentity {
        device: name.clone(),
        hardware,
        name: if enumerated { format!("{} ({name})", string(&device.DeviceString)) } else { name },
        work_area: rect(info.monitorInfo.rcWork),
        dpi: dpi_x,
    })
}

fn monitor_info(monitor: HMONITOR) -> Result<(Display, String)> {
    let monitor = raw_monitor_info(monitor)?;
    let id = monitor.hardware.ok_or_else(|| {
        Error::new(
            ErrorCode::AmbiguousBinding,
            "Monitor identity unavailable; explicit mapping required",
            &monitor.device,
        )
    })?;
    Ok((
        Display { id, name: monitor.name, work_area: monitor.work_area, dpi: monitor.dpi },
        monitor.device,
    ))
}

pub fn monitor_inventory() -> Result<Vec<crate::MonitorIdentity>> {
    unsafe extern "system" fn callback(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        parameter: LPARAM,
    ) -> BOOL {
        // SAFETY: synchronous enumeration owns the writable Vec for the complete callback lifetime.
        let output = unsafe { &mut *(parameter.0 as *mut Vec<Result<crate::MonitorIdentity>>) };
        output.push(raw_monitor_info(monitor));
        BOOL(1)
    }
    let _dpi = DpiGuard::new();
    let mut output: Vec<Result<crate::MonitorIdentity>> = Vec::new();
    unsafe { EnumDisplayMonitors(None, None, Some(callback), LPARAM((&raw mut output) as isize)) }
        .ok()
        .map_err(|error| native_error(ErrorCode::TargetMissing, "displays", error))?;
    output.into_iter().collect()
}

pub fn displays() -> Result<BTreeMap<Id, Display>> {
    unsafe extern "system" fn callback(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        parameter: LPARAM,
    ) -> BOOL {
        // SAFETY: EnumDisplayMonitors calls synchronously with our live Vec pointer.
        let output = unsafe { &mut *(parameter.0 as *mut Vec<Result<Display>>) };
        output.push(monitor_info(monitor).map(|(display, _)| display));
        BOOL(1)
    }
    let _dpi = DpiGuard::new();
    let mut output: Vec<Result<Display>> = Vec::new();
    unsafe { EnumDisplayMonitors(None, None, Some(callback), LPARAM((&raw mut output) as isize)) }
        .ok()
        .map_err(|error| native_error(ErrorCode::TargetMissing, "displays", error))?;
    let mut result = BTreeMap::new();
    for display in output {
        let display = display?;
        if result.insert(display.id.clone(), display).is_some() {
            return Err(Error::new(
                ErrorCode::AmbiguousBinding,
                "Duplicate monitor identity; automatic assignment blocked",
                "displays",
            ));
        }
    }
    Ok(result)
}

pub fn inventory() -> Result<Vec<Candidate>> {
    unsafe extern "system" fn callback(hwnd: HWND, parameter: LPARAM) -> BOOL {
        // SAFETY: synchronous EnumWindows callback with a live Vec pointer; query-only calls.
        let output = unsafe { &mut *(parameter.0 as *mut Vec<Candidate>) };
        let mut pid = 0;
        let mut frame = RECT::default();
        let mut title = [0; 1024];
        let mut class = [0; 256];
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&raw mut pid));
            if pid == std::process::id()
                || !IsWindowVisible(hwnd).as_bool()
                || GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.0.is_null())
                || GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0
            {
                return BOOL(1);
            }
            if GetWindowTextW(hwnd, &mut title) == 0 || GetWindowRect(hwnd, &raw mut frame).is_err()
            {
                return BOOL(1);
            }
            GetClassNameW(hwnd, &mut class);
        }
        let class = string(&class);
        if ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"]
            .contains(&class.as_str())
        {
            return BOOL(1);
        }
        output.push(Candidate {
            handle: hwnd.0 as usize as u64,
            title: string(&title),
            class,
            process: pid,
            frame: rect(frame),
        });
        BOOL(1)
    }
    let _dpi = DpiGuard::new();
    let mut output = Vec::new();
    // SAFETY: the callback uses output only during this synchronous enumeration.
    unsafe { EnumWindows(Some(callback), LPARAM((&raw mut output) as isize)) }
        .map_err(|error| native_error(ErrorCode::UnsupportedOperation, "inventory", error))?;
    output.sort_by(|a: &Candidate, b| a.title.cmp(&b.title));
    Ok(output)
}

fn has_owned_dialog(hwnd: HWND) -> bool {
    struct State {
        owner: HWND,
        found: bool,
    }
    unsafe extern "system" fn callback(candidate: HWND, parameter: LPARAM) -> BOOL {
        // SAFETY: State lives for the synchronous EnumWindows invocation.
        let state = unsafe { &mut *(parameter.0 as *mut State) };
        if unsafe { IsWindowVisible(candidate) }.as_bool()
            && unsafe { GetWindow(candidate, GW_OWNER) }.is_ok_and(|owner| owner == state.owner)
        {
            state.found = true;
        }
        BOOL(1)
    }
    let mut state = State { owner: hwnd, found: false };
    let _ = unsafe { EnumWindows(Some(callback), LPARAM((&raw mut state) as isize)) };
    state.found
}

pub fn observe(binding: &Binding, allow_hide: bool) -> Result<ObservedWindow> {
    observe_internal(binding, allow_hide, None)
}

pub fn observe_mapped(
    binding: &Binding,
    allow_hide: bool,
    mappings: &crate::MonitorMappings,
) -> Result<ObservedWindow> {
    observe_internal(binding, allow_hide, Some(mappings))
}

fn observe_internal(
    binding: &Binding,
    allow_hide: bool,
    mappings: Option<&crate::MonitorMappings>,
) -> Result<ObservedWindow> {
    validate_binding(binding)?;
    let _dpi = DpiGuard::new();
    let hwnd = handle(binding.handle);
    let mut frame = RECT::default();
    let mut client = RECT::default();
    // SAFETY: validated HWND with correctly sized writable rectangles; queries don't send app messages.
    unsafe { GetWindowRect(hwnd, &raw mut frame) }
        .map_err(|error| native_error(ErrorCode::StaleBinding, "window", error))?;
    unsafe { GetClientRect(hwnd, &raw mut client) }
        .map_err(|error| native_error(ErrorCode::StaleBinding, "window", error))?;
    let show_state = unsafe {
        if IsIconic(hwnd).as_bool() {
            ShowState::Minimized
        } else if IsZoomed(hwnd).as_bool() {
            ShowState::Maximized
        } else {
            ShowState::Normal
        }
    };
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let raw = raw_monitor_info(monitor)?;
    let display = if let Some(mappings) = mappings {
        if !mappings.inventory().contains(&raw) {
            return Err(Error::new(
                ErrorCode::StaleRevision,
                "Monitor topology changed during observation",
                "displays",
            ));
        }
        mappings.resolve(&raw.device)?
    } else {
        // Recovery needs visibility/show state even when durable display identity is unavailable.
        raw.hardware.unwrap_or_else(|| format!("unresolved:{}", raw.device))
    };
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    let observed = ObservedWindow {
        binding: binding.clone(),
        frame: rect(frame),
        client: [client.right - client.left, client.bottom - client.top],
        dpi: unsafe { GetDpiForWindow(hwnd) },
        display,
        visible: unsafe { IsWindowVisible(hwnd) }.as_bool(),
        show_state,
        can_move: show_state == ShowState::Normal,
        can_resize: style & WS_THICKFRAME.0 != 0 && show_state == ShowState::Normal,
        normal_resize_supported: style & WS_THICKFRAME.0 != 0,
        can_hide: allow_hide,
        has_owned_dialog: has_owned_dialog(hwnd),
    };
    validate_binding(binding)?;
    Ok(observed)
}

/// One final positioning request. It never requests Z-order/topmost or synthetic input.
pub fn submit(mutation: &Mutation) -> Result<()> {
    validate_binding(&mutation.binding)?;
    let _dpi = DpiGuard::new();
    let hwnd = handle(mutation.binding.handle);
    if let Some(state) = mutation.show_state {
        use windows::Win32::UI::WindowsAndMessaging::{
            SW_SHOWMAXIMIZED, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, ShowWindowAsync,
        };
        let command = match state {
            ShowState::Normal => SW_SHOWNOACTIVATE,
            ShowState::Minimized => SW_SHOWMINNOACTIVE,
            ShowState::Maximized => SW_SHOWMAXIMIZED,
        };
        // SAFETY: separately opted-in, explicit show-state action on the lifetime-validated HWND.
        if !unsafe { ShowWindowAsync(hwnd, command) }.as_bool() {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "Show-state submission was rejected",
                &mutation.window,
            ));
        }
    }
    let mut flags = SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS;
    if mutation.move_only {
        flags |= SWP_NOSIZE;
    }
    let frame = mutation.geometry.unwrap_or_default();
    if mutation.geometry.is_none() {
        flags |= SWP_NOMOVE | SWP_NOSIZE;
    }
    if let Some(visible) = mutation.visible {
        flags |= if visible { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
    }
    if mutation.geometry.is_some() || mutation.visible.is_some() {
        // SAFETY: revalidated top-level HWND; asynchronous request preserves activation and Z order.
        unsafe { SetWindowPos(hwnd, None, frame.x, frame.y, frame.width, frame.height, flags) }
            .map_err(|error| {
                native_error(ErrorCode::UnsupportedOperation, &mutation.window, error)
            })?;
    }
    Ok(())
}

pub fn focus(binding: &Binding) -> Result<()> {
    validate_binding(binding)?;
    // SAFETY: explicit user-requested focus on a validated window; no injection workaround.
    if unsafe { SetForegroundWindow(handle(binding.handle)) }.as_bool() {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::PermissionDenied,
            "Windows denied foreground activation",
            "focus",
        ))
    }
}

/// Batch only same-thread/same-parent windows, where `EndDeferWindowPos` cannot wait on a hung foreign thread.
///
/// Foreign app threads use one asynchronous final request each. There is no replay after a batch failure.
#[must_use]
pub fn submit_many(mutations: &[Mutation]) -> crate::SubmissionReport {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{GA_PARENT, GetAncestor};
    let mut report = crate::SubmissionReport {
        results: BTreeMap::new(),
        batched_windows: 0,
        individual_windows: 0,
    };
    let mut groups: BTreeMap<u64, Vec<&Mutation>> = BTreeMap::new();
    for mutation in mutations {
        let hwnd = handle(mutation.binding.handle);
        // SAFETY: query-only native ownership. Actual lifetime is revalidated before submission.
        let same_thread = unsafe { GetWindowThreadProcessId(hwnd, None) == GetCurrentThreadId() };
        if same_thread
            && mutation.show_state.is_none()
            && (mutation.geometry.is_some() || mutation.visible.is_some())
        {
            let parent = unsafe { GetAncestor(hwnd, GA_PARENT) }.0 as usize as u64;
            groups.entry(parent).or_default().push(mutation);
        } else {
            report.results.insert(mutation.window.clone(), submit(mutation));
            report.individual_windows += 1;
        }
    }
    for group in groups.values() {
        let result = submit_batch(group);
        report.batched_windows += group.len();
        for mutation in group {
            report.results.insert(mutation.window.clone(), result.clone());
        }
    }
    report
}

fn defer_mutation(
    batch: windows::Win32::UI::WindowsAndMessaging::HDWP,
    mutation: &Mutation,
) -> windows::core::Result<windows::Win32::UI::WindowsAndMessaging::HDWP> {
    use windows::Win32::UI::WindowsAndMessaging::DeferWindowPos;
    let mut flags = SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER;
    let frame = mutation.geometry.unwrap_or_default();
    if mutation.move_only {
        flags |= SWP_NOSIZE;
    }
    if mutation.geometry.is_none() {
        flags |= SWP_NOMOVE | SWP_NOSIZE;
    }
    if let Some(visible) = mutation.visible {
        flags |= if visible { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
    }
    // SAFETY: caller validates lifetime and supplies the most recently returned batch handle.
    unsafe {
        DeferWindowPos(
            batch,
            handle(mutation.binding.handle),
            None,
            frame.x,
            frame.y,
            frame.width,
            frame.height,
            flags,
        )
    }
}

fn submit_batch(mutations: &[&Mutation]) -> Result<()> {
    use windows::Win32::UI::WindowsAndMessaging::{BeginDeferWindowPos, EndDeferWindowPos};
    submit_batch_using(
        mutations,
        |count| unsafe { BeginDeferWindowPos(count) },
        defer_mutation,
        |batch| unsafe { EndDeferWindowPos(batch) },
    )
}

fn submit_batch_using(
    mutations: &[&Mutation],
    begin: impl FnOnce(i32) -> windows::core::Result<windows::Win32::UI::WindowsAndMessaging::HDWP>,
    mut defer: impl FnMut(
        windows::Win32::UI::WindowsAndMessaging::HDWP,
        &Mutation,
    ) -> windows::core::Result<windows::Win32::UI::WindowsAndMessaging::HDWP>,
    end: impl FnOnce(windows::Win32::UI::WindowsAndMessaging::HDWP) -> windows::core::Result<()>,
) -> Result<()> {
    if mutations.len() > 256 {
        return Err(Error::new(
            ErrorCode::UnsupportedOperation,
            "Native batch budget exceeded",
            "batch",
        ));
    }
    for mutation in mutations {
        validate_binding(&mutation.binding)?;
    }
    let _dpi = DpiGuard::new();
    let mut batch = begin(mutations.len() as i32)
        .map_err(|error| native_error(ErrorCode::UnsupportedOperation, "batch", error))?;
    for mutation in mutations {
        // Retain each returned handle. A failure abandons the batch without End or individual replay.
        batch = defer(batch, mutation).map_err(|error| {
            native_error(ErrorCode::UnsupportedOperation, &mutation.window, error)
        })?;
    }
    end(batch).map_err(|error| native_error(ErrorCode::UnsupportedOperation, "batch", error))
}

/// Independent recovery restores visibility only, never historical geometry or user-minimized windows.
pub fn recover(path: &Path) -> Result<Vec<String>> {
    recover_selected(path, None)
}

/// Failed components reveal only their own windows; other hidden tabs stay hidden.
pub fn recover_selected(
    path: &Path,
    selected: Option<&std::collections::BTreeSet<Id>>,
) -> Result<Vec<String>> {
    let mut journal = Journal::load(path)?;
    let mut diagnostics = Vec::new();
    let mut keep = Vec::new();
    for entry in journal.entries {
        if selected.is_some_and(|windows| !windows.contains(&entry.window)) {
            keep.push(entry);
            continue;
        }
        match observe(&entry.prior.binding, true) {
            Ok(current)
                if !current.visible
                    && current.show_state != ShowState::Minimized
                    && entry.prior.visible
                    && entry.prior.show_state != ShowState::Minimized =>
            {
                let mutation = Mutation {
                    window: entry.window.clone(),
                    binding: current.binding,
                    geometry: None,
                    move_only: true,
                    visible: Some(true),
                    focus: false,
                    show_state: None,
                };
                if let Err(error) = submit(&mutation) {
                    diagnostics.push(error.to_string());
                    keep.push(entry);
                } else {
                    let deadline =
                        std::time::Instant::now() + std::time::Duration::from_millis(900);
                    let settled = loop {
                        if observe(&entry.prior.binding, true).is_ok_and(|current| current.visible)
                        {
                            break true;
                        }
                        if std::time::Instant::now() >= deadline {
                            break false;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    };
                    if settled {
                        diagnostics.push(format!("{}: revealed; geometry unchanged", entry.window));
                    } else {
                        diagnostics.push(format!(
                            "{}: reveal did not settle; journal retained",
                            entry.window
                        ));
                        keep.push(entry);
                    }
                }
            }
            Ok(_) => {
                diagnostics.push(format!("{}: skipped (visible or user-minimized)", entry.window));
            }
            Err(error) => diagnostics.push(format!("{}: skipped ({error})", entry.window)),
        }
    }
    journal.entries = keep;
    journal.save(path)?;
    Ok(diagnostics)
}

/// Recovery helper checks the parent's creation time as well as PID to avoid PID reuse.
#[must_use]
pub fn parent_alive(pid: u32, started: u64) -> bool {
    use windows::Win32::Foundation::WAIT_TIMEOUT;
    use windows::Win32::System::Threading::{PROCESS_SYNCHRONIZE, WaitForSingleObject};
    if !process_identity(pid).is_ok_and(|(actual, _)| actual == started) {
        return false;
    }
    // SAFETY: query-only synchronization handle; zero timeout never blocks.
    let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else {
        return false;
    };
    let waiting = unsafe { WaitForSingleObject(process, 0) } == WAIT_TIMEOUT;
    let _ = unsafe { CloseHandle(process) };
    waiting
}
pub fn current_process_started() -> Result<u64> {
    process_identity(std::process::id()).map(|(time, _)| time)
}
pub fn process_started(pid: u32) -> Result<u64> {
    process_identity(pid).map(|(time, _)| time)
}

#[derive(Clone, Debug)]
pub enum NativeEvent {
    Changed(u64),
    GestureStarted(u64),
    GestureEnded(u64),
    Shortcut(u32),
    RegistrationError(String),
}

/// Out-of-context callbacks enqueue compact events; all observation/planning stays off the callback.
#[must_use]
pub fn event_stream(shortcuts: &[u32]) -> EventStream {
    event_stream_with_key(shortcuts, |number| 0x30 + number)
}

#[allow(clippy::too_many_lines)] // Hook callback and registration cleanup share one thread lifetime.
fn event_stream_with_key(shortcuts: &[u32], virtual_key: fn(u32) -> u32) -> EventStream {
    use windows::Win32::UI::WindowsAndMessaging::{
        PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_QUIT,
    };
    let (sender, receiver) = mpsc::sync_channel(1024);
    let numbers: std::collections::BTreeSet<_> = shortcuts.iter().copied().collect();
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        use std::cell::RefCell;
        use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            MOD_CONTROL, MOD_NOREPEAT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
        };
        thread_local! { static SENDER: RefCell<Option<mpsc::SyncSender<NativeEvent>>> = const { RefCell::new(None) }; }
        unsafe extern "system" fn callback(
            _: HWINEVENTHOOK,
            event: u32,
            hwnd: HWND,
            object: i32,
            child: i32,
            _: u32,
            _: u32,
        ) {
            if object == 0 && child == 0 {
                SENDER.with(|sender| {
                    if let Some(sender) = sender.borrow().as_ref() {
                        let handle = hwnd.0 as usize as u64;
                        let event = match event {
                            windows::Win32::UI::WindowsAndMessaging::EVENT_SYSTEM_MOVESIZESTART => {
                                NativeEvent::GestureStarted(handle)
                            }
                            windows::Win32::UI::WindowsAndMessaging::EVENT_SYSTEM_MOVESIZEEND => {
                                NativeEvent::GestureEnded(handle)
                            }
                            _ => NativeEvent::Changed(handle),
                        };
                        let _ = sender.try_send(event);
                    }
                });
            }
        }
        // Create the thread message queue before exposing its ID to shutdown.
        let mut queued = MSG::default();
        let _ = unsafe { PeekMessageW(&raw mut queued, None, 0, 0, PM_NOREMOVE) };
        let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
        SENDER.with(|slot| *slot.borrow_mut() = Some(sender.clone()));
        // SAFETY: out-of-context callback is valid for this thread's message-loop lifetime.
        let hook = unsafe {
            SetWinEventHook(
                EVENT_OBJECT_DESTROY,
                EVENT_OBJECT_LOCATIONCHANGE,
                None,
                Some(callback),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        // SAFETY: same bounded out-of-context callback; distinguishes user gestures from our native submission.
        let gesture_hook = unsafe {
            SetWinEventHook(
                windows::Win32::UI::WindowsAndMessaging::EVENT_SYSTEM_MOVESIZESTART,
                windows::Win32::UI::WindowsAndMessaging::EVENT_SYSTEM_MOVESIZEEND,
                None,
                Some(callback),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if hook.0.is_null() || gesture_hook.0.is_null() {
            let _ = sender.try_send(NativeEvent::RegistrationError(
                "Window observation hook unavailable; manual autosave disabled".into(),
            ));
        }
        let mut registered = Vec::new();
        for number in &numbers {
            if !(1..=9).contains(number) {
                let _ = sender.try_send(NativeEvent::RegistrationError(format!(
                    "Shortcut number {number} is outside 1..=9"
                )));
                continue;
            }
            // SAFETY: thread-scoped hotkey; stable configured number, no HWND or input injection.
            if let Err(error) = unsafe {
                RegisterHotKey(
                    None,
                    *number as i32,
                    MOD_CONTROL | MOD_WIN | MOD_NOREPEAT,
                    virtual_key(*number),
                )
            } {
                let _ = sender.try_send(NativeEvent::RegistrationError(format!(
                    "Win+Ctrl+{number}: {error}"
                )));
            } else {
                registered.push(*number);
            }
        }
        let _ = ready_sender.send(thread_id);
        let mut message = MSG::default();
        // SAFETY: writable MSG and a normal message loop for hooks/hotkeys on this worker.
        while unsafe { GetMessageW(&raw mut message, None, 0, 0) }.0 > 0 {
            if message.message == WM_HOTKEY
                && matches!(
                    sender.try_send(NativeEvent::Shortcut(message.wParam.0 as u32)),
                    Err(mpsc::TrySendError::Disconnected(_))
                )
            {
                break;
            }
        }
        for number in registered {
            let _ = unsafe { UnregisterHotKey(None, number as i32) };
        }
        if !hook.0.is_null() {
            let _ = unsafe { UnhookWinEvent(hook) };
        }
        if !gesture_hook.0.is_null() {
            let _ = unsafe { UnhookWinEvent(gesture_hook) };
        }
    });
    let thread_id = ready_receiver.recv().ok();
    EventStream {
        receiver,
        stop: Some(Box::new(move || {
            if let Some(thread_id) = thread_id {
                // SAFETY: this ID belongs to our live worker, whose queue already exists.
                let _ = unsafe {
                    PostThreadMessageW(
                        thread_id,
                        WM_QUIT,
                        windows::Win32::Foundation::WPARAM(0),
                        LPARAM(0),
                    )
                };
            }
            let _ = worker.join();
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SW_SHOWNOACTIVATE, ShowWindow, WINDOW_EX_STYLE,
        WS_OVERLAPPEDWINDOW,
    };
    use windows::core::w;
    #[test]
    fn shortcut_collision_preserves_owner_and_stream_drop_releases_registrations()
    -> windows::core::Result<()> {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            MOD_CONTROL, MOD_NOREPEAT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
        };
        struct Hotkey(i32);
        impl Drop for Hotkey {
            fn drop(&mut self) {
                let _ = unsafe { UnregisterHotKey(None, self.0) };
            }
        }
        let register = |number: u32| unsafe {
            RegisterHotKey(None, 100, MOD_CONTROL | MOD_WIN | MOD_NOREPEAT, 0x7C + number)
        };
        let Some(number) = (1..=9).find(|number| register(*number).is_ok()) else {
            eprintln!("No unclaimed Win+Ctrl+F14..F22 available; collision fixture skipped");
            return Ok(());
        };
        let owner = Hotkey(100);
        let stream = event_stream_with_key(&[number, number, 0, 10], |number| 0x7C + number);
        let errors = stream
            .try_iter()
            .filter_map(|event| match event {
                NativeEvent::RegistrationError(message) => Some(message),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            errors.iter().filter(|error| error.starts_with(&format!("Win+Ctrl+{number}:"))).count(),
            1
        );
        assert_eq!(errors.iter().filter(|error| error.contains("outside 1..=9")).count(), 2);
        drop(stream);
        // Our original registration remains owned by this thread after the failed worker attempt.
        let other = event_stream_with_key(&[number], |number| 0x7C + number);
        assert!(other.try_iter().any(|event| matches!(event, NativeEvent::RegistrationError(message) if message.starts_with(&format!("Win+Ctrl+{number}:")))));
        drop(other);
        drop(owner);
        for _ in 0..3 {
            let stream = event_stream_with_key(&[number], |number| 0x7C + number);
            assert!(
                !stream.try_iter().any(|event| matches!(event, NativeEvent::RegistrationError(_)))
            );
            assert!(register(number).is_err());
            drop(stream);
            register(number)?;
            drop(Hotkey(100));
        }
        Ok(())
    }

    struct Owned(HWND);
    impl Owned {
        fn new() -> std::result::Result<Self, windows::core::Error> {
            // SAFETY: built-in STATIC class; window owned and destroyed on this test thread.
            Ok(Self(unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    w!("Window manager integration fixture"),
                    WS_OVERLAPPEDWINDOW,
                    100,
                    120,
                    400,
                    300,
                    None,
                    None,
                    None,
                    None,
                )
            }?))
        }
        fn candidate(&self) -> Candidate {
            Candidate {
                handle: self.0.0 as usize as u64,
                title: "fixture".into(),
                class: "STATIC".into(),
                process: std::process::id(),
                frame: Rect::default(),
            }
        }
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = unsafe { DestroyWindow(self.0) };
        }
    }

    #[test]
    fn explicit_monitor_alias_observation_preserves_geometry_and_rejects_changed_inventory()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let window = Owned::new()?;
        let mut mappings = crate::MonitorMappings::default();
        mappings.update(monitor_inventory()?)?;
        let monitor =
            raw_monitor_info(unsafe { MonitorFromWindow(window.0, MONITOR_DEFAULTTONEAREST) })?;
        let epoch = mappings.epoch().to_owned();
        mappings.confirm(&epoch, &monitor.device, "manual-fixture".into())?;
        let before = bind_mapped(&window.candidate(), false, &mappings)?;
        assert_eq!(before.display, "manual-fixture");
        assert_eq!(observe(&before.binding, false)?.frame, before.frame);
        let mut changed = mappings.inventory().to_vec();
        for item in &mut changed {
            if item.device == monitor.device {
                item.dpi += 1;
            }
        }
        mappings.update(changed)?;
        assert!(observe_mapped(&before.binding, false, &mappings).is_err());
        assert!(observe(&before.binding, false).is_ok());
        Ok(())
    }

    #[test]
    fn move_only_keeps_client_size_and_destroyed_lifetime_is_rejected()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let window = Owned::new()?;
        let before = bind(&window.candidate(), false)?;
        let mutation = Mutation {
            window: "fixture".into(),
            binding: before.binding.clone(),
            geometry: Some(Rect { x: before.frame.x + 30, y: before.frame.y + 20, ..before.frame }),
            move_only: true,
            visible: None,
            focus: false,
            show_state: None,
        };
        submit(&mutation)?;
        let after = observe(&before.binding, false)?;
        assert_eq!(after.client, before.client);
        assert_eq!(after.frame.x, before.frame.x + 30);
        drop(window);
        assert!(validate_binding(&before.binding).is_err());
        Ok(())
    }

    #[test]
    fn journal_recovery_reveals_only_manager_hidden_and_preserves_geometry()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let window = Owned::new()?;
        let _ = unsafe { ShowWindow(window.0, SW_SHOWNOACTIVATE) };
        let before = bind(&window.candidate(), true)?;
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("journal.json");
        Journal {
            version: 1,
            entries: vec![crate::RecoveryEntry {
                window: "fixture".into(),
                prior: before.clone(),
                request: "hide".into(),
            }],
        }
        .save(&path)?;
        submit(&Mutation {
            window: "fixture".into(),
            binding: before.binding.clone(),
            geometry: None,
            move_only: true,
            visible: Some(false),
            focus: false,
            show_state: None,
        })?;
        assert!(!observe(&before.binding, true)?.visible);
        recover(&path)?;
        let after = observe(&before.binding, true)?;
        assert!(after.visible);
        assert_eq!(after.frame, before.frame);
        Ok(())
    }

    #[test]
    fn same_thread_batch_moves_once_and_preserves_each_client()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let first = Owned::new()?;
        let second = Owned::new()?;
        let observations = [bind(&first.candidate(), false)?, bind(&second.candidate(), false)?];
        let mutations: Vec<_> = observations
            .iter()
            .enumerate()
            .map(|(index, observed)| Mutation {
                window: format!("fixture-{index}"),
                binding: observed.binding.clone(),
                geometry: Some(Rect {
                    x: observed.frame.x + 30,
                    y: observed.frame.y + 20,
                    ..observed.frame
                }),
                move_only: true,
                visible: None,
                focus: false,
                show_state: None,
            })
            .collect();
        let report = submit_many(&mutations);
        assert_eq!(report.batched_windows, 2);
        assert_eq!(report.individual_windows, 0);
        for (index, before) in observations.iter().enumerate() {
            assert!(report.results[&format!("fixture-{index}")].is_ok());
            let after = observe(&before.binding, false)?;
            assert_eq!(after.frame, mutations[index].geometry.unwrap_or_default());
            assert_eq!(after.client, before.client);
        }
        Ok(())
    }

    #[test]
    fn batch_failure_paths_do_not_end_abandoned_batches_or_replay()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        use windows::Win32::UI::WindowsAndMessaging::HDWP;
        let window = Owned::new()?;
        let before = bind(&window.candidate(), false)?;
        let mutation = Mutation {
            window: "fixture".into(),
            binding: before.binding.clone(),
            geometry: Some(Rect { x: before.frame.x + 20, ..before.frame }),
            move_only: true,
            visible: None,
            focus: false,
            show_state: None,
        };
        let failure =
            || windows::core::Error::from_hresult(windows::core::HRESULT(0x8000_4005_u32 as i32));
        let tokens = [0_u8; 4];
        let token = |index: usize| HDWP((&raw const tokens[index]).cast_mut().cast());
        for phase in 0..3 {
            let calls = std::cell::RefCell::new(Vec::new());
            let result = submit_batch_using(
                &[&mutation, &mutation],
                |_| {
                    calls.borrow_mut().push("begin");
                    if phase == 0 { Err(failure()) } else { Ok(token(1)) }
                },
                |batch, _| {
                    let mut calls = calls.borrow_mut();
                    calls.push("defer");
                    if phase == 1 {
                        Err(failure())
                    } else {
                        assert_eq!(batch, token(calls.len() - 1));
                        Ok(token(calls.len()))
                    }
                },
                |batch| {
                    assert_eq!(batch, token(3));
                    calls.borrow_mut().push("end");
                    Err(failure())
                },
            );
            assert!(result.is_err());
            let expected: &[&str] = match phase {
                0 => &["begin"],
                1 => &["begin", "defer"],
                _ => &["begin", "defer", "defer", "end"],
            };
            assert_eq!(&*calls.borrow(), expected);
            assert_eq!(observe(&before.binding, false)?.frame, before.frame);
        }
        Ok(())
    }

    #[test]
    fn native_defer_failure_leaves_prior_window_unchanged_without_end()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        use windows::Win32::UI::WindowsAndMessaging::{BeginDeferWindowPos, EndDeferWindowPos};
        let first = Owned::new()?;
        let second = Owned::new()?;
        let before = [bind(&first.candidate(), false)?, bind(&second.candidate(), false)?];
        let mutations = before
            .iter()
            .enumerate()
            .map(|(index, prior)| Mutation {
                window: format!("fixture-{index}"),
                binding: prior.binding.clone(),
                geometry: Some(Rect { x: prior.frame.x + 20, ..prior.frame }),
                move_only: true,
                visible: None,
                focus: false,
                show_state: None,
            })
            .collect::<Vec<_>>();
        let ended = std::cell::Cell::new(false);
        let mut second = Some(second);
        let result = submit_batch_using(
            &[&mutations[0], &mutations[1]],
            |count| unsafe { BeginDeferWindowPos(count) },
            |batch, mutation| {
                if mutation.window == "fixture-1" {
                    drop(second.take());
                }
                defer_mutation(batch, mutation)
            },
            |batch| {
                ended.set(true);
                unsafe { EndDeferWindowPos(batch) }
            },
        );
        assert!(result.is_err());
        assert!(!ended.get());
        assert_eq!(observe(&before[0].binding, false)?.frame, before[0].frame);
        assert!(observe(&before[1].binding, false).is_err());
        Ok(())
    }

    #[test]
    fn explicit_minimize_and_normal_restore_do_not_need_input_injection()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let window = Owned::new()?;
        let _ = unsafe { ShowWindow(window.0, SW_SHOWNOACTIVATE) };
        let before = bind(&window.candidate(), false)?;
        let mut mutation = Mutation {
            window: "state-fixture".into(),
            binding: before.binding.clone(),
            geometry: None,
            move_only: true,
            visible: None,
            focus: false,
            show_state: Some(ShowState::Minimized),
        };
        submit(&mutation)?;
        let _ = wait_state(&before.binding, ShowState::Minimized)?;
        mutation.show_state = Some(ShowState::Normal);
        submit(&mutation)?;
        let after = wait_state(&before.binding, ShowState::Normal)?;
        assert_eq!(after.show_state, ShowState::Normal);
        assert_eq!(after.frame, before.frame);
        assert_eq!(after.client, before.client);
        Ok(())
    }

    fn wait_state(binding: &Binding, state: ShowState) -> Result<ObservedWindow> {
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, PM_REMOVE, PeekMessageW, TranslateMessage,
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(900);
        loop {
            let observed = observe(binding, false)?;
            if observed.show_state == state {
                return Ok(observed);
            }
            let mut message = MSG::default();
            // SAFETY: pump only this fixture thread's messages so asynchronous show events can settle.
            while unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                let _ = unsafe { TranslateMessage(&raw const message) };
                unsafe { DispatchMessageW(&raw const message) };
            }
            if std::time::Instant::now() >= deadline {
                return Err(Error::new(
                    ErrorCode::ApplicationTimeout,
                    "Fixture show state did not settle",
                    "fixture",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
