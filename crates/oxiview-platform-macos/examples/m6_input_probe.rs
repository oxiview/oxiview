//! Opt-in AppKit smoke check for M6 native text input.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    match (arguments.next(), arguments.next()) {
        (Some(argument), None) if argument == "--run" => match run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("M6 input probe failed. {error}");
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
        "Run m6_input_probe --run on macOS 14 or later to open one window for up to 8 seconds."
    );
}

#[cfg(target_os = "macos")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use std::{sync::mpsc, thread, time::Duration};
    let (complete, completion) = mpsc::channel::<()>();
    let watchdog = thread::Builder::new()
        .name("m6-input-probe-watchdog".into())
        .spawn(move || {
            if completion.recv_timeout(Duration::from_secs(8))
                == Err(mpsc::RecvTimeoutError::Timeout)
            {
                eprintln!("M6 input probe exceeded the 8-second process limit.");
                std::process::exit(1);
            }
        })?;
    let outcome = oxiview_platform_macos::diagnostic_m6_input_probe();
    drop(complete);
    watchdog
        .join()
        .map_err(|_| "M6 input probe watchdog panicked")?;
    outcome?;
    println!("M6 input probe passed.");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    Err("the native probe requires macOS".into())
}
