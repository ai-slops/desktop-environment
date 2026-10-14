use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

// Only processes started by this controller enter the job. A crashed host must
// not leave audio playing or mirrors running. The window-manager session stays
// outside it so its existing independent recovery watchdog can complete.
#[cfg(windows)]
struct ProcessJob(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl ProcessJob {
    fn attach(child: &Child) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        // SAFETY: unnamed, non-inherited job handle is owned by this guard.
        let job = Self(unsafe { CreateJobObjectW(None, windows::core::PCWSTR::null()) }?);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: structure and child process handle remain valid through these calls.
        unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                u32::try_from(std::mem::size_of_val(&limits))?,
            )?;
            AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle()))?;
        }
        Ok(job)
    }
}
#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        // SAFETY: this guard exclusively owns the job handle.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

pub struct OwnedProcess {
    child: Option<Child>,
    input: Option<ChildStdin>,
    lines: Receiver<String>,
    pub logs: VecDeque<String>,
    pub phase: String,
    pub error: Option<String>,
    #[cfg(windows)]
    _job: Option<ProcessJob>,
}

pub fn hidden_command(binary: &Path) -> Command {
    let mut command = Command::new(binary);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

fn reader(stream: impl Read + Send + 'static, sender: mpsc::SyncSender<String>, limit: usize) {
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            if sender.send(line.chars().take(limit).collect()).is_err() {
                break;
            }
        }
    });
}

impl OwnedProcess {
    pub fn spawn(binary: &Path, args: &[String], phase: &str) -> Result<Self> {
        let mut child = hidden_command(binary)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("NO_COLOR", "1")
            .spawn()
            .with_context(|| format!("실행 파일을 시작할 수 없습니다: {}", binary.display()))?;
        #[cfg(windows)]
        let job = match ProcessJob::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.context("Cannot contain the owned child process"));
            }
        };
        let (send, receive) = mpsc::sync_channel(256);
        if let Some(output) = child.stdout.take() {
            reader(output, send.clone(), 3000);
        }
        if let Some(output) = child.stderr.take() {
            reader(output, send, 3000);
        }
        Ok(Self {
            input: child.stdin.take(),
            child: Some(child),
            lines: receive,
            logs: VecDeque::new(),
            phase: phase.into(),
            error: None,
            #[cfg(windows)]
            _job: Some(job),
        })
    }
    pub fn write(&mut self, value: &Value) -> Result<()> {
        let input = self.input.as_mut().context("Worker has exited")?;
        serde_json::to_writer(&mut *input, value)?;
        input.write_all(b"\n")?;
        input.flush()?;
        Ok(())
    }
    fn drain(&mut self) {
        for line in self.lines.try_iter().take(256) {
            if let Ok(event) = serde_json::from_str::<Value>(&line)
                && event["kind"] == "worker_state"
            {
                if let Some(phase) = event["phase"].as_str() {
                    self.phase = phase.into();
                }
                continue;
            }
            self.logs.push_back(line);
            if self.logs.len() > 120 {
                self.logs.pop_front();
            }
        }
    }
    pub fn poll(&mut self) -> Result<()> {
        self.drain();
        if let Some(child) = self.child.as_mut()
            && let Some(exit) = child.try_wait()?
        {
            self.child = None;
            self.input = None;
            // Readers can trail the process exit. Bound collection of its final error.
            let deadline = Instant::now() + Duration::from_millis(100);
            while Instant::now() < deadline {
                match self.lines.recv_timeout(Duration::from_millis(5)) {
                    Ok(line) => {
                        self.logs.push_back(line);
                        if self.logs.len() > 120 {
                            self.logs.pop_front();
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            self.phase = if exit.success() { "stopped" } else { "failed" }.into();
            if !exit.success() {
                self.error = Some(format!(
                    "{exit}\n{}",
                    self.logs.iter().cloned().collect::<Vec<_>>().join("\n")
                ));
            }
        }
        Ok(())
    }
    pub const fn alive(&self) -> bool {
        self.child.is_some()
    }
    pub fn stop(&mut self) -> Result<()> {
        self.input.take();
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + Duration::from_millis(250);
            while child.try_wait()?.is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if child.try_wait()?.is_none() {
                child.kill()?;
            }
            child.wait()?;
        }
        self.drain();
        self.phase = "stopped".into();
        self.error = None;
        Ok(())
    }
    pub fn snapshot(&self) -> Value {
        json!({"phase":self.phase,"error":self.error,"logs":self.logs,"pid":self.child.as_ref().map(Child::id)})
    }
}
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub struct WindowSession {
    child: Child,
    input: Option<ChildStdin>,
    responses: Receiver<String>,
    errors: Receiver<String>,
}
impl WindowSession {
    pub fn spawn(binary: &Path, config: &Path) -> Result<Self> {
        let mut child = hidden_command(binary)
            .args(["--session", "--allow-control", "--config"])
            .arg(config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let (send, receive) = mpsc::sync_channel(64);
        let (err_send, err_receive) = mpsc::sync_channel(64);
        if let Some(output) = child.stdout.take() {
            reader(output, send, 1_048_576);
        }
        if let Some(output) = child.stderr.take() {
            reader(output, err_send, 6000);
        }
        Ok(Self { input: child.stdin.take(), child, responses: receive, errors: err_receive })
    }
    pub fn request(&mut self, id: &str, command: &Value) -> Result<Value> {
        let envelope = json!({"id":id,"command":command});
        if serde_json::to_vec(&envelope)?.len() >= 65_536 {
            bail!("창 명령이 64 KiB 제한을 초과했습니다. 변경 범위를 줄이세요.");
        }
        let input = self.input.as_mut().context("Window session closed")?;
        serde_json::to_writer(&mut *input, &envelope)?;
        input.write_all(b"\n")?;
        input.flush()?;
        let line = self.responses.recv_timeout(Duration::from_secs(6)).map_err(|error| {
            anyhow::anyhow!(
                "Window session unavailable: {error}. {}",
                self.errors.try_iter().collect::<Vec<_>>().join("\n")
            )
        })?;
        let response: Value = serde_json::from_str(&line)?;
        if response["id"] != id {
            bail!("Window session response ID mismatch");
        }
        if !response["error"].is_null() {
            bail!("{}: {}", response["error"]["code"], response["error"]["message"]);
        }
        Ok(response["result"].clone())
    }
}
impl Drop for WindowSession {
    fn drop(&mut self) {
        self.input.take();
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn powershell() -> Result<std::path::PathBuf> {
        Ok(std::path::PathBuf::from(
            std::env::var_os("SystemRoot").context("Windows system directory")?,
        )
        .join("System32/WindowsPowerShell/v1.0/powershell.exe"))
    }
    #[test]
    fn failed_child_preserves_final_cause_and_stop_is_bounded() -> Result<()> {
        let mut worker = OwnedProcess::spawn(
            &powershell()?,
            &[
                "-NoProfile".into(),
                "-Command".into(),
                "[Console]::Error.WriteLine('fixture final cause'); exit 1".into(),
            ],
            "starting",
        )?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while worker.alive() && Instant::now() < deadline {
            worker.poll()?;
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!worker.alive());
        assert_eq!(worker.phase, "failed");
        assert!(worker.error.as_deref().is_some_and(|error| error.contains("fixture final cause")));
        let mut waiting = OwnedProcess::spawn(
            &powershell()?,
            &["-NoProfile".into(), "-Command".into(), "Start-Sleep -Seconds 60".into()],
            "running",
        )?;
        let start = Instant::now();
        waiting.stop()?;
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(!waiting.alive());
        Ok(())
    }
    #[test]
    fn closing_owned_job_terminates_only_its_fixture_child() -> Result<()> {
        let mut child = hidden_command(&powershell()?)
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
            .spawn()?;
        let job = ProcessJob::attach(&child)?;
        drop(job);
        let deadline = Instant::now() + Duration::from_secs(3);
        while child.try_wait()?.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let exited = child.try_wait()?.is_some();
        if !exited {
            let _ = child.kill();
        }
        child.wait()?;
        assert!(exited, "Closing the host-owned job must terminate its child");
        Ok(())
    }
}
