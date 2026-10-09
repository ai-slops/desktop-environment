use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender};

const LOG_LIMIT: usize = 120;

pub struct ManagedProcess {
    child: Option<Child>,
    lines: Option<Receiver<String>>,
    pub logs: VecDeque<String>,
    pub status: String,
}

impl Default for ManagedProcess {
    fn default() -> Self {
        Self { child: None, lines: None, logs: VecDeque::new(), status: "중지됨".into() }
    }
}

impl ManagedProcess {
    pub const fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub fn start(&mut self, binary: &Path, args: &[impl AsRef<OsStr>]) -> Result<()> {
        if self.is_running() {
            anyhow::bail!("이미 실행 중입니다.");
        }
        let mut command = Command::new(binary);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("RUST_LOG", "info")
            .env("NO_COLOR", "1");
        hide_console(&mut command);
        let mut child = command.spawn().with_context(|| {
            format!(
                "{} 실행 실패. GUI와 같은 폴더에 실행 파일이 있는지 확인하세요.",
                binary.display()
            )
        })?;
        let (sender, receiver) = mpsc::sync_channel(256);
        if let Some(stdout) = child.stdout.take() {
            read_lines(stdout, sender.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            read_lines(stderr, sender);
        }
        self.logs.clear();
        self.lines = Some(receiver);
        self.child = Some(child);
        self.status = "실행 중".into();
        Ok(())
    }

    pub fn poll(&mut self) -> Result<()> {
        self.drain_logs();
        if let Some(child) = self.child.as_mut()
            && let Some(exit) = child.try_wait().context("실행 상태 확인 실패")?
        {
            self.child = None;
            self.drain_logs();
            if exit.success() {
                self.status = "종료됨".into();
            } else {
                self.status = format!("오류로 종료 ({exit})");
                anyhow::bail!("{} 아래 실행 로그에서 원인을 확인하세요.", self.status);
            }
        }
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if let Some(child) = self.child.as_mut() {
            if child.try_wait()?.is_none() {
                child.kill().context("실행 중인 도구를 중지할 수 없습니다.")?;
            }
            child.wait().context("도구 종료를 기다릴 수 없습니다.")?;
            self.child = None;
            self.status = "중지됨".into();
        }
        self.drain_logs();
        Ok(())
    }

    fn drain_logs(&mut self) {
        if let Some(receiver) = self.lines.as_ref() {
            for line in receiver.try_iter().take(256) {
                self.logs.push_back(line);
                if self.logs.len() > LOG_LIMIT {
                    self.logs.pop_front();
                }
            }
        }
    }
}

impl Drop for ManagedProcess {
    fn drop(&mut self) {
        // Only this GUI's child handle is terminated; unrelated instances are untouched.
        let _ = self.stop();
    }
}

fn read_lines(stream: impl Read + Send + 'static, sender: SyncSender<String>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            if sender.send(line.chars().take(2000).collect()).is_err() {
                break;
            }
        }
    });
}

#[cfg(target_os = "windows")]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn hide_console(_: &mut Command) {}

pub fn sibling_binary(name: &str) -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    let directory = executable.parent().context("실행 파일 폴더를 찾을 수 없습니다.")?;
    let binary = directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    if !binary.is_file() {
        anyhow::bail!(
            "{} 파일이 없습니다. just desktop-control로 실행하거나 세 실행 파일을 같은 폴더에 두세요.",
            binary.display()
        );
    }
    Ok(binary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn fixture_args(test: &str) -> Vec<String> {
        vec!["--ignored".into(), "--exact".into(), test.into(), "--nocapture".into()]
    }

    #[test]
    fn child_start_log_stop_and_drop() -> Result<()> {
        let mut process = ManagedProcess::default();
        let binary = std::env::current_exe()?;
        let args = fixture_args("process::tests::long_running_fixture");
        process.start(&binary, &args)?;
        assert!(process.start(&binary, &args).is_err());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !process.logs.iter().any(|line| line.contains("fixture ready")) {
            process.poll()?;
            assert!(process.is_running());
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        process.stop()?;
        assert!(!process.is_running());
        process.stop()?;
        process.start(&binary, &args)?;
        drop(process);
        Ok(())
    }

    #[test]
    fn failed_child_reports_exit_and_keeps_logs() -> Result<()> {
        let mut process = ManagedProcess::default();
        process.start(&std::env::current_exe()?, &fixture_args("process::tests::exit_fixture"))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let error = loop {
            if let Err(error) = process.poll() {
                break error;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(error.to_string().contains("오류로 종료"));
        assert!(!process.is_running());
        while !process.logs.iter().any(|line| line.contains("fixture failure")) {
            process.poll()?;
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    #[test]
    #[ignore = "spawned as a child by process lifecycle tests"]
    fn long_running_fixture() {
        println!("fixture ready");
        std::thread::sleep(std::time::Duration::from_secs(30));
    }

    #[test]
    #[ignore = "spawned as a child by process lifecycle tests"]
    fn exit_fixture() {
        eprintln!("fixture failure");
        std::process::exit(7);
    }
}
