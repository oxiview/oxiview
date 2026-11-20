//! Native two-pane Markdown editor for the M6 editing milestone.

mod app;
mod commands;
mod document;
pub mod preview;
mod preview_pane;
pub mod replay;
#[cfg(test)]
mod snapshots;
mod view;

/// Launches the macOS editor or runs the same application on the headless backend.
pub fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::path::{Path, PathBuf};
    let mut args = std::env::args_os().skip(1);
    let mut path = None;
    let mut headless = false;
    let mut json = false;
    let mut output = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--headless") => headless = true,
            Some("--json") => json = true,
            Some("--output") => {
                output = Some(PathBuf::from(
                    args.next().ok_or("--output needs a directory")?,
                ))
            }
            Some("--help" | "-h") => {
                println!(
                    "markdown-editor-oxi [FILE.md]\nmarkdown-editor-oxi --headless [--output DIRECTORY] [--json]"
                );
                return Ok(());
            }
            Some(option) if option.starts_with('-') => {
                return Err(format!("Unknown option {option}").into());
            }
            _ if path.is_none() => path = Some(PathBuf::from(arg)),
            _ => return Err("Open one document at a time".into()),
        }
    }
    if headless {
        if path.is_some() {
            return Err("The replay uses its own document fixture".into());
        }
        let directory = output.unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/xtask/markdown-editor-oxi")
        });
        let report = replay::run(&directory)?;
        if json {
            println!("{}", serde_json::to_string_pretty(&report)?)
        } else {
            println!(
                "Passed {} checks. {}",
                report.checks.len(),
                report.output.display()
            );
        }
        return Ok(());
    }
    if output.is_some() || json {
        return Err("--output and --json require --headless".into());
    }
    #[cfg(target_os = "macos")]
    {
        oxiview_app::run_macos(
            oxiview_platform_macos::MacOsPlatform::default(),
            oxiview_platform::WindowAttributes {
                title: "Oxiview Markdown".into(),
                logical_size: oxiview_base::Size::new(1120., 720.),
                resize_timeout: std::time::Duration::from_millis(250),
            },
            move || app::EditorApp::new(path),
        )?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err("Use --headless on this platform. The native editor currently runs on macOS.".into())
    }
}
