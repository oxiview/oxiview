//! Generate records with the real harness and verify review/accept workflows in isolated directories.
use super::*;
use oxiview_testing::{CompareConfig, RgbaImage, SnapshotError, SnapshotHarness};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("root directory");
        let path = root.join("target/xtask").join(format!(
            "snapshot-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("temporary directory");
        Self(path)
    }
    fn harness(&self, name: &str) -> SnapshotHarness {
        let crate_root = self.0.join(name);
        fs::create_dir_all(&crate_root).expect("caller directory");
        fs::write(
            crate_root.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\n"),
        )
        .expect("manifest");
        SnapshotHarness::new(&self.0, &crate_root, name).expect("harness")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn checkerboard() -> RgbaImage {
    let pixels = (0..16)
        .flat_map(|y| {
            (0..16).flat_map(move |x| {
                let value = if (x + y) % 2 == 0 { 0 } else { 255 };
                [value, value, value, 255]
            })
        })
        .collect();
    RgbaImage::new(16, 16, pixels).expect("checkerboard")
}

#[test]
fn missing_review_accept_and_overwrite_round_trip() {
    let fixture = Fixture::new();
    let harness = fixture.harness("oxiview-example");
    let image = checkerboard();
    let config = CompareConfig::default();
    let baseline = fixture.0.join("oxiview-example/tests/snapshots/board.png");
    assert!(matches!(
        harness.assert_snapshot("board", &image, &config),
        Err(SnapshotError::Mismatch { .. })
    ));
    assert!(!baseline.exists());
    let entries = pending_entries(&fixture.0).expect("pending records");
    assert_eq!(entries.len(), 1);
    let artifacts = &entries[0].directory;
    assert_eq!(
        RgbaImage::read_png(&artifacts.join("actual.png")).expect("actual"),
        image
    );
    assert_eq!(
        &RgbaImage::read_png(&artifacts.join("diff.png"))
            .expect("diff")
            .pixels()[..4],
        &[255, 0, 255, 255]
    );
    let review = process(&fixture.0, None).expect("review");
    assert!(review.contains("oxiview-example/board: Missing"));
    assert!(review.contains("pending.json"));
    assert!(!baseline.exists());
    assert!(
        process(&fixture.0, Some("board"))
            .expect("accept")
            .contains("Created baseline file")
    );
    assert!(!artifacts.join("pending.json").exists());
    harness
        .assert_snapshot("board", &image, &config)
        .expect("passes after acceptance");

    let changed = RgbaImage::new(16, 16, vec![255; 16 * 16 * 4]).expect("changed image");
    assert!(harness.assert_snapshot("board", &changed, &config).is_err());
    assert_eq!(
        RgbaImage::read_png(&baseline).expect("not overwritten automatically"),
        image
    );
    let review = process(&fixture.0, None).expect("review");
    assert!(review.contains("Tolerance exceeded"));
    assert!(review.contains("different_pixels=128"));
    assert!(
        process(&fixture.0, Some("all"))
            .expect("accept all")
            .contains("Overwritten baseline file")
    );
    harness
        .assert_snapshot("board", &changed, &config)
        .expect("passes after update");
    assert!(pending_entries(&fixture.0).expect("cleaned up").is_empty());
}

#[test]
fn dimension_failure_and_stale_record_cleanup() {
    let fixture = Fixture::new();
    let harness = fixture.harness("oxiview-example");
    let image = checkerboard();
    let config = CompareConfig::default();
    assert!(harness.assert_snapshot("size", &image, &config).is_err());
    process(&fixture.0, Some("all")).expect("accept");
    let small = RgbaImage::new(1, 1, vec![0; 4]).expect("small image");
    assert!(harness.assert_snapshot("size", &small, &config).is_err());
    assert!(
        process(&fixture.0, None)
            .expect("review")
            .contains("Dimensions differ")
    );
    harness
        .assert_snapshot("size", &image, &config)
        .expect("passes after restoration");
    assert!(!has_pending(&fixture.0));
    assert!(process(&fixture.0, Some("size")).is_err());
}

#[test]
fn duplicate_names_require_qualification() {
    let fixture = Fixture::new();
    for name in ["oxiview-one", "oxiview-two"] {
        let harness = fixture.harness(name);
        assert!(
            harness
                .assert_snapshot("board", &checkerboard(), &CompareConfig::default())
                .is_err()
        );
    }
    assert!(
        process(&fixture.0, Some("board"))
            .expect_err("reject ambiguity")
            .contains("Duplicate snapshot name")
    );
    process(&fixture.0, Some("oxiview-one/board")).expect("explicit selection");
    assert_eq!(pending_entries(&fixture.0).expect("remaining").len(), 1);
    process(&fixture.0, Some("all")).expect("accept remaining");
}

#[test]
fn invalid_paths_and_corrupt_baselines_are_not_pending() {
    let fixture = Fixture::new();
    let harness = fixture.harness("oxiview-example");
    for name in ["../escape", "a/b", "all", ""] {
        assert!(matches!(
            harness.assert_snapshot(name, &checkerboard(), &CompareConfig::default()),
            Err(SnapshotError::InvalidInput(_))
        ));
    }
    let directory = fixture.0.join("oxiview-example/tests/snapshots");
    fs::create_dir_all(&directory).expect("baseline directory");
    fs::write(directory.join("broken.png"), b"invalid png").expect("corrupted baseline");
    assert!(matches!(
        harness.assert_snapshot("broken", &checkerboard(), &CompareConfig::default()),
        Err(SnapshotError::Png(_))
    ));
    assert!(!has_pending(&fixture.0));
    assert!(validate_destination(&fixture.0, Path::new("../escape.png")).is_err());
}

#[test]
fn failed_test_summary_points_to_review() {
    let fixture = Fixture::new();
    let harness = fixture.harness("oxiview-example");
    assert!(
        harness
            .assert_snapshot("board", &checkerboard(), &CompareConfig::default())
            .is_err()
    );
    struct Failure;
    impl Check for Failure {
        fn name(&self) -> &str {
            "test"
        }
        fn run(&self, _: &Path) -> (Status, String) {
            (Status::Fail, "Pixel snapshot failed".into())
        }
    }
    let result = crate::check::execute(&Failure, &fixture.0);
    assert!(
        result
            .failure_summary
            .expect("failure summary")
            .contains("cargo xtask snapshot --review")
    );
}

#[test]
fn partial_accept_error_preserves_modified_file_log() {
    let fixture = Fixture::new();
    let harness = fixture.harness("oxiview-example");
    for name in ["first", "second"] {
        assert!(
            harness
                .assert_snapshot(name, &checkerboard(), &CompareConfig::default())
                .is_err()
        );
    }
    // Make the second target a directory to verify that the failure report preserves the first file's completed change.
    fs::create_dir_all(fixture.0.join("oxiview-example/tests/snapshots/second.png"))
        .expect("force copy failure");
    let log = process(&fixture.0, Some("all")).expect_err("fails after partial acceptance");
    assert!(log.contains("Created baseline file"));
    assert!(log.contains("first.png"));
    assert!(log.contains("Snapshot acceptance failed"));
    assert_eq!(
        pending_entries(&fixture.0)
            .expect("remaining records")
            .len(),
        1
    );
}

#[test]
fn tampered_pending_cannot_escape_workspace() {
    let fixture = Fixture::new();
    let harness = fixture.harness("oxiview-example");
    assert!(
        harness
            .assert_snapshot("board", &checkerboard(), &CompareConfig::default())
            .is_err()
    );
    let pending = fixture
        .0
        .join("target/xtask/snapshots/oxiview-example/board/pending.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&pending).expect("read record")).expect("JSON");
    value["crate_root"] = "../escape".into();
    fs::write(&pending, serde_json::to_vec(&value).expect("encode")).expect("tamper with path");
    assert!(
        process(&fixture.0, Some("all"))
            .expect_err("reject path escape")
            .contains("ordinary relative paths")
    );
    assert!(pending.exists());
}
