//! Continuous native animation with live submission and on-screen frame rates.
#[cfg(any(target_os = "macos", test))]
mod cadence;
#[cfg(target_os = "macos")]
mod native;
mod scene;

use clap::Parser;
use std::{error::Error, process::ExitCode};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
const WIDTH: u32 = 960;
const HEIGHT: u32 = 600;
const MAX_OBJECTS: u16 = 128;

#[derive(Parser)]
#[command(
    about = "Measure continuous native animation. Space pauses or resumes; Up/Down changes the moving-object count."
)]
struct Options {
    /// Number of moving objects (1 to 128).
    #[arg(long, default_value_t = 32, value_parser = clap::value_parser!(u16).range(1..=i64::from(MAX_OBJECTS)))]
    objects: u16,
    /// Measure this many seconds after one warm-up second, print results and exit.
    #[arg(long, value_name = "SECONDS", value_parser = clap::value_parser!(u16).range(1..=60))]
    measure: Option<u16>,
    /// Print the native measurement as JSON.
    #[arg(long, requires = "measure")]
    json: bool,
    /// Start the native animation in fullscreen.
    #[arg(long)]
    fullscreen: bool,
    /// Export CPU previews of two animation phases.
    #[arg(long, conflicts_with_all = ["measure", "fullscreen"])]
    headless: bool,
}

fn run(options: Options) -> Result<()> {
    if options.headless {
        return scene::export(options.objects);
    }
    #[cfg(target_os = "macos")]
    return native::run(options);
    #[cfg(not(target_os = "macos"))]
    Err("On-screen FPS requires the native macOS backend. Use --headless to render two animation phases.".into())
}

fn main() -> ExitCode {
    match run(Options::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("animation-fps: {error}");
            ExitCode::from(2)
        }
    }
}
