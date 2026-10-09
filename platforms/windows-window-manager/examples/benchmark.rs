//! Native adapter measurements use only disposable windows created by this process.
use serde as _;
use tempfile as _;
#[cfg(windows)]
mod fixture {
    use std::time::{Duration, Instant};
    use window_manager_core::{Mutation, Rect};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW,
        SW_SHOWNOACTIVATE, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
    };
    use windows::core::w;
    use windows_window_manager::{Candidate, bind, foreground_handle, observe, submit_many};

    struct Owned(HWND);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = unsafe { DestroyWindow(self.0) };
        }
    }
    impl Owned {
        fn new(index: i32) -> windows::core::Result<Self> {
            // SAFETY: built-in STATIC class and handles owned by this benchmark thread.
            let window = Self(unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    w!("Disposable window manager benchmark"),
                    WS_OVERLAPPEDWINDOW,
                    100 + (index % 4) * 280,
                    100 + (index / 4) * 210,
                    250,
                    180,
                    None,
                    None,
                    None,
                    None,
                )?
            });
            let _ = unsafe { ShowWindow(window.0, SW_SHOWNOACTIVATE) };
            Ok(window)
        }
        #[allow(clippy::cast_sign_loss)] // Opaque HWND identifier; never dereferenced or used as arithmetic.
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
    fn pump() {
        let mut message = MSG::default();
        // SAFETY: this benchmark thread owns only the fixture windows.
        while unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            let _ = unsafe { TranslateMessage(&raw const message) };
            unsafe { DispatchMessageW(&raw const message) };
        }
    }
    fn nanos(duration: Duration) -> u64 {
        u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
    }
    fn distribution(values: &mut [u64]) -> serde_json::Value {
        values.sort_unstable();
        let at = |percent: usize| values[(values.len() * percent).div_ceil(100).saturating_sub(1)];
        serde_json::json!({"median":at(50),"p95":at(95),"p99":at(99),"worst":values.last()})
    }
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let mut cases = Vec::new();
        for count in [4, 8, 16] {
            let owned = (0..count).map(Owned::new).collect::<windows::core::Result<Vec<_>>>()?;
            let bindings = owned
                .iter()
                .map(|window| bind(&window.candidate(), true).map(|window| window.binding))
                .collect::<window_manager_core::Result<Vec<_>>>()?;
            let baseline = bindings
                .iter()
                .map(|binding| observe(binding, true))
                .collect::<window_manager_core::Result<Vec<_>>>()?;
            for mode in ["move_only", "partial_resize", "widespread_resize", "hide_show"] {
                let mut submitted = Vec::new();
                let mut settled = Vec::new();
                let mut failures = 0;
                let mut focus_changes = 0;
                let mut batched = 0;
                let mut individual = 0;
                for iteration in 0..25 {
                    let positive = iteration % 2 == 0;
                    let mutations = bindings
                        .iter()
                        .zip(&baseline)
                        .enumerate()
                        .filter(|(index, _)| mode != "partial_resize" || index % 2 == 0)
                        .map(|(index, (binding, prior))| {
                            let resize = mode == "widespread_resize" || mode == "partial_resize";
                            let frame = Rect {
                                x: prior.frame.x + if positive { 10 } else { 0 },
                                y: prior.frame.y,
                                width: prior.frame.width + if resize && positive { 10 } else { 0 },
                                height: prior.frame.height
                                    + if resize && positive { 10 } else { 0 },
                            };
                            Mutation {
                                window: format!("fixture-{index}"),
                                binding: binding.clone(),
                                geometry: (mode != "hide_show").then_some(frame),
                                move_only: !resize,
                                visible: (mode == "hide_show").then_some(!positive),
                                focus: false,
                                show_state: None,
                            }
                        })
                        .collect::<Vec<_>>();
                    let foreground = foreground_handle();
                    let started = Instant::now();
                    let submission = submit_many(&mutations);
                    let submission_ns = nanos(started.elapsed());
                    let deadline = started + Duration::from_millis(900);
                    let mut success;
                    loop {
                        pump();
                        success = submission.results.values().all(Result::is_ok);
                        for mutation in &mutations {
                            let current = observe(&mutation.binding, true)?;
                            success &= mutation.geometry.is_none_or(|frame| current.frame == frame)
                                && mutation
                                    .visible
                                    .is_none_or(|visible| current.visible == visible);
                            if mode == "move_only" {
                                let index = mutation
                                    .window
                                    .strip_prefix("fixture-")
                                    .ok_or("fixture ID")?
                                    .parse::<usize>()?;
                                assert_eq!(current.client, baseline[index].client);
                                assert_eq!(current.dpi, baseline[index].dpi);
                            }
                        }
                        if success || Instant::now() >= deadline {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    if iteration >= 5 {
                        submitted.push(submission_ns);
                        settled.push(nanos(started.elapsed()));
                        failures += usize::from(!success);
                        focus_changes += usize::from(foreground_handle() != foreground);
                        batched += submission.batched_windows;
                        individual += submission.individual_windows;
                    }
                }
                cases.push(serde_json::json!({"application_class":"Win32 STATIC fixture","windows":count,"mode":mode,"samples":20,"submission_ns":distribution(&mut submitted),"geometry_settlement_ns":distribution(&mut settled),"failures":failures,"observed_foreground_changes":focus_changes,"batched_operations":batched,"individual_operations":individual,"rendering_readiness":"unknown"}));
                if failures != 0 {
                    return Err(format!("{mode}/{count} failed to settle").into());
                }
            }
            // RAII destroys only our own fixture handles, even if a measurement fails.
        }
        let output = serde_json::to_vec_pretty(
            &serde_json::json!({"fixture":"disposable same-thread native windows; no third-party compatibility claim","dpi":"observed same-monitor basis","cases":cases}),
        )?;
        if let Some(path) = std::env::args_os().nth(1) {
            std::fs::write(path, output)?;
        } else {
            println!("{}", String::from_utf8(output)?);
        }
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    fixture::run()?;
    #[cfg(not(windows))]
    return Err("Native benchmark requires a Windows desktop".into());
    Ok(())
}
