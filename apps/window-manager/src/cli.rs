use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use window_manager_core::{
    Configuration, LayoutPackage, MAX_CONFIGURATION_BYTES, atomic_write, backup_path,
};

#[derive(Default)]
pub enum Action {
    #[default]
    Gui,
    Check,
    Initialize,
    Recover,
    RestoreBackup,
    Watch(u32, u64),
    Export(String, PathBuf),
    Import(PathBuf),
    Session,
    Help,
}

pub struct Arguments {
    pub path: Option<PathBuf>,
    pub action: Action,
    pub workspace: Option<String>,
    pub mappings: BTreeMap<String, String>,
    pub allow_control: bool,
    pub allow_providers: bool,
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Arguments> {
    let mut args = args.into_iter();
    let mut result = Arguments {
        path: None,
        action: Action::Gui,
        workspace: None,
        mappings: BTreeMap::new(),
        allow_control: false,
        allow_providers: false,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                result.path = Some(PathBuf::from(args.next().context("--config requires a path")?));
            }
            "--allow-control" if !result.allow_control => result.allow_control = true,
            "--allow-providers" if !result.allow_providers => result.allow_providers = true,
            "--workspace" => {
                result.workspace = Some(args.next().context("--workspace requires a stable ID")?);
            }
            "--map" => {
                let mapping = args.next().context("--map requires role=window-id")?;
                let (role, window) =
                    mapping.split_once('=').context("--map requires role=window-id")?;
                if role.is_empty()
                    || window.is_empty()
                    || result.mappings.insert(role.into(), window.into()).is_some()
                {
                    bail!("Empty or repeated role mapping");
                }
            }
            flag => {
                if !matches!(result.action, Action::Gui) {
                    bail!("Choose one command action");
                }
                result.action = match flag {
                    "--check" => Action::Check,
                    "--session" => Action::Session,
                    "--init" => Action::Initialize,
                    "--recover" => Action::Recover,
                    "--restore-backup" => Action::RestoreBackup,
                    "--watch-parent" => Action::Watch(
                        args.next().context("missing PID")?.parse()?,
                        args.next().context("missing process creation time")?.parse()?,
                    ),
                    "--export" => Action::Export(
                        args.next().context("--export requires View ID")?,
                        PathBuf::from(args.next().context("--export requires output file")?),
                    ),
                    "--import" => Action::Import(PathBuf::from(
                        args.next().context("--import requires package file")?,
                    )),
                    "--help" | "-h" => Action::Help,
                    _ => bail!("Unknown argument: {flag}"),
                };
            }
        }
    }
    if !matches!(result.action, Action::Import(_))
        && (result.workspace.is_some() || !result.mappings.is_empty())
    {
        bail!("Role mappings and Workspace apply only to import");
    }
    if (result.allow_control || result.allow_providers) && !matches!(result.action, Action::Session)
    {
        bail!("Capability grants apply only to --session");
    }
    Ok(result)
}

pub fn lock_configuration(path: &Path) -> Result<std::fs::File> {
    std::fs::create_dir_all(path.parent().context("Configuration directory missing")?)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    options
        .open(path.with_extension("lock"))
        .context("Configuration is in use; close the GUI before editing it from the CLI")
}

pub fn execute(arguments: Arguments, path: PathBuf) -> Result<()> {
    match arguments.action {
        Action::Gui => crate::ui::run(path),
        Action::Session => {
            crate::session::run(&path, arguments.allow_control, arguments.allow_providers)
        }
        Action::Check => {
            let config = Configuration::load(&path)?;
            println!("{}", serde_json::to_string_pretty(&config)?);
            Ok(())
        }
        Action::Initialize => {
            let _lock = lock_configuration(&path)?;
            if path.exists() {
                bail!("Configuration already exists");
            }
            Configuration::default().save(&path)?;
            println!("Initialized {}", path.display());
            Ok(())
        }
        Action::Recover => {
            for diagnostic in windows_window_manager::recover(&crate::service::journal_path(&path))?
            {
                println!("{diagnostic}");
            }
            Ok(())
        }
        Action::Watch(pid, started) => {
            while windows_window_manager::parent_alive(pid, started) {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            windows_window_manager::recover(&crate::service::journal_path(&path))?;
            Ok(())
        }
        Action::RestoreBackup => {
            let _lock = lock_configuration(&path)?;
            let backup = backup_path(&path);
            if !backup.is_file() {
                bail!("Known-good backup does not exist");
            }
            let config = Configuration::load(&backup)?;
            atomic_write(&path, &serde_json::to_vec_pretty(&config)?)?;
            println!("Restored revision {}", config.revision);
            Ok(())
        }
        Action::Export(view, output) => {
            let config = Configuration::load(&path)?;
            let view = config.views.get(&view).context("View ID missing")?;
            let output = std::path::absolute(output)?;
            if output == path {
                bail!("Package output must be separate from configuration");
            }
            atomic_write(&output, &serde_json::to_vec_pretty(&LayoutPackage::from_view(view))?)?;
            println!("Exported privacy-safe package to {}", output.display());
            Ok(())
        }
        Action::Import(package_path) => {
            let _lock = lock_configuration(&path)?;
            if std::fs::metadata(&package_path)?.len() > MAX_CONFIGURATION_BYTES {
                bail!("Package exceeds 4 MiB budget");
            }
            let package: LayoutPackage = window_manager_core::read_json(&package_path)?;
            let workspace = arguments
                .workspace
                .context("--import requires --workspace and explicit --map for every role")?;
            let mut config = Configuration::load(&path)?;
            let id = package.install(&mut config, &workspace, &arguments.mappings)?;
            config.save(&path)?;
            println!("Imported independent View {id}; no application windows changed");
            Ok(())
        }
        Action::Help => {
            println!(
                "Window Manager\n  window-manager [--config <file>]\n  window-manager --init | --check | --recover | --restore-backup [--config <file>]\n  window-manager --export <view-id> <package.json> [--config <file>]\n  window-manager --import <package.json> --workspace <id> --map <role>=<window-id> [--config <file>]\n  window-manager --session [--allow-control] [--allow-providers] [--config <file>]\n\nSession uses JSON Lines over stdin/stdout and is read-only by default. GUI startup never applies layouts. Import never moves windows. Recovery only reveals manager-hidden windows."
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_and_widening_cli_arguments_are_rejected() {
        for input in [
            vec!["--check", "--recover"],
            vec!["--check", "--map", "role=window"],
            vec!["--import", "package.json", "--map", "role=a", "--map", "role=b"],
            vec!["--unknown"],
        ] {
            assert!(parse(input.into_iter().map(str::to_owned)).is_err());
        }
    }
}
