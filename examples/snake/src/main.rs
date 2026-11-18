//! Standalone native Snake, with a deterministic headless acceptance replay.

use snake::{
    application::{Config, run_native},
    replay,
};
use std::{error::Error, path::PathBuf};

fn run() -> Result<(), Box<dyn Error>> {
    let mut config = Config::default();
    let mut headless = false;
    let mut json = false;
    let mut output_selected = false;
    let mut output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/xtask/snake");
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
                    "snake [--seed N] [--debug-overlay] [--headless [--output DIRECTORY] [--json]]\nNative: Enter starts a new game, Space pauses/resumes, arrows or WASD steer, Escape pauses.\n--debug-overlay shows app FPS, display cadence, CPU timing, scene/layout counts, and viewport scale."
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
        let report = replay::run(config, &output)?;
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
        run_native(config)?;
    }
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("snake: {error}");
            std::process::ExitCode::from(2)
        }
    }
}
