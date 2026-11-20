//! Opt-in native smoke check for M5 keyboard input and system preferences.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    match (arguments.next(), arguments.next()) {
        (Some(argument), None) if argument == "--run" => match run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("M5 input probe failed. {error}");
                ExitCode::FAILURE
            }
        },
        (None, None) => {
            print_help();
            ExitCode::SUCCESS
        }
        (Some(argument), None) if argument == "--help" || argument == "-h" => {
            print_help();
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("Expected --run or --help.");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "Run m5_input_probe --run on macOS 14 or later to open one window for up to 8 seconds."
    );
}

#[cfg(target_os = "macos")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        sync::mpsc::{self, RecvTimeoutError},
        thread,
        time::{Duration, Instant},
    };

    let start = Instant::now();
    let (finished, completion) = mpsc::channel::<()>();
    let watchdog = thread::Builder::new()
        .name("m5-input-probe-watchdog".into())
        .spawn(move || {
            let remaining = Duration::from_secs(8).saturating_sub(start.elapsed());
            if completion.recv_timeout(remaining) == Err(RecvTimeoutError::Timeout) {
                eprintln!("M5 input probe exceeded the 8-second process limit.");
                std::process::exit(1);
            }
        })?;

    let outcome = oxiview_platform_macos::diagnostic_m5_input_probe();
    drop(finished);
    watchdog
        .join()
        .map_err(|_| "M5 input probe watchdog panicked")?;
    let initial = outcome?;
    println!(
        "M5 input probe passed. AppKit routed W down/up, Command-K, Tab and Shift-Tab. The view's effective appearance changed and was restored."
    );
    println!("Initial preferences {initial:?}");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    Err("the native probe requires macOS".into())
}
