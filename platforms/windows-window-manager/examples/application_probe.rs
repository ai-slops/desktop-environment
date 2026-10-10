//! Measures an isolated browser process. Never binds an existing user window.
use serde as _;
#[cfg(windows)]
mod fixture {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::time::{Duration, Instant};
    use window_manager_core::{Mutation, ObservedWindow, Rect, ShowState};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_TIMEOUT};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
        QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
    };
    use windows::Win32::System::Threading::{
        CREATE_SUSPENDED, CreateProcessW, PROCESS_INFORMATION, ResumeThread, STARTUPINFOW,
        TerminateProcess, WaitForSingleObject,
    };
    use windows::core::{PCWSTR, PWSTR};
    use windows_window_manager::{
        Journal, RecoveryEntry, bind, displays, foreground_handle, inventory, observe,
        process_started, recover, submit_many,
    };

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
    struct Browser {
        job: Option<Handle>,
        process: Handle,
        pid: u32,
        started: u64,
        profile: tempfile::TempDir,
    }
    impl Drop for Browser {
        fn drop(&mut self) {
            // The job owns only the process launched suspended by this fixture and its children.
            if let Some(job) = self.job.take() {
                let _ = unsafe { TerminateJobObject(job.0, 0) };
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
                    let size = u32::try_from(size_of_val(&accounting)).unwrap_or(u32::MAX);
                    let queried = unsafe {
                        QueryInformationJobObject(
                            Some(job.0),
                            JobObjectBasicAccountingInformation,
                            (&raw mut accounting).cast(),
                            size,
                            None,
                        )
                    };
                    if queried.is_err()
                        || accounting.ActiveProcesses == 0
                        || Instant::now() >= deadline
                    {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                drop(job);
            }
            let _ = unsafe { WaitForSingleObject(self.process.0, 5_000) };
            // TempDir removes only its uniquely created profile after the browser has exited.
        }
    }
    impl Browser {
        fn launch(executable: &Path) -> Result<Self, Box<dyn std::error::Error>> {
            let root = std::env::current_dir()?.join("target").join("application-profiles");
            std::fs::create_dir_all(&root)?;
            let root = root.canonicalize()?;
            let profile = tempfile::Builder::new().prefix("owned-browser-").tempdir_in(&root)?;
            if !profile.path().canonicalize()?.starts_with(&root) {
                return Err("Profile is outside fixture root".into());
            }
            let exe = executable.canonicalize()?;
            // Arguments are constructed from canonical filesystem paths and constant flags only.
            let command = format!(
                "\"{}\" --user-data-dir=\"{}\" --no-first-run --no-default-browser-check --disable-background-networking --disable-sync --disable-component-update --disable-extensions --disable-default-apps --disable-features=msEdgeSidebarV2 --window-position=100,100 --window-size=800,600 --app=\"data:text/html,<title>Window-manager-fixture</title><h1>Disposable-fixture</h1>\"",
                exe.display(),
                profile.path().display()
            );
            let exe_wide: Vec<_> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut command_wide: Vec<_> = command.encode_utf16().chain(Some(0)).collect();
            let job = Handle(unsafe { CreateJobObjectW(None, PCWSTR::null()) }?);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    u32::try_from(size_of_val(&limits))?,
                )?;
            }
            let startup = STARTUPINFOW {
                cb: u32::try_from(size_of::<STARTUPINFOW>())?,
                ..Default::default()
            };
            let mut information = PROCESS_INFORMATION::default();
            // Suspended creation prevents children from escaping assignment to our cleanup job.
            unsafe {
                CreateProcessW(
                    PCWSTR(exe_wide.as_ptr()),
                    Some(PWSTR(command_wide.as_mut_ptr())),
                    None,
                    None,
                    false,
                    CREATE_SUSPENDED,
                    None,
                    PCWSTR::null(),
                    &raw const startup,
                    &raw mut information,
                )?;
            }
            let process = Handle(information.hProcess);
            let thread = Handle(information.hThread);
            if let Err(error) = unsafe { AssignProcessToJobObject(job.0, process.0) } {
                let _ = unsafe { TerminateProcess(process.0, 1) };
                return Err(error.into());
            }
            let mut browser =
                Self { job: Some(job), process, pid: information.dwProcessId, started: 0, profile };
            browser.started = process_started(browser.pid)?;
            if unsafe { ResumeThread(thread.0) } == u32::MAX {
                return Err(windows::core::Error::from_win32().into());
            }
            Ok(browser)
        }
        fn alive(&self) -> bool {
            (unsafe { WaitForSingleObject(self.process.0, 0) }) == WAIT_TIMEOUT
                && process_started(self.pid).is_ok_and(|started| started == self.started)
        }
        fn window(&self) -> Result<ObservedWindow, Box<dyn std::error::Error>> {
            let deadline = Instant::now() + Duration::from_secs(20);
            while self.alive() && Instant::now() < deadline {
                let candidates = inventory()?
                    .into_iter()
                    .filter(|candidate| {
                        candidate.process == self.pid && candidate.class == "Chrome_WidgetWin_1"
                    })
                    .collect::<Vec<_>>();
                // Ambiguous/missing windows never widen ownership to an existing browser process.
                if candidates.len() == 1 {
                    let window = bind(&candidates[0], true)?;
                    if self.alive()
                        && window.binding.process_started == self.started
                        && window.visible
                        && window.show_state == ShowState::Normal
                        && !window.has_owned_dialog
                    {
                        return Ok(window);
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err("No single normal window owned by the launched browser process".into())
        }
    }
    fn distribution(values: &mut [u64]) -> serde_json::Value {
        values.sort_unstable();
        let at = |percent: usize| values[(values.len() * percent).div_ceil(100).saturating_sub(1)];
        serde_json::json!({"median":at(50),"p95":at(95),"p99":at(99),"worst":values.last()})
    }
    fn nanos(duration: Duration) -> u64 {
        u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
    }
    fn wait_for(
        baseline: &ObservedWindow,
        predicate: impl Fn(&ObservedWindow) -> bool,
    ) -> Result<ObservedWindow, Box<dyn std::error::Error>> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let current = observe(&baseline.binding, true)?;
            if predicate(&current) {
                return Ok(current);
            }
            if Instant::now() >= deadline {
                return Err("Lifecycle operation did not settle".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn lifecycle(
        browser: &Browser,
        baseline: &ObservedWindow,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let mut mutation = Mutation {
            window: "fixture".into(),
            binding: baseline.binding.clone(),
            geometry: None,
            move_only: true,
            visible: None,
            focus: false,
            show_state: Some(ShowState::Minimized),
        };
        let report = submit_many(std::slice::from_ref(&mutation));
        if !report.results.values().all(Result::is_ok) {
            return Err("Minimize submission failed".into());
        }
        wait_for(baseline, |current| current.show_state == ShowState::Minimized)?;
        let path = browser.profile.path().join("recovery-fixture.json");
        let journal = Journal {
            version: 1,
            entries: vec![RecoveryEntry {
                window: "fixture".into(),
                prior: baseline.clone(),
                request: "visibility-fixture".into(),
            }],
        };
        journal.save(&path)?;
        recover(&path)?;
        let minimized_preserved =
            observe(&baseline.binding, true)?.show_state == ShowState::Minimized;
        mutation.show_state = Some(ShowState::Normal);
        let report = submit_many(std::slice::from_ref(&mutation));
        if !report.results.values().all(Result::is_ok) {
            return Err("Normal restore submission failed".into());
        }
        wait_for(baseline, |current| current.show_state == ShowState::Normal)?;
        journal.save(&path)?;
        mutation.show_state = None;
        mutation.visible = Some(false);
        let report = submit_many(&[mutation]);
        if !report.results.values().all(Result::is_ok) {
            return Err("Hide submission failed".into());
        }
        wait_for(baseline, |current| !current.visible)?;
        recover(&path)?;
        let revealed = wait_for(baseline, |current| current.visible)?;
        Ok(serde_json::json!({"manual_minimized_state_preserved_by_recovery":minimized_preserved,
            "hidden_window_revealed":revealed.visible,"recovery_geometry_unchanged":revealed.frame==baseline.frame,
            "recovery_client_unchanged":revealed.client==baseline.client,
            "recovery_journal_empty":Journal::load(&path)?.entries.is_empty()}))
    }
    fn mixed_dpi(
        browser: &Browser,
        baseline: &ObservedWindow,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let scale = |value: i32, dpi: u32| {
            i32::try_from(i64::from(value) * i64::from(dpi) / i64::from(baseline.dpi))
        };
        let monitors = displays()?;
        let Some(target) = monitors.values().find(|display| {
            display.dpi != baseline.dpi
                && scale(baseline.frame.width, display.dpi)
                    .is_ok_and(|width| width + 200 <= display.work_area.width)
                && scale(baseline.frame.height, display.dpi)
                    .is_ok_and(|height| height + 200 <= display.work_area.height)
        }) else {
            return Ok(
                serde_json::json!({"status":"not_measured","reason":"no fitting monitor with a different DPI"}),
            );
        };
        let mut times = Vec::new();
        let mut failures = 0;
        let mut foreground_changes = 0;
        let mut mismatches = Vec::new();
        for iteration in 0..25 {
            if !browser.alive() {
                return Err("Owned process lifetime ended".into());
            }
            let transfer = iteration % 2 == 0;
            let dpi = if transfer { target.dpi } else { baseline.dpi };
            let frame = if transfer {
                Rect {
                    x: target.work_area.x + 100,
                    y: target.work_area.y + 100,
                    width: scale(baseline.frame.width, dpi)?,
                    height: scale(baseline.frame.height, dpi)?,
                }
            } else {
                baseline.frame
            };
            let client = [scale(baseline.client[0], dpi)?, scale(baseline.client[1], dpi)?];
            let mutation = Mutation {
                window: "fixture".into(),
                binding: baseline.binding.clone(),
                geometry: Some(frame),
                move_only: false,
                visible: None,
                focus: false,
                show_state: None,
            };
            let foreground = foreground_handle();
            let started = Instant::now();
            let report = submit_many(&[mutation]);
            let deadline = started + Duration::from_millis(900);
            let settled = loop {
                let current = observe(&baseline.binding, true)?;
                let settled = report.results.values().all(Result::is_ok)
                    && current.frame == frame
                    && current.client == client
                    && current.dpi == dpi;
                if settled || Instant::now() >= deadline {
                    if !settled {
                        let mismatch = serde_json::json!({"expected_dpi":dpi,"observed_dpi":current.dpi,
                            "expected_client":client,"observed_client":current.client,
                            "expected_frame_size":[frame.width,frame.height],
                            "observed_frame_size":[current.frame.width,current.frame.height],
                            "position_matches":current.frame.x==frame.x && current.frame.y==frame.y});
                        if !mismatches.contains(&mismatch) && mismatches.len() < 10 {
                            mismatches.push(mismatch);
                        }
                    }
                    break settled;
                }
                std::thread::sleep(Duration::from_millis(2));
            };
            if iteration >= 5 {
                times.push(nanos(started.elapsed()));
                failures += usize::from(!settled);
                foreground_changes += usize::from(foreground_handle() != foreground);
            }
        }
        let restore = Mutation {
            window: "fixture".into(),
            binding: baseline.binding.clone(),
            geometry: Some(baseline.frame),
            move_only: false,
            visible: None,
            focus: false,
            show_state: None,
        };
        let _ = submit_many(&[restore]);
        wait_for(baseline, |current| {
            current.frame == baseline.frame && current.dpi == baseline.dpi
        })?;
        Ok(
            serde_json::json!({"status":"measured","source_dpi":baseline.dpi,"destination_dpi":target.dpi,
            "mismatches":mismatches,"samples":20,"geometry_client_dpi_settlement_ns":distribution(&mut times),"failures":failures,
            "observed_foreground_changes":foreground_changes,"final_geometry_requests_per_transition":1,
            "rendering_readiness":"unknown"}),
        )
    }
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        if args.len() != 4 {
            return Err("Usage: application_probe <chrome-or-edge-exe> <application> <file-version> <output-json>".into());
        }
        let browser = Browser::launch(Path::new(&args[0]))?;
        let initial = browser.window()?;
        // Allow normal first-window startup to finish before obtaining the measurement baseline.
        std::thread::sleep(Duration::from_millis(750));
        let baseline = observe(&initial.binding, true)?;
        let mut cases = Vec::new();
        for mode in ["move_only", "resize", "hide_show"] {
            let mut submission_times = Vec::new();
            let mut settlement_times = Vec::new();
            let mut failures = 0;
            let mut foreground_changes = 0;
            let mut client_mismatches = 0;
            let mut operations = 0;
            for iteration in 0..35 {
                if !browser.alive() {
                    return Err("Owned process lifetime ended".into());
                }
                let offset = if iteration % 2 == 0 { 20 } else { 0 };
                let frame = Rect {
                    x: baseline.frame.x + offset,
                    y: baseline.frame.y,
                    width: baseline.frame.width + if mode == "resize" { offset } else { 0 },
                    height: baseline.frame.height + if mode == "resize" { offset } else { 0 },
                };
                let mutation = Mutation {
                    window: "fixture".into(),
                    binding: baseline.binding.clone(),
                    geometry: (mode != "hide_show").then_some(frame),
                    move_only: mode != "resize",
                    visible: (mode == "hide_show").then_some(offset == 0),
                    focus: false,
                    show_state: None,
                };
                let foreground = foreground_handle();
                let started = Instant::now();
                let report = submit_many(std::slice::from_ref(&mutation));
                let submitted = nanos(started.elapsed());
                let deadline = started + Duration::from_millis(900);
                let (settled, client_ok) = loop {
                    let current = observe(&baseline.binding, true)?;
                    let settled = report.results.values().all(Result::is_ok)
                        && mutation.geometry.is_none_or(|expected| current.frame == expected)
                        && mutation.visible.is_none_or(|expected| current.visible == expected);
                    let expected_client = [
                        baseline.client[0] + if mode == "resize" { offset } else { 0 },
                        baseline.client[1] + if mode == "resize" { offset } else { 0 },
                    ];
                    let client_ok =
                        current.client == expected_client && current.dpi == baseline.dpi;
                    if (settled && client_ok) || Instant::now() >= deadline {
                        break (settled, client_ok);
                    }
                    std::thread::sleep(Duration::from_millis(2));
                };
                if iteration >= 5 {
                    submission_times.push(submitted);
                    settlement_times.push(nanos(started.elapsed()));
                    failures += usize::from(!settled);
                    client_mismatches += usize::from(!client_ok);
                    foreground_changes += usize::from(foreground_handle() != foreground);
                    operations += report.individual_windows;
                }
            }
            cases.push(serde_json::json!({"mode":mode,"windows":1,"samples":30,
                "submission_ns":distribution(&mut submission_times),"geometry_settlement_ns":distribution(&mut settlement_times),
                "failures":failures,"client_or_dpi_mismatches":client_mismatches,
                "individual_operations":operations,"observed_foreground_changes":foreground_changes}));
            let restore = Mutation {
                window: "fixture".into(),
                binding: baseline.binding.clone(),
                geometry: Some(baseline.frame),
                move_only: false,
                visible: Some(true),
                focus: false,
                show_state: None,
            };
            let _ = submit_many(&[restore]);
            std::thread::sleep(Duration::from_millis(100));
        }
        let lifecycle = lifecycle(&browser, &baseline)?;
        let mixed_dpi = mixed_dpi(&browser, &baseline)?;
        let profile_path = browser.profile.path().to_path_buf();
        drop(browser);
        let terminated_binding_rejected = observe(&baseline.binding, true).is_err();
        let temporary_profile_removed = !profile_path.exists();
        let output = serde_json::json!({"lifecycle":lifecycle,"mixed_dpi":mixed_dpi,
            "terminated_binding_rejected":terminated_binding_rejected,"temporary_profile_removed":temporary_profile_removed,
            "application":args[1].to_string_lossy(),"file_version":args[2].to_string_lossy(),
            "fixture":"single ordinary-user browser app window with unique temporary profile and constant data page",
            "process_cleanup":"suspended creation; kill-on-close job assigned before resume",
            "dpi":baseline.dpi,"initial_client":baseline.client,"show_state":"normal","visibility_opt_in":true,
            "rendering_readiness":"unknown","cases":cases});
        std::fs::write(Path::new(&args[3]), serde_json::to_vec_pretty(&output)?)?;
        Ok(())
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    fixture::run()?;
    #[cfg(not(windows))]
    return Err("Application probe requires Windows".into());
    Ok(())
}
