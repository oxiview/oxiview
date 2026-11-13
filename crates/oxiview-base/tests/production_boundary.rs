//! Check real Cargo artifacts for the absence and presence of test-only system adapters.

use std::{fs, path::PathBuf, process::Command};

fn build_artifact(arguments: &[&str], test: bool) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO"))
        .args(arguments)
        .args(["-p", "oxiview-base", "--locked", "--message-format=json"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "cargo {} failed: {}\n{}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        )
        .into());
    }

    // Use the paths Cargo reports for this invocation, never a glob that could
    // pick an old artifact or the integration test containing our search strings.
    for line in String::from_utf8(output.stdout)?.lines() {
        let message: serde_json::Value = serde_json::from_str(line)?;
        if message["reason"] != "compiler-artifact"
            || message["target"]["name"] != "oxiview_base"
            || message["profile"]["test"] != test
        {
            continue;
        }
        if test {
            if let Some(executable) = message["executable"].as_str() {
                return Ok(executable.into());
            }
        } else if let Some(filenames) = message["filenames"].as_array() {
            for filename in filenames.iter().filter_map(serde_json::Value::as_str) {
                let path = PathBuf::from(filename);
                if path
                    .extension()
                    .is_some_and(|extension| extension == "rlib")
                {
                    return Ok(path);
                }
            }
        }
    }
    Err(format!("Cargo did not report the oxiview-base artifact (test={test})").into())
}

#[test]
#[allow(clippy::panic)] // Assertions compare real production and unit-test artifacts, including a positive control.
fn system_adapters_exist_only_in_test_artifacts() -> Result<(), Box<dyn std::error::Error>> {
    let production_artifact = build_artifact(&["build"], false)?;
    let production = fs::read(&production_artifact)?;
    // --no-run prevents recursive execution of this integration test. Cargo's
    // profile.test and executable fields identify the library's unit-test binary.
    let test_artifact = build_artifact(&["test", "--no-run"], true)?;
    let tests = fs::read(&test_artifact)?;

    for name in ["SystemClock", "SystemScheduler", "SystemRng"] {
        // Search raw bytes so names in both symbols and strings are detected,
        // without depending on a platform-specific symbol inspection utility.
        assert!(
            !production
                .windows(name.len())
                .any(|bytes| bytes == name.as_bytes()),
            "{name} leaked into the production library: {}",
            production_artifact.display(),
        );
        assert!(
            tests
                .windows(name.len())
                .any(|bytes| bytes == name.as_bytes()),
            "{name} is missing from the unit-test binary: {}",
            test_artifact.display(),
        );
    }
    Ok(())
}
