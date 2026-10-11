#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#![allow(clippy::multiple_crate_versions)]
mod panel;
use anyhow::{Context, Result, bail};
use eframe::egui;
use std::path::PathBuf;
#[cfg(windows)]
use wgpu as _;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = match args.next() {
        None => default_path()?,
        Some(arg) if arg == "--config" => {
            PathBuf::from(args.next().context("--config requires a path")?)
        }
        Some(arg) if arg == "--help" || arg == "-h" => {
            println!("Audio Cable\nUsage: audio-cable [--config <settings.json>]");
            return Ok(());
        }
        Some(arg) => bail!("Unknown argument: {}", arg.to_string_lossy()),
    };
    if args.next().is_some() {
        bail!("Unexpected extra arguments");
    }
    let options = eframe::NativeOptions {
        #[cfg(windows)]
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([590.0, 520.0])
            .with_min_inner_size([500.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Audio Cable",
        options,
        Box::new(move |cc| {
            configure_style(&cc.egui_ctx);
            Ok(Box::new(panel::CablePanel::new(path, native_backend())))
        }),
    )
    .map_err(|error| anyhow::anyhow!("설정 창을 열 수 없습니다: {error}"))
}

fn default_path() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(root).join("DesktopEnvironment/audio-cable/settings.json"));
    }
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .context("Cannot locate settings directory")?;
    Ok(root.join("desktop-environment/audio-cable/settings.json"))
}

// Non-Windows builds legitimately return None until their native backend exists.
#[allow(clippy::unnecessary_wraps)]
fn native_backend() -> Option<std::sync::Arc<dyn audio_cable_core::AudioBackend>> {
    #[cfg(windows)]
    {
        Some(std::sync::Arc::new(windows_audio_router::WindowsAudioBackend))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn configure_style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Some(root) = std::env::var_os("WINDIR")
        && let Ok(bytes) = std::fs::read(PathBuf::from(root).join("Fonts/malgun.ttf"))
    {
        fonts.font_data.insert("korean".into(), egui::FontData::from_owned(bytes).into());
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "korean".into());
    }
    ctx.set_fonts(fonts);
    ctx.set_visuals(egui::Visuals::light());
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
    style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
    ctx.set_style(style);
}
