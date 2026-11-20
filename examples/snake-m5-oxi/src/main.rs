//! Launches the native M5 demo or its deterministic acceptance replay.

fn main() -> std::process::ExitCode {
    match snake_m5_oxi::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("snake-m5-oxi: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
