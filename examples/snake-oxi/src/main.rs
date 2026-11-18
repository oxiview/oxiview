//! Runs the native or headless game with the precompiled screen.

fn main() -> std::process::ExitCode {
    snake::cli::main(env!("CARGO_PKG_NAME"), snake_oxi::screen)
}
