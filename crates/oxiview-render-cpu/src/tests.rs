use super::*;
use oxiview_base::Affine;
use oxiview_scene::NodeKind;
use oxiview_testing::{CompareConfig, compare};
#[path = "../tests/scenes/mod.rs"]
mod scenes;
use scenes::{TestResult, blue, fill, red};

const SIZE: PixelSize = PixelSize {
    width: 64,
    height: 64,
};
fn render(name: &str) -> TestResult<RgbaImage> {
    let (scene, root) = scenes::scene(name)?;
    Ok(CpuRenderer.render(&scene, root, SIZE)?)
}
fn pixel(image: &RgbaImage, x: usize, y: usize) -> &[u8] {
    &image.pixels()[(y * image.width() as usize + x) * 4..][..4]
}

#[test]
fn image_draw_preserves_scale_alpha_clips_and_following_paint() -> TestResult {
    let image = RgbaImage::new(
        PixelSize {
            width: 2,
            height: 1,
        },
        vec![255, 0, 0, 128, 0, 255, 0, 255],
    )?;
    let mut scene = Scene::new();
    let root = scene.insert(
        None,
        0,
        NodeKind::Transform(Affine::new([2.0, 0.0, 0.0, 2.0, 0.0, 0.0])),
    )?;
    let group = scene.insert(Some(root), 0, NodeKind::Opacity(0.5))?;
    let clip = scene.insert(
        Some(group),
        0,
        NodeKind::Clip(Clip::Rect(Rect::new(8.0, 8.0, 12.0, 12.0))),
    )?;
    scene.insert(
        Some(clip),
        0,
        NodeKind::Draw(Primitive::Image(Rect::new(8.0, 8.0, 16.0, 12.0), image)),
    )?;
    scene.insert(
        Some(root),
        1,
        NodeKind::Draw(Primitive::Fill(
            Rect::new(20.0, 8.0, 24.0, 12.0),
            CornerRadii::default(),
            Brush::Solid(blue()),
        )),
    )?;
    let rendered = CpuRenderer.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&rendered, 19, 19), [255, 0, 0, 64]);
    assert_eq!(pixel(&rendered, 29, 19), [0, 0, 0, 0]);
    assert_eq!(pixel(&rendered, 43, 19), [0, 0, 255, 255]);
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn straight_alpha_and_group_compositing() -> TestResult {
    let image = render("opacity")?;
    assert_eq!(pixel(&image, 12, 12), [255, 0, 0, 128]);
    assert_eq!(pixel(&image, 30, 30), [0, 0, 255, 128]);
    assert_eq!(pixel(&image, 50, 50), [0, 0, 255, 128]);
    assert_eq!(pixel(&image, 0, 0), [0, 0, 0, 0]);
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn order_reuse_clip_and_linear_interpolation() -> TestResult {
    assert_eq!(pixel(&render("overlap")?, 30, 30), [0, 0, 255, 255]);
    let reused = render("reused-subtree")?;
    assert_eq!(pixel(&reused, 10, 22), pixel(&reused, 42, 22));
    assert_eq!(pixel(&reused, 10, 22), [255, 0, 0, 255]);
    assert_eq!(pixel(&reused, 30, 22), [0, 0, 0, 0]);
    let clipped = render("clip")?;
    assert_eq!(pixel(&clipped, 10, 10), [0, 0, 0, 0]);
    assert_eq!(pixel(&clipped, 30, 30), [255, 0, 0, 255]);
    let gradient = render("linear-gradient")?;
    let midpoint = pixel(&gradient, 32, 20);
    // Linear-light interpolation is brighter than the ~128 sRGB midpoint.
    assert!((180..=195).contains(&midpoint[0]));
    assert!((180..=195).contains(&midpoint[2]));
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn one_changed_pixel_fails_comparison() -> TestResult {
    let rendered = render("rounded-rect")?;
    let expected = oxiview_testing::RgbaImage::new(64, 64, rendered.pixels().to_vec())?;
    let mut changed = rendered.pixels().to_vec();
    changed[0] ^= 1;
    let actual = oxiview_testing::RgbaImage::new(64, 64, changed)?;
    let (equal, _) = compare(&expected, &expected, &CompareConfig::default())?;
    let (different, _) = compare(&actual, &expected, &CompareConfig::default())?;
    assert!(equal.passed);
    assert!(!different.passed);
    assert_eq!(different.diff_pixels, 1);
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn rejects_sizes_and_missing_roots() -> TestResult {
    let (mut scene, root) = scenes::scene("rounded-rect")?;
    for (width, height) in [
        (0, 64),
        (64, 0),
        (65536, 1),
        (1, 65536),
        (u32::MAX, u32::MAX),
    ] {
        assert!(matches!(
            CpuRenderer.render(&scene, root, PixelSize { width, height }),
            Err(Error::InvalidSize(_))
        ));
    }
    assert_eq!(
        dimensions(PixelSize {
            width: 65535,
            height: 65535
        })
        .ok(),
        Some((65535, 65535))
    );
    scene.remove(root)?;
    assert!(matches!(
        CpuRenderer.render(&scene, root, SIZE),
        Err(Error::SceneEncoding(_))
    ));
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn transformed_intersecting_rounded_clips_restore_before_next_draw() -> TestResult {
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Transform(Affine::IDENTITY))?;
    let transform = scene.insert(
        Some(root),
        0,
        NodeKind::Transform(Affine::new([1.0, 0.0, 0.0, 1.0, 16.0, 8.0])),
    )?;
    let outer = scene.insert(
        Some(transform),
        0,
        NodeKind::Clip(Clip::RoundedRect(
            Rect::new(0.0, 0.0, 32.0, 32.0),
            CornerRadii::uniform(8.0),
        )),
    )?;
    let inner = scene.insert(
        Some(outer),
        0,
        NodeKind::Clip(Clip::Rect(Rect::new(0.0, 0.0, 16.0, 32.0))),
    )?;
    scene.insert(
        Some(inner),
        0,
        fill(Rect::new(-8.0, -8.0, 40.0, 40.0), red()),
    )?;
    scene.insert(Some(root), 1, fill(Rect::new(0.0, 0.0, 8.0, 8.0), blue()))?;
    let image = CpuRenderer.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&image, 24, 24), [255, 0, 0, 255]);
    assert_eq!(pixel(&image, 40, 24), [0, 0, 0, 0]);
    assert_eq!(pixel(&image, 16, 8), [0, 0, 0, 0]);
    assert_eq!(pixel(&image, 2, 2), [0, 0, 255, 255]);
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn stroke_and_nested_opacity() -> TestResult {
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Opacity(0.5))?;
    let nested = scene.insert(Some(root), 0, NodeKind::Opacity(0.5))?;
    scene.insert(
        Some(nested),
        0,
        NodeKind::Draw(Primitive::Stroke(
            Rect::new(8.0, 8.0, 56.0, 56.0),
            CornerRadii::uniform(4.0),
            4.0,
            Brush::Solid(red()),
        )),
    )?;
    let image = CpuRenderer.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&image, 8, 32), [255, 0, 0, 64]);
    assert_eq!(pixel(&image, 32, 32), [0, 0, 0, 0]);
    Ok(())
}
