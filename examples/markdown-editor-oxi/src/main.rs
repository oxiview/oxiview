//! Native Markdown editor executable.

fn main() -> std::process::ExitCode {
    match markdown_editor_oxi::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("markdown-editor-oxi: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
