//! Command-line entry point for the oxiview language server.

use std::{io, process::ExitCode};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    match (args.next().as_deref().and_then(|s| s.to_str()), args.next()) {
        (Some("--version" | "-V"), None) => println!("oxiview-lsp {}", env!("CARGO_PKG_VERSION")),
        (Some("--help" | "-h"), None) => println!(
            "oxiview-lsp [--stdio | --version]\nServe .oxi editor requests on stdin and stdout."
        ),
        (None | Some("--stdio"), None) => {
            if let Err(error) = oxiview_lsp::serve(io::stdin().lock(), io::stdout().lock()) {
                eprintln!("oxiview-lsp: {error}");
                return ExitCode::FAILURE;
            }
        }
        _ => {
            eprintln!("usage: oxiview-lsp [--stdio | --version]");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}
