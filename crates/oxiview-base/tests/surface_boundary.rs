//! Guard the sole non-adapter unsafe declaration and the transferable handle.

use oxiview_base::NativeSurfaceHandle;
use std::{fs, path::Path};

fn check_source(
    path: &Path,
    declarations: &mut usize,
    allowances: &mut usize,
) -> std::io::Result<()> {
    for entry in fs::read_dir(path)? {
        let path = entry?.path();
        if path.is_dir() {
            check_source(&path, declarations, allowances)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = fs::read_to_string(&path)?;
            // Tokenize punctuation as well as whitespace so line breaks cannot
            // conceal a block or implementation. Conservative about comments.
            let tokens: Vec<_> = source
                .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
                .filter(|token| !token.is_empty())
                .collect();
            for pair in tokens.windows(2) {
                if pair == ["unsafe", "impl"] {
                    return Err(std::io::Error::other(format!(
                        "unsafe impl in {}",
                        path.display()
                    )));
                }
                if pair == ["unsafe", "fn"] {
                    *declarations += 1;
                }
            }
            let compact: String = source.chars().filter(|ch| !ch.is_whitespace()).collect();
            if compact.contains("unsafe{") {
                return Err(std::io::Error::other(format!(
                    "unsafe block in {}",
                    path.display()
                )));
            }
            *allowances += compact.matches("#[allow(unsafe_code)]").count();
            if compact.contains("#![allow(unsafe_code)]") {
                return Err(std::io::Error::other("crate/module unsafe allowance"));
            }
        }
    }
    Ok(())
}

#[test]
fn base_has_only_the_surface_constructor_declaration() -> std::io::Result<()> {
    let mut declarations = 0;
    let mut allowances = 0;
    check_source(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut declarations,
        &mut allowances,
    )?;
    assert_eq!(declarations, 1);
    assert_eq!(allowances, 1);
    Ok(())
}

#[test]
fn handle_is_transferable_without_native_access() {
    fn require_traits<T: Send + Sync + Copy>() {}
    require_traits::<NativeSurfaceHandle>();
}
