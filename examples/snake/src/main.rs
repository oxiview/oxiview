//! Native or headless Rust Snake entry point.

fn main() -> std::process::ExitCode {
    snake::cli::main(env!("CARGO_PKG_NAME"), snake::application::screen)
}
