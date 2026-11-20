//! Runs the native window and deterministic replay through the same game controller.

#[cfg(target_os = "macos")]
use crate::app::Demo;
use crate::{replay, settings::storage};
use snake::application::Config;
use std::{error::Error, path::PathBuf};

/// Starts the native game or its isolated command-line acceptance replay.
pub fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut config = Config::default();
    let mut headless = false;
    let mut json = false;
    let mut output = None;
    let mut prefs = None;
    let mut theme = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--headless" => headless = true,
            "--json" => json = true,
            "--seed" => config.seed = args.next().ok_or("--seed requires an integer")?.parse()?,
            "--settings" => {
                prefs = Some(PathBuf::from(
                    args.next().ok_or("--settings requires a path")?,
                ))
            }
            "--theme" => theme = Some(PathBuf::from(args.next().ok_or("--theme requires a path")?)),
            "--output" => {
                output = Some(PathBuf::from(
                    args.next().ok_or("--output requires a directory")?,
                ))
            }
            "--debug-overlay" => config.debug_overlay = true,
            "--help" | "-h" => {
                println!(
                    "snake-m5-oxi [--seed N] [--theme PATH] [--settings PATH] [--debug-overlay]\n             [--headless [--output DIR] [--json]]\nTab navigates controls. Open Settings to choose an appearance or record a shortcut."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    if headless {
        if prefs.is_some() {
            return Err(
                "the headless replay creates isolated player settings under --output".into(),
            );
        }
        let output = output.unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/xtask/snake-m5-oxi")
        });
        let report = replay::run(config, &output, theme.as_deref())?;
        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!(
                "Snake M5 passed {} checks. Artifacts: {}",
                report.checks.len(),
                output.display()
            );
        }
        return Ok(());
    }
    if json || output.is_some() {
        return Err("--json and --output require --headless".into());
    }
    let prefs = prefs
        .or_else(storage::user_path)
        .ok_or("cannot locate player settings; supply --settings PATH")?;
    native(config, prefs, theme)
}

#[cfg(target_os = "macos")]
fn native(config: Config, prefs: PathBuf, theme: Option<PathBuf>) -> Result<(), Box<dyn Error>> {
    use oxiview_platform::WindowAttributes;
    oxiview_app::run_macos(
        oxiview_platform_macos::MacOsPlatform::default(),
        WindowAttributes {
            title: "Snake · Keyboard & appearance".into(),
            logical_size: oxiview_base::Size::new(
                f64::from(config.width),
                f64::from(config.height),
            ),
            resize_timeout: std::time::Duration::from_millis(250),
        },
        move || Demo::new(config, prefs, theme, false),
    )?;
    Ok(())
}
#[cfg(not(target_os = "macos"))]
fn native(_: Config, _: PathBuf, _: Option<PathBuf>) -> Result<(), Box<dyn Error>> {
    Err("the native demo requires macOS; use --headless on this platform".into())
}
