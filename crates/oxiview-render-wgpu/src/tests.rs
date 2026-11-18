use super::*;
use oxiview_base::{Affine, Color, CornerRadii, Point, Rect};
use oxiview_scene::{Brush, Clip, ColorStop, NodeKind, Primitive};

type TestResult = Result<(), Box<dyn std::error::Error>>;
const SIZE: PixelSize = PixelSize {
    width: 65,
    height: 64,
}; // Exercises padded rows and partial workgroups.
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
        fill(Rect::new(20.0, 8.0, 24.0, 12.0), blue()),
    )?;
    let rendered = RenderContext::new()?.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&rendered, 19, 19), [255, 0, 0, 64]);
    assert_eq!(pixel(&rendered, 29, 19), [0, 0, 0, 0]);
    assert_eq!(pixel(&rendered, 43, 19), [0, 0, 255, 255]);
    Ok(())
}

#[test]
fn repeated_image_draws_share_gpu_pixel_storage() -> TestResult {
    let image = RgbaImage::new(
        PixelSize {
            width: 64,
            height: 64,
        },
        vec![255; 64 * 64 * 4],
    )?;
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Transform(Affine::IDENTITY))?;
    scene.insert(
        Some(root),
        0,
        NodeKind::Draw(Primitive::Image(
            Rect::new(0.0, 0.0, 16.0, 16.0),
            image.clone(),
        )),
    )?;
    let once = encoding::Commands::new(&scene.encode(root)?, SIZE)?
        .bytes()
        .len();
    scene.insert(
        Some(root),
        1,
        NodeKind::Draw(Primitive::Image(
            Rect::new(24.0, 0.0, 40.0, 16.0),
            image.clone(),
        )),
    )?;
    let twice = encoding::Commands::new(&scene.encode(root)?, SIZE)?
        .bytes()
        .len();
    assert!(twice - once < image.pixels().len());
    Ok(())
}

#[test]
fn filled_box_border_shares_outer_edge_coverage() -> TestResult {
    let gpu = RenderContext::new()?;
    let outer = Rect::new(4.0, 4.0, 60.0, 60.0);
    let radii = CornerRadii::uniform(24.0);
    let background = Color::from_rgba8(18, 35, 30, 255);
    let paper = Color::from_rgba8(245, 246, 239, 255);
    for border in [
        Color::from_rgba8(38, 61, 46, 255),
        Color::from_rgba8(255, 0, 0, 128),
    ] {
        let mut scene = Scene::new();
        let root = scene.insert(None, 0, fill(Rect::new(0.0, 0.0, 65.0, 64.0), paper))?;
        scene.insert(
            Some(root),
            0,
            NodeKind::Draw(Primitive::Fill(outer, radii, Brush::Solid(background))),
        )?;
        let clip = scene.insert(
            Some(root),
            1,
            NodeKind::Clip(Clip::RoundedRect(outer, radii)),
        )?;
        scene.insert(
            Some(clip),
            0,
            NodeKind::Draw(Primitive::Stroke(
                Rect::new(4.5, 4.5, 59.5, 59.5),
                CornerRadii::uniform(23.5),
                1.0,
                Brush::Solid(border),
            )),
        )?;
        let actual = gpu.render(&scene, root, SIZE)?;

        // At this outer-edge pixel, the inside border fully replaces the box
        // background wherever the shape covers. Compare with the corresponding
        // single fill, rather than accepting opacity from two averaged edges.
        let foreground = border.to_srgb();
        let behind = background.to_srgb();
        let effective = [
            foreground[0] * foreground[3] + behind[0] * (1.0 - foreground[3]),
            foreground[1] * foreground[3] + behind[1] * (1.0 - foreground[3]),
            foreground[2] * foreground[3] + behind[2] * (1.0 - foreground[3]),
        ]
        .map(|value| (value * 255.0).round() as u8);
        let mut reference = Scene::new();
        let reference_root =
            reference.insert(None, 0, fill(Rect::new(0.0, 0.0, 65.0, 64.0), paper))?;
        reference.insert(
            Some(reference_root),
            0,
            NodeKind::Draw(Primitive::Fill(
                outer,
                radii,
                Brush::Solid(Color::from_rgba8(
                    effective[0],
                    effective[1],
                    effective[2],
                    255,
                )),
            )),
        )?;
        let expected = gpu.render(&reference, reference_root, SIZE)?;
        for (actual, expected) in pixel(&actual, 12, 9).iter().zip(pixel(&expected, 12, 9)) {
            assert!(
                actual.abs_diff(*expected) <= 1,
                "correlated outer edge differs: {actual} vs {expected}"
            );
        }
        assert_eq!(pixel(&actual, 32, 32), [18, 35, 30, 255]);
    }
    Ok(())
}

#[test]
fn differently_clipped_border_does_not_replace_background_coverage() -> TestResult {
    let gpu = RenderContext::new()?;
    let outer = Rect::new(4.0, 4.0, 60.0, 60.0);
    let radii = CornerRadii::uniform(24.0);
    let mut scene = Scene::new();
    let root = scene.insert(
        None,
        0,
        NodeKind::Draw(Primitive::Fill(outer, radii, Brush::Solid(blue()))),
    )?;
    let before = gpu.render(&scene, root, SIZE)?;
    let clip = scene.insert(
        Some(root),
        0,
        NodeKind::Clip(Clip::Rect(Rect::new(32.0, 0.0, 65.0, 64.0))),
    )?;
    scene.insert(
        Some(clip),
        0,
        NodeKind::Draw(Primitive::Stroke(
            Rect::new(4.5, 4.5, 59.5, 59.5),
            CornerRadii::uniform(23.5),
            1.0,
            Brush::Solid(red()),
        )),
    )?;
    let after = gpu.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&before, 12, 9), pixel(&after, 12, 9));
    assert_ne!(pixel(&before, 51, 9), pixel(&after, 51, 9));
    Ok(())
}

#[test]
fn tiled_render_matches_unculled_scene_at_edges_and_nested_groups() -> TestResult {
    let context = RenderContext::new()?;
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Opacity(0.7))?;
    scene.insert(Some(root), 0, NodeKind::Opacity(0.3))?; // Empty scope.
    for i in 0..40 {
        let transform = match i % 4 {
            0 => Affine::IDENTITY,
            1 => Affine::new([0.0, 1.5, -1.5, 0.0, 64.25, -8.5]),
            2 => Affine::new([-1.0, 0.0, 0.0, 1.0, 65.0, 0.0]),
            _ => Affine::new([1.2, 0.25, 0.3, 0.8, -4.0, 0.25]),
        };
        let group = scene.insert(Some(root), i + 1, NodeKind::Opacity(0.6))?;
        let transformed = scene.insert(Some(group), 0, NodeKind::Transform(transform))?;
        let clip = scene.insert(
            Some(transformed),
            0,
            NodeKind::Clip(Clip::RoundedRect(
                Rect::new(15.8, -8.0, 63.8, 66.0),
                CornerRadii::uniform(3.0),
            )),
        )?;
        let x = (i % 8) as f64 * 16.0 - 16.2;
        let y = (i / 8) as f64 * 16.0 - 0.2;
        let rect = Rect::new(x, y, x + 16.0, y + 17.0);
        let brush = Brush::Solid(Color::from_rgba8((i * 5) as u8, 180, 100, 190));
        let primitive = if i % 2 == 0 {
            Primitive::Stroke(rect, CornerRadii::uniform(4.0), 3.5, brush)
        } else {
            Primitive::Fill(rect, CornerRadii::uniform(4.0), brush)
        };
        scene.insert(Some(clip), 0, NodeKind::Draw(primitive))?;
    }
    let tiled = context.render(&scene, root, SIZE)?;
    let commands = encoding::Commands::unbinned(&scene.encode(root)?, SIZE)?;
    let unculled = context
        .process
        .lock()
        .map_err(|e| e.to_string())?
        .render(&commands, SIZE)?;
    assert_eq!(tiled.pixels(), unculled.pixels());
    assert!(tiled.pixels().iter().any(|b| *b != 0));
    Ok(())
}

#[test]
fn sparse_thousand_primitive_scene_keeps_tile_work_local() -> TestResult {
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Opacity(0.5))?;
    for i in 0..1000 {
        let x = (i % 40) as f64 * 32.0 + 2.0;
        let y = (i / 40) as f64 * 32.0 + 2.0;
        scene.insert(
            Some(root),
            i,
            fill(Rect::new(x, y, x + 24.0, y + 24.0), red()),
        )?;
    }
    let size = PixelSize {
        width: 1280,
        height: 840,
    };
    let data = encoding::Commands::new(&scene.encode(root)?, size)?.bytes();
    let words: Vec<_> = data
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let table = words[3] as usize;
    let tiles = size.width.div_ceil(16) * size.height.div_ceil(16);
    for tile in 0..tiles as usize {
        // At most one primitive plus its opacity begin/end, never all 1,000.
        assert!(words[table + tile * 2 + 1] <= 3);
    }
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions report test failures only.
fn offscreen_readback_and_process_wide_recovery() -> TestResult {
    let first = RenderContext::new()?;
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
    let context = RenderContext::new()?;
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
    let context = RenderContext::new()?;
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
    let context = RenderContext::new()?;
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
    let context = RenderContext::new()?;
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
    let context = RenderContext::new()?;
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

#[test]
#[allow(clippy::panic)] // No native pointer is fabricated to exercise rejection.
fn surface_kind_and_configuration_validation() -> TestResult {
    use oxiview_base::NativeSurfaceKind;
    assert!(matches!(
        surface::validate_kind(NativeSurfaceKind::MacosMetalLayer, false),
        Err(Error::UnsupportedSurface)
    ));
    surface::validate_kind(NativeSurfaceKind::MacosMetalLayer, true)?;
    for (width, height) in [(0, 0), (0, 64), (64, 0)] {
        assert!(!surface::validate_size(PixelSize { width, height }, 4096)?);
    }
    assert!(surface::validate_size(SIZE, 4096)?);
    assert!(
        surface::validate_size(
            PixelSize {
                width: 4097,
                height: 1
            },
            4096
        )
        .is_err()
    );
    assert!(
        surface::validate_size(
            PixelSize {
                width: 1,
                height: u32::MAX
            },
            4096
        )
        .is_err()
    );
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Inspect the actual encoded shape tags, not only the predicate.
fn sdf_and_polygon_path_selection() -> TestResult {
    for (affine, expected) in [
        (Affine::IDENTITY, true),
        (Affine::new([0.0, 2.0, -2.0, 0.0, 32.0, 4.0]), true),
        (Affine::new([-1.0, 0.0, 0.0, 1.0, 32.0, 0.0]), true),
        (Affine::new([1.0, 0.0, 0.25, 1.0, 0.0, 0.0]), false),
        (Affine::new([2.0, 0.0, 0.0, 1.0, 0.0, 0.0]), false),
    ] {
        let mut scene = Scene::new();
        let root = scene.insert(None, 0, NodeKind::Transform(affine))?;
        let clip = scene.insert(
            Some(root),
            0,
            NodeKind::Clip(Clip::RoundedRect(
                Rect::new(0.0, 0.0, 32.0, 32.0),
                CornerRadii::uniform(4.0),
            )),
        )?;
        scene.insert(Some(clip), 0, fill(Rect::new(4.0, 4.0, 28.0, 28.0), red()))?;
        let bytes = encoding::Commands::new(&scene.encode(root)?, SIZE)?.bytes();
        let words: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        assert_eq!(words[words[6] as usize] == u32::MAX, expected);
        assert_eq!(words[words[10] as usize] == u32::MAX, expected);
    }
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Compare real GPU bytes from the presentation copy path.
fn surface_copy_matches_offscreen_and_recovers() -> TestResult {
    let context = RenderContext::new()?;
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Opacity(0.7))?;
    scene.insert(
        Some(root),
        0,
        NodeKind::Draw(Primitive::Fill(
            Rect::new(4.0, 4.0, 60.0, 60.0),
            CornerRadii::uniform(9.0),
            Brush::LinearGradient(
                Point::new(4.0, 4.0),
                Point::new(60.0, 60.0),
                vec![
                    ColorStop {
                        position: 0.0,
                        color: Color::from_rgba8(23, 150, 240, 255),
                    },
                    ColorStop {
                        position: 1.0,
                        color: Color::from_rgba8(231, 60, 19, 128),
                    },
                ],
            ),
        )),
    )?;
    let commands = encoding::Commands::new(&scene.encode(root)?, SIZE)?;
    // Hold the process lock across each diagnostic cycle so other tests cannot
    // destroy this device between direct internal operations.
    let mut process = context.process.lock().map_err(|e| e.to_string())?;
    for _ in 0..2 {
        process.ensure_device()?;
        let gpu = process.gpu.as_ref().ok_or("missing GPU")?;
        let expected = gpu
            .render(&commands, SIZE, None, true)?
            .ok_or("missing pixels")?;
        for format in [
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Bgra8Unorm,
        ] {
            let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("surface-like target"),
                size: wgpu::Extent3d {
                    width: SIZE.width,
                    height: SIZE.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let actual = gpu
                .render(&commands, SIZE, Some(&target), true)?
                .ok_or("missing copied pixels")?;
            assert_eq!(actual, expected, "copy format {format:?}");
        }
        process.lose_device();
    }
    Ok(())
}
