//! Exercises the release acceptance CLI and its persisted output.

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
fn headless_cli_writes_isolated_acceptance_evidence() -> Result<(), Box<dyn std::error::Error>> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let parent = std::env::home_dir()
        .ok_or("Could not locate the home directory")?
        .join(".tmp/codex-unknown");
    fs::create_dir_all(&parent)?;
    let output = parent.join(format!("log-viewer-test-{}-{stamp}", std::process::id()));
    fs::create_dir(&output)?;
    for name in ["replay.json", "expanded-list.png", "stream.log"] {
        fs::write(output.join(name), name)?;
    }

    let executable = Path::new(env!("CARGO_BIN_EXE_log-viewer-oxi"));
    #[cfg(target_os = "macos")]
    let executable = {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("package-macos.sh");
        let stage = Command::new(script)
            .arg("--from-build")
            .arg(executable)
            .arg(&output)
            .output()?;
        assert!(
            stage.status.success(),
            "{}",
            String::from_utf8_lossy(&stage.stderr)
        );
        let contents = output.join("Oxiview Logs.app/Contents");
        let identifier = Command::new("/usr/bin/plutil")
            .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
            .arg(contents.join("Info.plist"))
            .output()?;
        assert!(identifier.status.success());
        assert_eq!(
            String::from_utf8(identifier.stdout)?.trim(),
            "dev.oxiview.log-viewer"
        );
        assert!(
            contents
                .join("Resources/licenses/NotoSans-OFL.txt")
                .is_file()
        );
        let staged = contents.join("MacOS/log-viewer-oxi");
        assert!(
            Command::new("/usr/bin/cmp")
                .arg("-s")
                .arg(executable)
                .arg(&staged)
                .status()?
                .success()
        );
        staged
    };
    let mut child = Command::new(executable)
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
            child.kill()?;
            let log = child.wait_with_output()?;
            return Err(format!(
                "Release replay did not finish. Output in {}. {}",
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
        "Release replay failed. Output in {}. {}",
        output.display(),
        String::from_utf8_lossy(&log.stderr)
    );

    let report: Value = serde_json::from_slice(&log.stdout)?;
    let artifacts = Path::new(report["output"].as_str().ok_or("Missing run directory")?);
    assert_eq!(artifacts.parent(), Some(output.canonicalize()?.as_path()));
    let saved_report: Value = serde_json::from_reader(File::open(artifacts.join("replay.json"))?)?;
    assert_eq!(saved_report, report);
    assert_eq!(report["idle_opportunities"], 1_000);
    assert_eq!(report["idle_submissions"], 0);
    assert_eq!(report["idle_presentations"], 0);

    let checks = report["checks"].as_array().ok_or("Missing checks")?;
    for check in [
        "fractional-wheel-framework-momentum-and-thumb-interruption",
        "seeded-100k-bounded-mounts-expanded-row-clipping-and-remeasurement",
        "table-mode-keeps-key-and-row-header-copies-raw",
        "ime-candidate-preedit-commit-severity-keyboard-and-clear",
        "native-open-watch-append-follow-and-pause",
        "row-and-column-recycling-exact-tsv-append-resize-and-sort",
        "filter-collapses-to-surviving-cell-clears-removed-key-and-disables-copy",
        "pause-anchor-and-distinct-partial-utf8-arrivals-explicit-resume",
        "nested-inline-detail-transfers-only-unused-scroll-distance",
        "watcher-truncate-identical-rotation-read-error-cancel-and-switch",
        "idle-1000-opportunities",
    ] {
        assert!(checks.iter().any(|value| value == check), "Missing {check}");
    }
    let expected_images = [
        "expanded-list.png",
        "offscreen-table-range.png",
        "switched-source.png",
    ];
    assert_eq!(report["images"], serde_json::to_value(expected_images)?);
    for name in expected_images {
        let decoder = png::Decoder::new(BufReader::new(File::open(artifacts.join(name))?));
        let mut reader = decoder.read_info()?;
        let mut pixels = vec![
            0;
            reader
                .output_buffer_size()
                .ok_or("Invalid PNG output size")?
        ];
        let info = reader.next_frame(&mut pixels)?;
        assert_eq!((info.width, info.height), (640, 720), "{name}");
        assert_eq!(info.color_type, png::ColorType::Rgba, "{name}");
        assert!(
            pixels.chunks_exact(4).any(|pixel| pixel != &pixels[..4]),
            "{name} is blank"
        );
    }
    assert!(!artifacts.join(".log-fixture").exists());
    for name in ["replay.json", "expanded-list.png", "stream.log"] {
        assert_eq!(fs::read_to_string(output.join(name))?, name);
    }
    fs::remove_dir_all(output)?;
    Ok(())
}
