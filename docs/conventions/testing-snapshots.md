# Pixel snapshots

`oxiview-testing` is the M-1 test harness. It handles only generated or caller-provided RGBA8 images, without window or renderer dependencies. Callers inherit `oxiview-testing.workspace = true` in dev-dependencies.

```rust
use std::path::Path;
use oxiview_testing::{CompareConfig, RgbaImage, SnapshotHarness};

// This example assumes <workspace>/crates/<crate>; pass the workspace root explicitly for other layouts.
let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
let workspace_root = crate_root.parent().expect("crates").parent().expect("workspace");
let harness = SnapshotHarness::new(workspace_root, crate_root, env!("CARGO_PKG_NAME"))?;
let image = RgbaImage::new(1, 1, vec![255, 0, 0, 255])?;
harness.assert_snapshot("red", &image, &CompareConfig::default())?;
# Ok::<(), oxiview_testing::SnapshotError>(())
```

Both roots must be existing absolute directories, independent of the process working directory. Baselines live in the caller's `tests/snapshots/<name>.png`, must be RGBA8 PNGs, and are committed directly to Git. Names allow only ASCII letters, digits, hyphens, and underscores; `all` is reserved. Use a different snapshot name for each test in a crate to avoid parallel tests overwriting artifacts. Do not run tests and accept concurrently.

Exact equality is the default. `channel_tolerance` ranges from 0–255 and checks all four channels; a difference equal to the tolerance passes. `allowed_diff_ratio` must be a finite number from 0–1: any channel exceeding tolerance marks one differing pixel, and a ratio equal to the limit passes. Different dimensions always fail. Metrics use the bounding rectangle of both images; missing pixels count as different with a maximum channel difference of 255. The diff image fades pixels within tolerance and highlights differing pixels in opaque magenta. When the baseline is missing, the whole image is highlighted and difference metrics are absent.

Failures write `<workspace>/target/xtask/snapshots/<crate>/<name>/actual.png`, `diff.png`, and `pending.json`. Metadata records the caller's relative root, name, reason, and metrics. Tests never write baselines. A successful rerun clears stale pending records; corrupted PNGs return codec errors instead of being accepted as missing baselines.

1. Run `cargo xtask snapshot --review` to inspect names, reasons, metrics, and paths, and have a human review actual and diff images.
2. After confirming the change is expected, run `cargo xtask snapshot --accept <name>`, or `<crate>/<name>` to disambiguate duplicate names. `--accept all` accepts every pending item.
3. The command explicitly reports baseline paths created or overwritten and removes corresponding pending records. actual/diff remain in target for inspection. Rerun tests and review baseline changes in Git.

`snapshot` is not included in `verify`, so verify never accepts snapshots. On snapshot test failure, the test summary suggests running review. The command follows xtask reporting conventions, writing `target/xtask/report.json` and `snapshot.log`; `--json` prints the report, with review details in its referenced log. Acceptance proceeds item by item, so a filesystem error may occur after some baselines have changed. Inspect baseline changes and remaining pending records before retrying.

The added dependencies [`png 0.18.1`](https://github.com/image-rs/image-png) and [`thiserror 2.0.21`](https://github.com/dtolnay/thiserror) are maintained pure Rust projects licensed MIT OR Apache-2.0. Only default stable features are enabled; the full image library is not introduced. serde/serde_json reuse workspace versions, and `cargo xtask deps` handles dependency review.
