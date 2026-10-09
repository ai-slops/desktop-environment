#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
// eframe and winit pull in distinct versions of platform and image dependencies.
#![allow(clippy::multiple_crate_versions)]

mod panel;
mod process;

use anyhow::{Context, Result, bail};
use desktop_presets::default_store_path;
use eframe::egui;
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = match args.next() {
        None => default_store_path()?,
        Some(arg) if arg == "--config" => {
            let path = args.next().context("--config requires a file path")?;
            std::path::absolute(PathBuf::from(path))?
        }
        Some(arg) if arg == "--help" || arg == "-h" => {
            println!("Desktop Control\nUsage: desktop-control [--config <presets.json>]");
            return Ok(());
        }
        Some(arg) => bail!("Unknown argument: {}", arg.to_string_lossy()),
    };
    if args.next().is_some() {
        bail!("Unexpected extra arguments");
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 760.0])
            .with_min_inner_size([850.0, 650.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Desktop Control",
        options,
        Box::new(move |cc| {
            configure_style(&cc.egui_ctx);
            let mut panel = panel::ControlPanel::new(path);
            panel.restore_window(cc);
            Ok(Box::new(panel))
        }),
    )
    .map_err(|error| anyhow::anyhow!("설정 창을 열 수 없습니다: {error}"))
}

fn configure_style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    // egui's bundled Latin fonts do not include Korean device names or UI labels.
    if let Some(windows_dir) = std::env::var_os("WINDIR") {
        let path = PathBuf::from(windows_dir).join("Fonts").join("malgun.ttf");
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert("malgun".into(), egui::FontData::from_owned(bytes).into());
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().insert(0, "malgun".into());
            }
        }
    }
    ctx.set_fonts(fonts);
    ctx.set_visuals(egui::Visuals::light());
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.visuals.selection.bg_fill = egui::Color32::from_rgb(36, 96, 180);
    style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
    style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
    ctx.set_style(style);
}
