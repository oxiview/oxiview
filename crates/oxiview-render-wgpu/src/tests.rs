use super::*;
use oxiview_base::{Affine, Color, CornerRadii, Point, Rect};
use oxiview_scene::{Brush, Clip, ColorStop, NodeKind, Primitive};

type TestResult = Result<(), Box<dyn std::error::Error>>;
const SIZE: PixelSize = PixelSize {
    width: 65,
    height: 64,
}; // Exercises padded rows and partial workgroups.
fn context() -> Result<Option<RenderContext>, Error> {
    match RenderContext::new() {
        Ok(context) => Ok(Some(context)),
        Err(Error::NoAdapter(reason)) => {
            eprintln!("Skipping GPU test: no adapter available ({reason})");
            Ok(None)
        }
        Err(error) => Err(error),
    }
}
fn fill(rect: Rect, color: Color) -> NodeKind {
    NodeKind::Draw(Primitive::Fill(
        rect,
        CornerRadii::default(),
        Brush::Solid(color),
    ))
}
fn red() -> Color {
    Color::from_rgba8(255, 0, 0, 255)
}
fn blue() -> Color {
    Color::from_rgba8(0, 0, 255, 255)
}
fn pixel(image: &RgbaImage, x: usize, y: usize) -> &[u8] {
    &image.pixels()[(y * image.width() as usize + x) * 4..][..4]
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn offscreen_readback_and_process_wide_recovery() -> TestResult {
    let Some(first) = context()? else {
        return Ok(());
    };
    let second = RenderContext::new()?;
    let clone = first.clone();
    assert!(Arc::ptr_eq(&first.process, &second.process));
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, fill(Rect::new(0.0, 0.0, 65.0, 64.0), red()))?;
    let before = first.render(&scene, root, SIZE)?;
    assert_eq!(before.pixels().len(), 65 * 64 * 4);
    assert_eq!(pixel(&before, 64, 63), [255, 0, 0, 255]);
    first.simulate_device_loss()?;
    // Each handle existed before the loss, including the independent constructor.
    for context in [&first, &second, &clone] {
        let after = context.render(&scene, root, SIZE)?;
        assert_eq!(after.width(), 65);
        assert_eq!(after.height(), 64);
        assert_eq!(pixel(&after, 32, 32), [255, 0, 0, 255]);
    }
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn isolated_nested_opacity_order_and_straight_alpha() -> TestResult {
    let Some(context) = context()? else {
        return Ok(());
    };
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Opacity(0.5))?;
    let group = scene.insert(Some(root), 0, NodeKind::Opacity(0.5))?;
    scene.insert(Some(group), 0, fill(Rect::new(8.0, 8.0, 40.0, 40.0), red()))?;
    scene.insert(
        Some(group),
        1,
        fill(Rect::new(24.0, 24.0, 56.0, 56.0), blue()),
    )?;
    let image = context.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&image, 12, 12), [255, 0, 0, 64]);
    assert_eq!(pixel(&image, 30, 30), [0, 0, 255, 64]);
    assert_eq!(pixel(&image, 50, 50), [0, 0, 255, 64]);
    assert_eq!(pixel(&image, 0, 0), [0, 0, 0, 0]);
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn transformed_intersecting_clips_and_state_restoration() -> TestResult {
    let Some(context) = context()? else {
        return Ok(());
    };
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Transform(Affine::IDENTITY))?;
    // Shear and translation apply to geometry and both clips.
    let transformed = scene.insert(
        Some(root),
        0,
        NodeKind::Transform(Affine::new([1.0, 0.0, 0.25, 1.0, 16.0, 8.0])),
    )?;
    let clip = scene.insert(
        Some(transformed),
        0,
        NodeKind::Clip(Clip::RoundedRect(
            Rect::new(0.0, 0.0, 32.0, 32.0),
            CornerRadii::uniform(8.0),
        )),
    )?;
    let inner = scene.insert(
        Some(clip),
        0,
        NodeKind::Clip(Clip::Rect(Rect::new(0.0, 0.0, 16.0, 32.0))),
    )?;
    scene.insert(
        Some(inner),
        0,
        fill(Rect::new(-8.0, -8.0, 40.0, 40.0), red()),
    )?;
    scene.insert(Some(root), 1, fill(Rect::new(0.0, 0.0, 8.0, 8.0), blue()))?;
    let image = context.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&image, 28, 24), [255, 0, 0, 255]);
    assert_eq!(pixel(&image, 44, 24), [0, 0, 0, 0]);
    assert_eq!(pixel(&image, 16, 8), [0, 0, 0, 0]);
    assert_eq!(pixel(&image, 2, 2), [0, 0, 255, 255]);
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn rounded_stroke_and_reused_subtrees() -> TestResult {
    let Some(context) = context()? else {
        return Ok(());
    };
    let mut scene = Scene::new();
    let shape = scene.insert(
        None,
        0,
        NodeKind::Draw(Primitive::Stroke(
            Rect::new(4.0, 4.0, 24.0, 24.0),
            CornerRadii::uniform(6.0),
            4.0,
            Brush::Solid(red()),
        )),
    )?;
    let root = scene.insert(None, 1, NodeKind::Transform(Affine::IDENTITY))?;
    scene.insert(Some(root), 0, NodeKind::Reference(shape, Affine::IDENTITY))?;
    scene.insert(
        Some(root),
        1,
        NodeKind::Reference(shape, Affine::new([1.0, 0.0, 0.0, 1.0, 32.0, 0.0])),
    )?;
    let image = context.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&image, 14, 4), [255, 0, 0, 255]);
    assert_eq!(pixel(&image, 46, 4), [255, 0, 0, 255]);
    assert_eq!(pixel(&image, 14, 14), [0, 0, 0, 0]);
    assert_eq!(pixel(&image, 3, 3), [0, 0, 0, 0]);
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn gradient_linear_light_and_hard_stops() -> TestResult {
    let Some(context) = context()? else {
        return Ok(());
    };
    let mut scene = Scene::new();
    let gradient = Brush::LinearGradient(
        Point::new(0.0, 0.0),
        Point::new(64.0, 0.0),
        vec![
            ColorStop {
                position: 0.0,
                color: red(),
            },
            ColorStop {
                position: 1.0,
                color: blue(),
            },
        ],
    );
    let root = scene.insert(
        None,
        0,
        NodeKind::Draw(Primitive::Fill(
            Rect::new(0.0, 0.0, 64.0, 64.0),
            CornerRadii::uniform(8.0),
            gradient,
        )),
    )?;
    let image = context.render(&scene, root, SIZE)?;
    let midpoint = pixel(&image, 32, 20);
    assert!((180..=195).contains(&midpoint[0]));
    assert!((180..=195).contains(&midpoint[2]));
    assert_eq!(pixel(&image, 0, 0), [0, 0, 0, 0]);
    let gradient = Brush::LinearGradient(
        Point::new(0.0, 0.0),
        Point::new(64.0, 0.0),
        vec![
            ColorStop {
                position: 0.5,
                color: red(),
            },
            ColorStop {
                position: 0.5,
                color: blue(),
            },
        ],
    );
    scene.replace(
        root,
        NodeKind::Draw(Primitive::Fill(
            Rect::new(0.0, 0.0, 64.0, 64.0),
            CornerRadii::default(),
            gradient,
        )),
    )?;
    let image = context.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&image, 10, 10), [255, 0, 0, 255]);
    assert_eq!(pixel(&image, 50, 10), [0, 0, 255, 255]);
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn rejects_invalid_sizes_and_roots() -> TestResult {
    let Some(context) = context()? else {
        return Ok(());
    };
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, fill(Rect::new(0.0, 0.0, 4.0, 4.0), red()))?;
    for (width, height) in [(0, 1), (1, 0), (u32::MAX, u32::MAX), (u32::MAX, 1)] {
        assert!(matches!(
            context.render(&scene, root, PixelSize { width, height }),
            Err(Error::InvalidSize(_))
        ));
    }
    scene.remove(root)?;
    assert!(matches!(
        context.render(&scene, root, SIZE),
        Err(Error::SceneEncoding(_))
    ));
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn unpremultiplication_rounds_clamps_and_clears() {
    assert_eq!(straight(64, 128), 128);
    assert_eq!(straight(129, 128), 255);
    assert_eq!(straight(255, 0), 0);
}

#[test]
fn shader_validates_without_a_device() -> TestResult {
    let module = wgpu::naga::front::wgsl::parse_str(include_str!("raster.wgsl"))?;
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::empty(),
    )
    .validate(&module)?;
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn encoding_rejects_nonfinite_geometry_without_a_device() -> TestResult {
    let mut scene = Scene::new();
    let root = scene.insert(
        None,
        0,
        fill(Rect::new(0.0, 0.0, f64::INFINITY, 4.0), red()),
    )?;
    assert!(matches!(
        encoding::Commands::new(&scene.encode(root)?, SIZE),
        Err(Error::SceneEncoding(_))
    ));
    Ok(())
}
