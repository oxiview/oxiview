//! 通过真实 harness 生成记录，在隔离目录验证审阅与接受流程。
use super::*;
use oxiview_testing::{CompareConfig, RgbaImage, SnapshotError, SnapshotHarness};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("根目录");
        let path = root.join("target/xtask").join(format!(
            "snapshot-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("临时目录");
        Self(path)
    }
    fn harness(&self, name: &str) -> SnapshotHarness {
        let crate_root = self.0.join(name);
        fs::create_dir_all(&crate_root).expect("调用方目录");
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
    RgbaImage::new(16, 16, pixels).expect("棋盘格")
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
    let entries = pending_entries(&fixture.0).expect("待处理记录");
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
    assert!(review.contains("oxiview-example/board：缺失"));
    assert!(review.contains("pending.json"));
    assert!(!baseline.exists());
    assert!(
        process(&fixture.0, Some("board"))
            .expect("accept")
            .contains("已创建基准文件")
    );
    assert!(!artifacts.join("pending.json").exists());
    harness
        .assert_snapshot("board", &image, &config)
        .expect("接受后通过");

    let changed = RgbaImage::new(16, 16, vec![255; 16 * 16 * 4]).expect("变化图");
    assert!(harness.assert_snapshot("board", &changed, &config).is_err());
    assert_eq!(RgbaImage::read_png(&baseline).expect("未自动覆盖"), image);
    let review = process(&fixture.0, None).expect("review");
    assert!(review.contains("超出容差"));
    assert!(review.contains("差异像素=128"));
    assert!(
        process(&fixture.0, Some("all"))
            .expect("accept all")
            .contains("已覆盖基准文件")
    );
    harness
        .assert_snapshot("board", &changed, &config)
        .expect("更新后通过");
    assert!(pending_entries(&fixture.0).expect("已清理").is_empty());
}

#[test]
fn dimension_failure_and_stale_record_cleanup() {
    let fixture = Fixture::new();
    let harness = fixture.harness("oxiview-example");
    let image = checkerboard();
    let config = CompareConfig::default();
    assert!(harness.assert_snapshot("size", &image, &config).is_err());
    process(&fixture.0, Some("all")).expect("接受");
    let small = RgbaImage::new(1, 1, vec![0; 4]).expect("小图");
    assert!(harness.assert_snapshot("size", &small, &config).is_err());
    assert!(
        process(&fixture.0, None)
            .expect("review")
            .contains("尺寸不一致")
    );
    harness
        .assert_snapshot("size", &image, &config)
        .expect("恢复后通过");
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
            .expect_err("拒绝歧义")
            .contains("名称重复")
    );
    process(&fixture.0, Some("oxiview-one/board")).expect("明确选择");
    assert_eq!(pending_entries(&fixture.0).expect("剩余").len(), 1);
    process(&fixture.0, Some("all")).expect("接受剩余");
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
    fs::create_dir_all(&directory).expect("基准目录");
    fs::write(directory.join("broken.png"), b"invalid png").expect("损坏的基准");
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
            (Status::Fail, "像素快照失败".into())
        }
    }
    let result = crate::check::execute(&Failure, &fixture.0);
    assert!(
        result
            .failure_summary
            .expect("失败汇总")
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
    // 第二个目标故意为目录，验证失败报告保留已修改的第一个文件。
    fs::create_dir_all(fixture.0.join("oxiview-example/tests/snapshots/second.png"))
        .expect("制造复制失败");
    let log = process(&fixture.0, Some("all")).expect_err("部分接受后失败");
    assert!(log.contains("已创建基准文件"));
    assert!(log.contains("first.png"));
    assert!(log.contains("快照接受失败"));
    assert_eq!(pending_entries(&fixture.0).expect("剩余记录").len(), 1);
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
        serde_json::from_slice(&fs::read(&pending).expect("读取记录")).expect("JSON");
    value["crate_root"] = "../escape".into();
    fs::write(&pending, serde_json::to_vec(&value).expect("编码")).expect("篡改路径");
    assert!(
        process(&fixture.0, Some("all"))
            .expect_err("拒绝越界")
            .contains("普通相对路径")
    );
    assert!(pending.exists());
}
