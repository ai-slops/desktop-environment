//! Windows-first multi-view window manager with a separate recovery invocation.
#![allow(clippy::multiple_crate_versions)]
#![cfg_attr(test, allow(unused_crate_dependencies))] // Integration tests share this binary's dev dependencies.

mod cli;
mod reflow;
mod replay;
mod service;
mod session;
mod ui;

use anyhow::{Context, Result};
use std::path::PathBuf;

fn default_path() -> Result<PathBuf> {
    let root =
        std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable; use --config")?;
    Ok(PathBuf::from(root).join("DesktopEnvironment").join("window-manager.json"))
}

fn main() -> Result<()> {
    let mut arguments = cli::parse(std::env::args().skip(1))?;
    let path = std::path::absolute(arguments.path.take().map_or_else(default_path, Ok)?)?;
    cli::execute(arguments, path)
}
