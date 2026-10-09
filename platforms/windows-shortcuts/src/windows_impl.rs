use anyhow::{Context, Result, bail};
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize, IPersistFile,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, IShellLinkW, KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellLink,
};
use windows::core::{Interface, PCWSTR, w};

/// Create or refresh Desktop Control.lnk on the user's actual Windows desktop.
pub fn create_desktop_shortcut(executable: &Path, config: &Path) -> Result<PathBuf> {
    let executable = std::path::absolute(executable)?;
    let config = std::path::absolute(config)?;
    // A dedicated COM apartment also works when the GUI thread already uses another mode.
    std::thread::spawn(move || {
        let _com = ComGuard::new()?;
        let desktop = desktop_path()?;
        create_shortcut_in(&desktop, &executable, &config)
    })
    .join()
    .map_err(|_| anyhow::anyhow!("바로가기를 만드는 중 오류가 발생했습니다."))?
}

fn desktop_path() -> Result<PathBuf> {
    // SAFETY: the known-folder GUID is valid; Windows returns a NUL-terminated allocation.
    let allocated = unsafe { SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None) }
        .context("바탕화면 폴더를 찾을 수 없습니다.")?;
    // SAFETY: the pointer is owned by this call and valid until CoTaskMemFree below.
    let path = unsafe { OsString::from_wide(allocated.as_wide()) };
    // SAFETY: this allocation came from SHGetKnownFolderPath and is released exactly once.
    unsafe { CoTaskMemFree(Some(allocated.0.cast())) };
    Ok(PathBuf::from(path))
}

fn create_shortcut_in(directory: &Path, executable: &Path, config: &Path) -> Result<PathBuf> {
    if !executable.is_file() {
        bail!("GUI 실행 파일을 찾을 수 없습니다: {}", executable.display());
    }
    let working_dir = executable.parent().context("GUI 실행 파일 폴더가 없습니다.")?;
    let executable_wide = wide(executable.as_os_str())?;
    let working_dir_wide = wide(working_dir.as_os_str())?;
    let arguments = config_arguments(config)?;
    // SAFETY: the caller initialized COM; CLSID_ShellLink exposes IShellLinkW.
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }?;
    // SAFETY: all strings are NUL-terminated and stay alive for these synchronous calls.
    unsafe {
        link.SetPath(PCWSTR(executable_wide.as_ptr()))?;
        link.SetWorkingDirectory(PCWSTR(working_dir_wide.as_ptr()))?;
        link.SetArguments(PCWSTR(arguments.as_ptr()))?;
        link.SetDescription(w!("Desktop Control - saved audio and display settings"))?;
        link.SetIconLocation(PCWSTR(executable_wide.as_ptr()), 0)?;
    }
    let persist: IPersistFile = link.cast()?;
    let temporary = tempfile::Builder::new().suffix(".lnk").tempfile_in(directory)?;
    // Close the temporary file handle so the Shell can write to it.
    let temporary = temporary.into_temp_path();
    let temporary_wide = wide(temporary.as_os_str())?;
    // SAFETY: persist is a live COM object and the path buffer outlives Save.
    unsafe { persist.Save(PCWSTR(temporary_wide.as_ptr()), false) }
        .context("바로가기 파일을 저장할 수 없습니다.")?;
    let path = directory.join("Desktop Control.lnk");
    temporary.persist(&path).context("바탕화면 바로가기를 추가할 수 없습니다.")?;
    Ok(path)
}

fn wide(value: &OsStr) -> Result<Vec<u16>> {
    let mut units: Vec<u16> = value.encode_wide().collect();
    if units.contains(&0) {
        bail!("경로에 NUL 문자를 사용할 수 없습니다.");
    }
    units.push(0);
    Ok(units)
}

fn config_arguments(config: &Path) -> Result<Vec<u16>> {
    let mut arguments: Vec<u16> = "--config \"".encode_utf16().collect();
    let mut backslashes = 0;
    for unit in config.as_os_str().encode_wide() {
        if unit == 0 {
            bail!("설정 파일 경로에 NUL 문자를 사용할 수 없습니다.");
        }
        if unit == u16::from(b'\\') {
            backslashes += 1;
            continue;
        }
        // Windows command-line parsing doubles backslashes before quotes.
        let count = if unit == u16::from(b'"') { backslashes * 2 + 1 } else { backslashes };
        arguments.extend(std::iter::repeat_n(u16::from(b'\\'), count));
        arguments.push(unit);
        backslashes = 0;
    }
    arguments.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2));
    arguments.extend([u16::from(b'"'), 0]);
    Ok(arguments)
}

struct ComGuard;

impl ComGuard {
    fn new() -> Result<Self> {
        // SAFETY: this guard balances each successful initialization on the same thread.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() }?;
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY: created only after a successful CoInitializeEx, on this same thread.
        unsafe { CoUninitialize() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::STGM_READ;
    use windows::Win32::UI::Shell::SLGP_RAWPATH;

    fn decoded(units: &[u16]) -> String {
        let end = units.iter().position(|unit| *unit == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end])
    }

    #[test]
    fn shortcut_round_trip_and_refresh_keep_config_and_target() -> Result<()> {
        let _com = ComGuard::new()?;
        let directory = tempfile::tempdir()?;
        let executable = std::env::current_exe()?;
        let first_config = directory.path().join("한글 설정 & spaces.json");
        let second_config = directory.path().join("another config.json");
        for config in [&first_config, &second_config] {
            let path = create_shortcut_in(directory.path(), &executable, config)?;
            assert_eq!(path.file_name(), Some(OsStr::new("Desktop Control.lnk")));
            let path_wide = wide(path.as_os_str())?;
            // SAFETY: this test initialized COM; all output buffers are writable and bounded.
            unsafe {
                let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
                let persist: IPersistFile = link.cast()?;
                persist.Load(PCWSTR(path_wide.as_ptr()), STGM_READ)?;
                let mut target = vec![0; 32768];
                link.GetPath(&mut target, std::ptr::null_mut(), SLGP_RAWPATH.0.cast_unsigned())?;
                assert_eq!(PathBuf::from(decoded(&target)), executable);
                let mut arguments = vec![0; 32768];
                link.GetArguments(&mut arguments)?;
                assert_eq!(decoded(&arguments), decoded(&config_arguments(config)?));
                let mut working_dir = vec![0; 32768];
                link.GetWorkingDirectory(&mut working_dir)?;
                assert_eq!(
                    PathBuf::from(decoded(&working_dir)),
                    executable.parent().context("no parent")?
                );
            }
        }
        assert_eq!(std::fs::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn missing_executable_leaves_existing_shortcut_intact() -> Result<()> {
        let _com = ComGuard::new()?;
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("Desktop Control.lnk");
        std::fs::write(&path, b"existing")?;
        assert!(
            create_shortcut_in(
                directory.path(),
                &directory.path().join("missing.exe"),
                &directory.path().join("settings.json")
            )
            .is_err()
        );
        assert_eq!(std::fs::read(path)?, b"existing");
        Ok(())
    }

    #[test]
    fn config_argument_quotes_spaces_quotes_and_trailing_slashes() -> Result<()> {
        let config = Path::new("C:\\some space\\quoted\"name\\");
        assert_eq!(
            decoded(&config_arguments(config)?),
            "--config \"C:\\some space\\quoted\\\"name\\\\\""
        );
        assert!(config_arguments(Path::new("bad\0path")).is_err());
        Ok(())
    }

    #[test]
    fn desktop_is_resolved_through_windows_known_folders() -> Result<()> {
        let _com = ComGuard::new()?;
        let desktop = desktop_path()?;
        assert!(desktop.is_absolute());
        assert!(desktop.is_dir());
        Ok(())
    }
}
