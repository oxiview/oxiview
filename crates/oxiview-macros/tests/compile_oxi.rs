//! Exercises Cargo file tracking and rustc diagnostics through a pure-.oxi binary.

use std::{
    error::Error,
    fs,
    path::Path,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

fn cargo(directory: &Path, command: &str) -> std::io::Result<Output> {
    Command::new(env!("CARGO"))
        .args([command, "--quiet", "--offline"])
        .env("CARGO_TARGET_DIR", directory.join("target"))
        .current_dir(directory)
        .output()
}

fn expect_stdout(output: Output, expected: &str) -> Result<(), Box<dyn Error>> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout)?.trim(), expected);
    Ok(())
}

#[test]
fn rebuilds_either_file_and_checks_each_scope() -> Result<(), Box<dyn Error>> {
    let macros = env!("CARGO_MANIFEST_DIR");
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let directory = Path::new(macros)
        .join("../../target/xtask")
        .join(format!("oxi-compile-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&directory)?;
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            r#"
[workspace]
[package]
name = "oxi-fixture"
version = "0.0.0"
edition = "2024"
[dependencies]
oxiview-macros = {{ path = {macros:?} }}
[[bin]]
name = "oxi-fixture"
path = "main.oxi"
"#
        ),
    )?;
    fs::write(
        directory.join("main.oxi"),
        r#"
extern crate self as oxiview_core;
extern crate self as oxiview_style;
type View = u32;
#[derive(Default)]
struct Style { weight: u32 }
mod logic { oxiview_macros::oxi_module!("logic.oxi"); }
mod named { oxiview_macros::oxi_module!("named.oxi"); }
fn first(value: u32) -> View { oxiview_macros::oxi_view!("fragment.oxi") }
fn second(value: &u32) -> View { oxiview_macros::oxi_view!("fragment.oxi") }
fn main() { println!("{}", logic::sum(first(3), second(&4)) + named::offset(1)); }
"#,
    )?;
    let named = directory.join("named.oxi");
    let fragment = directory.join("fragment.oxi");
    let logic = directory.join("logic.oxi");
    let logic_source = "pub(super) fn sum(left: u32, right: u32) -> u32 { left + right }";
    let items = r#"
style weight<T>(input: T) where T: Into<u32> { weight: input.into() }
pub(super) view offset(value: u32) { let next = weight(value).weight; { next + 1 } }
"#;
    fs::write(&named, items)?;
    fs::write(&logic, logic_source)?;
    fs::write(&fragment, "{ value + 1 }")?;
    expect_stdout(cargo(&directory, "run")?, "11")?;
    fs::write(&fragment, "{ value + 20 }")?;
    expect_stdout(cargo(&directory, "run")?, "49")?;
    fs::write(&named, items.replace("next + 1", "next + 100"))?;
    expect_stdout(cargo(&directory, "run")?, "148")?;

    fs::remove_file(&named)?;
    fs::remove_file(&fragment)?;
    fs::remove_file(&logic)?;
    let binary = directory
        .join("target/debug")
        .join(format!("oxi-fixture{}", std::env::consts::EXE_SUFFIX));
    expect_stdout(
        Command::new(binary).current_dir(&directory).output()?,
        "148",
    )?;
    fs::write(&named, items)?;
    fs::write(&logic, logic_source)?;

    for (source, diagnostic) in [
        ("<Node></Wrong>", "fragment.oxi: closing tag does not match"),
        ("{ unknown_local }", "cannot find value `unknown_local`"),
        ("{ String::new() }", "mismatched types"),
        (
            "{ let x = String::new(); let _taken = x; drop(x); value + 1 }",
            "use of moved value",
        ),
        // The same expression is valid at first() and invalid at second().
        ("{ value }", "expected `u32`, found `&u32`"),
    ] {
        fs::write(&fragment, source)?;
        let output = cargo(&directory, "check")?;
        let errors = String::from_utf8(output.stderr)?;
        assert!(
            !output.status.success() && errors.contains(diagnostic),
            "{errors}"
        );
    }
    fs::write(&fragment, "{ value + 1 }")?;
    for (source, diagnostic) in [
        (
            "pub(super) view offset(_: u32) { { value } }",
            "cannot find value `value`",
        ),
        (
            "pub(super) view offset(_: u32) { { 0 } } style bad() { weight: false }",
            "mismatched types",
        ),
    ] {
        fs::write(&named, source)?;
        let output = cargo(&directory, "check")?;
        let errors = String::from_utf8(output.stderr)?;
        assert!(
            !output.status.success() && errors.contains(diagnostic),
            "{errors}"
        );
    }
    fs::remove_dir_all(directory)?;
    Ok(())
}
