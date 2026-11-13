//! Rendering snapshots; baselines are accepted only after human review.
mod scenes;
use oxiview_render_cpu::{CpuRenderer, PixelSize, Renderer};
use oxiview_testing::{CompareConfig, RgbaImage, SnapshotHarness};
use scenes::TestResult;
use std::path::Path;

fn snapshot(name: &str) -> TestResult {
    let (scene, root) = scenes::scene(name)?;
    let image = CpuRenderer.render(
        &scene,
        root,
        PixelSize {
            width: 64,
            height: 64,
        },
    )?;
    let image = RgbaImage::new(image.width(), image.height(), image.pixels().to_vec())?;
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = crate_root.join("../..").canonicalize()?;
    SnapshotHarness::new(&workspace, crate_root, env!("CARGO_PKG_NAME"))?.assert_snapshot(
        name,
        &image,
        &CompareConfig::default(),
    )?;
    Ok(())
}
#[test]
fn rounded_rect() -> TestResult {
    snapshot("rounded-rect")
}
#[test]
fn linear_gradient() -> TestResult {
    snapshot("linear-gradient")
}
#[test]
fn clip() -> TestResult {
    snapshot("clip")
}
#[test]
fn opacity() -> TestResult {
    snapshot("opacity")
}
#[test]
fn reused_subtree() -> TestResult {
    snapshot("reused-subtree")
}
#[test]
fn overlap() -> TestResult {
    snapshot("overlap")
}
