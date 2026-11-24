//! Native log viewer executable.

fn main() -> std::process::ExitCode {
    match log_viewer_oxi::run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("log-viewer-oxi: {error}");
            std::process::ExitCode::from(2)
        }
    }
}
