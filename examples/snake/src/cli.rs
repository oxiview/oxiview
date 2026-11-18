//! Native and headless command line shared by both Snake examples.

use crate::{
    application::{Config, Screen, run_native_with_screen},
    replay,
};
use std::{error::Error, path::PathBuf};

fn run(name: &str, screen: Screen) -> Result<(), Box<dyn Error>> {
    let mut config = Config::default();
    let mut headless = false;
    let mut json = false;
    let mut output_selected = false;
    let mut output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/xtask")
        .join(name);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--headless" => headless = true,
            "--json" => json = true,
            "--debug-overlay" => config.debug_overlay = true,
            "--seed" => config.seed = args.next().ok_or("--seed requires an integer")?.parse()?,
            "--output" => {
                output = PathBuf::from(args.next().ok_or("--output requires a directory")?);
                output_selected = true;
            }
            "--help" | "-h" => {
                println!(
                    "{name} [--seed N] [--debug-overlay] [--headless [--output DIRECTORY] [--json]]\nNative: Enter starts a new game, Space pauses/resumes, arrows or WASD steer, Escape pauses.\n--debug-overlay shows app FPS, display cadence, CPU timing, scene/layout counts, and viewport scale."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument {arg}; use --help").into()),
        }
    }
    if (json || output_selected) && !headless {
        return Err("--json and --output require --headless".into());
    }
    if headless {
        let report = replay::run_with_screen(config, &output, screen)?;
        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!(
                "Snake acceptance passed: {} checks; {} rendered real-font images.\nArtifacts: {}",
                report.checks.len(),
                report.images.len(),
                output.display()
            );
        }
    } else {
        println!(
            "Snake: Enter starts a new game; Space pauses/resumes; arrows or WASD steer; Escape pauses."
        );
        run_native_with_screen(config, screen)?;
    }
    Ok(())
}
/// Runs the game CLI and reports errors with the selected executable name.
pub fn main(name: &str, screen: Screen) -> std::process::ExitCode {
    match run(name, screen) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{name}: {error}");
            std::process::ExitCode::from(2)
        }
    }
}
