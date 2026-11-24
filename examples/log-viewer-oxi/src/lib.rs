//! A self-drawn log viewer for the M7 widget and scrolling milestone.

mod app;
#[cfg(any(test, target_os = "macos"))]
mod cadence;
#[cfg(target_os = "macos")]
mod measurement;
pub mod model;
pub mod replay;
#[cfg(test)]
mod snapshots;
mod source;
mod view;

#[cfg(target_os = "macos")]
fn run_native_viewer<A: oxiview_core::Application>(
    platform: oxiview_platform_macos::MacOsPlatform,
    application: impl FnOnce() -> Result<A, oxiview_core::Error> + Send + 'static,
) -> Result<(), oxiview_app::Error> {
    oxiview_app::run_macos_with_display_sync(
        platform.with_metal_drawables(),
        oxiview_platform::WindowAttributes {
            title: "Oxiview Logs".into(),
            logical_size: oxiview_base::Size::new(cadence::WINDOW_WIDTH, cadence::WINDOW_HEIGHT),
            resize_timeout: std::time::Duration::from_millis(250),
        },
        application,
    )
}
/// Opens the viewer, optionally following a local text file, or measures scrolling.
pub fn run() -> Result<std::process::ExitCode, Box<dyn std::error::Error + Send + Sync>> {
    use std::{path::PathBuf, process::ExitCode};
    let mut args = std::env::args_os().skip(1);
    let mut file = None;
    let mut measure = false;
    let mut headless = false;
    let mut output = None;
    let mut json = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--measure-scroll") => measure = true,
            Some("--headless") => headless = true,
            Some("--output") => {
                output = Some(PathBuf::from(
                    args.next().ok_or("--output needs a directory")?,
                ))
            }
            Some("--json") => json = true,
            Some("-h" | "--help") => {
                println!(
                    "log-viewer-oxi [FILE]\nlog-viewer-oxi --headless [--output DIRECTORY] [--json]\nlog-viewer-oxi --measure-scroll [--json]"
                );
                return Ok(ExitCode::SUCCESS);
            }
            Some(s) if s.starts_with('-') => return Err(format!("Unknown argument {s}").into()),
            _ if file.is_none() => file = Some(PathBuf::from(arg)),
            _ => return Err("Open one log file at a time".into()),
        }
    }
    if headless {
        if measure || file.is_some() {
            return Err("The headless replay uses its own log fixtures".into());
        }
        let directory = output.unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/xtask/log-viewer-oxi")
        });
        let report = replay::run(&directory)?;
        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!(
                "Passed {} checks. {}",
                report.checks.len(),
                report.output.display()
            );
        }
        return Ok(ExitCode::SUCCESS);
    }
    if output.is_some() {
        return Err("--output requires --headless".into());
    }
    if measure && file.is_some() {
        return Err("Scroll measurement requires the built-in sample".into());
    }
    if json && !measure {
        return Err("--json requires --headless or --measure-scroll".into());
    }
    #[cfg(target_os = "macos")]
    {
        if measure {
            let report = measurement::run()?;
            if json {
                println!("{}", serde_json::to_string(&report)?);
            } else {
                println!(
                    "{} of {} refresh slots displayed; {} dropped, {} missing callbacks.",
                    report.occupied_slots,
                    report.expected_slots,
                    report.dropped_frames,
                    report.missing_feedback
                );
            }
            return Ok(ExitCode::from(if !report.valid {
                2
            } else if report.pass {
                0
            } else {
                1
            }));
        }
        run_native_viewer(
            oxiview_platform_macos::MacOsPlatform::default(),
            move || app::ViewerApp::new(file),
        )?;
        Ok(ExitCode::SUCCESS)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Native log viewing and display measurement currently require macOS.".into())
    }
}
