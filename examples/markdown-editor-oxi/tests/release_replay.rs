//! Stable end-to-end coverage of the release acceptance entry point.

use serde_json::Value;
use std::{
    fs::{self, File},
    io::BufReader,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "run the full CPU acceptance replay with cargo test --release"
)]
fn headless_release_replay_preserves_existing_output_files()
-> Result<(), Box<dyn std::error::Error>> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/xtask/replay-tests");
    fs::create_dir_all(&parent)?;
    let output = parent.join(format!("{}-{stamp}", std::process::id()));
    fs::create_dir(&output)?;
    for name in [
        "acceptance.md",
        "roundtrip.md",
        "hill.png",
        "replay.json",
        "document.png",
    ] {
        fs::write(output.join(name), name)?;
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_markdown-editor-oxi"))
        .arg("--headless")
        .arg("--output")
        .arg(&output)
        .arg("--json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(90);
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let log = child.wait_with_output()?;
            return Err(format!(
                "Headless acceptance did not finish. Output in {}. {}",
                output.display(),
                String::from_utf8_lossy(&log.stderr)
            )
            .into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    let log = child.wait_with_output()?;
    assert!(
        log.status.success(),
        "Headless acceptance failed. Output in {}. {}",
        output.display(),
        String::from_utf8_lossy(&log.stderr)
    );
    let report: Value = serde_json::from_slice(&log.stdout)?;
    let artifacts = Path::new(report["output"].as_str().ok_or("Missing run directory")?);
    assert_eq!(artifacts.parent(), Some(output.canonicalize()?.as_path()));
    let manifest: Value = serde_json::from_reader(File::open(artifacts.join("replay.json"))?)?;
    assert_eq!(manifest, report);
    assert_eq!(report["idle_presentations"], 0);
    assert_eq!(report["idle_opportunities"], 1_000);
    assert_eq!(report["preedit_parses"], 0);
    assert_eq!(report["commit_parses"], 1);

    let images = report["images"].as_array().ok_or("Missing CPU captures")?;
    for (state, height) in [
        ("selection", 720),
        ("preedit", 720),
        ("graphics-corrected", 1920),
    ] {
        for (scale, numerator, denominator) in [("1x", 1, 1), ("1.5x", 3, 2), ("2x", 2, 1)] {
            let name = format!("{state}-{scale}.png");
            assert!(
                images
                    .iter()
                    .any(|image| image.as_str() == Some(name.as_str()))
            );
            let decoder = png::Decoder::new(BufReader::new(File::open(artifacts.join(name))?));
            let reader = decoder.read_info()?;
            let info = reader.info();
            assert_eq!(info.width, 1120 * numerator / denominator);
            assert_eq!(info.height, height * numerator / denominator);
        }
    }

    let saved =
        fs::read(artifacts.join(report["roundtrip"].as_str().ok_or("Missing saved file")?))?;
    let text = std::str::from_utf8(&saved)?;
    assert!(text.starts_with("\u{feff}# 山林笔记\r\n"));
    assert!(text.contains("-->|read more|"));
    assert!(text.contains(r"\frac{1}{5}"));
    assert!(text.contains("Saved via the normal Save Action.\r\n"));
    for name in [
        "acceptance.md",
        "roundtrip.md",
        "hill.png",
        "replay.json",
        "document.png",
    ] {
        assert_eq!(fs::read_to_string(output.join(name))?, name);
    }
    fs::remove_dir_all(output)?;
    Ok(())
}
