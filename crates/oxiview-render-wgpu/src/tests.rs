use super::*;
use oxiview_base::{Affine, Color, CornerRadii, Point, Rect};
use oxiview_scene::{Brush, Clip, ColorStop, NodeKind, Primitive};

mod colr;

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
fn a_flat_fullscreen_scene_needs_no_pixel_sized_opacity_stack() -> TestResult {
    let size = PixelSize {
        width: 3456,
        height: 2168,
    };
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, fill(Rect::new(0.0, 0.0, 100.0, 150.0), red()))?;
    let commands = encoding::Commands::new(&scene.encode(root)?, size)?;
    assert_eq!(commands.depth, 0);
    assert_eq!(commands.opacity_stack_bytes(size)?, 16);
    let group = scene.insert(Some(root), 0, NodeKind::Opacity(0.6))?;
    let inner = scene.insert(Some(group), 0, NodeKind::Opacity(0.5))?;
    scene.insert(Some(inner), 0, fill(Rect::new(2.0, 3.0, 8.0, 9.0), blue()))?;
    let nested = encoding::Commands::new(&scene.encode(root)?, size)?;
    assert_eq!(nested.depth, 2);
    assert_eq!(nested.opacity_stack_bytes(size)?, 3456 * 2168 * 2 * 16);
    Ok(())
}

#[test]
fn solid_and_constant_gradient_agree_after_isolated_compositing() -> TestResult {
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, fill(Rect::new(0.0, 0.0, 65.0, 64.0), blue()))?;
    let group = scene.insert(Some(root), 0, NodeKind::Opacity(0.73))?;
    for (i, c) in [
        Color::from_rgba8(123, 67, 194, 177),
        Color::from_linear([0.25, 0.5, 0.85, 0.5]),
        Color::from_srgb([1.2, -0.1, 0.6, 0.8]),
        Color::from_rgba8(22, 99, 199, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let y = (i * 12) as f64;
        scene.insert(
            Some(group),
            2 * i,
            fill(Rect::new(0.0, y, 20.0, y + 10.0), c),
        )?;
        scene.insert(
            Some(group),
            2 * i + 1,
            NodeKind::Draw(Primitive::Fill(
                Rect::new(25.0, y, 45.0, y + 10.0),
                CornerRadii::default(),
                Brush::LinearGradient(
                    Point::new(25.0, 0.0),
                    Point::new(45.0, 0.0),
                    vec![
                        ColorStop {
                            position: 0.0,
                            color: c,
                        },
                        ColorStop {
                            position: 1.0,
                            color: c,
                        },
                    ],
                ),
            )),
        )?;
    }
    let image = RenderContext::new()?.render(&scene, root, SIZE)?;
    for y in [4, 16, 28, 40] {
        assert!(
            pixel(&image, 5, y)
                .iter()
                .zip(pixel(&image, 30, y))
                .all(|(&a, &b)| a.abs_diff(b) <= 1)
        );
    }
    Ok(())
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
fn vello_full_background_clear_matches_the_clipped_draw_path() -> TestResult {
    let bounds = Rect::new(0.0, 0.0, f64::from(SIZE.width), f64::from(SIZE.height));
    let image = RgbaImage::new(
        PixelSize {
            width: 1,
            height: 1,
        },
        vec![14, 91, 162, 111],
    )?;
    let renderer = RenderContext::new()?;
    let render = |root_kind| -> Result<RgbaImage, Box<dyn std::error::Error>> {
        let mut scene = Scene::new();
        let root = scene.insert(None, 0, root_kind)?;
        for (index, color) in [
            red(),
            Color::from_rgba8(19, 41, 77, 255),
            Color::from_rgba8(255, 0, 255, 0),
        ]
        .into_iter()
        .enumerate()
        {
            scene.insert(Some(root), index, fill(bounds, color))?;
        }
        scene.insert(
            Some(root),
            3,
            NodeKind::Draw(Primitive::Image(
                Rect::new(8.0, 9.0, 28.0, 19.0),
                image.clone(),
            )),
        )?;
        Ok(renderer.render(&scene, root, SIZE)?)
    };
    let cleared = render(NodeKind::Transform(Affine::IDENTITY))?;
    assert_eq!(pixel(&cleared, 0, 0), [19, 41, 77, 255]);
    assert_ne!(pixel(&cleared, 12, 13), pixel(&cleared, 0, 0));
    assert_eq!(cleared, render(NodeKind::Clip(Clip::Rect(bounds)))?);
    let clipped = render(NodeKind::Clip(Clip::Rect(Rect::new(5.0, 5.0, 40.0, 20.0))))?;
    assert_eq!(pixel(&clipped, 0, 0), [0, 0, 0, 0]);
    assert_eq!(pixel(&clipped, 12, 13), pixel(&cleared, 12, 13));
    Ok(())
}

#[test]
fn vello_rect_fills_match_path_coverage_with_and_without_clips() -> TestResult {
    use oxiview_scene::{BezierPath, FillRule, PathCommand::*};

    let renderer = RenderContext::new()?;
    let render =
        |through_path, clip: Option<Clip>| -> Result<RgbaImage, Box<dyn std::error::Error>> {
            let mut scene = Scene::new();
            let root_kind = clip.map_or(NodeKind::Transform(Affine::IDENTITY), NodeKind::Clip);
            let root = scene.insert(None, 0, root_kind)?;
            let opacity = scene.insert(Some(root), 0, NodeKind::Opacity(0.75))?;
            let transform = scene.insert(
                Some(opacity),
                0,
                NodeKind::Transform(Affine::new([2.0, 0.0, 0.0, 2.0, 2.0, 4.0])),
            )?;
            let empty = BezierPath::new(Vec::new())?;
            scene.insert(
                Some(transform),
                0,
                NodeKind::Draw(Primitive::FillPath(
                    empty,
                    FillRule::NonZero,
                    Brush::Solid(red()),
                )),
            )?;
            for (index, (rect, color)) in [
                (Rect::new(-1.0, 2.0, 21.5, 18.0), red()),
                (
                    Rect::new(2.0, 7.0, 17.5, 12.5),
                    Color::from_rgba8(19, 41, 77, 173),
                ),
                (Rect::new(6.125, 11.25, 13.5, 22.0), blue()),
            ]
            .into_iter()
            .enumerate()
            {
                let shape = if through_path {
                    let [x0, y0, x1, y1] = rect.edges();
                    NodeKind::Draw(Primitive::FillPath(
                        BezierPath::new(vec![
                            MoveTo(Point::new(x0, y0)),
                            LineTo(Point::new(x1, y0)),
                            LineTo(Point::new(x1, y1)),
                            LineTo(Point::new(x0, y1)),
                            Close,
                        ])?,
                        FillRule::NonZero,
                        Brush::Solid(color),
                    ))
                } else {
                    fill(rect, color)
                };
                scene.insert(Some(transform), index + 1, shape)?;
            }
            Ok(renderer.render(&scene, root, SIZE)?)
        };

    let bounds = Rect::new(2.25, 3.75, 40.5, 40.25);
    for clip in [
        None,
        Some(Clip::Rect(bounds)),
        Some(Clip::RoundedRect(bounds, CornerRadii::uniform(4.0))),
    ] {
        let aligned = render(false, clip.clone())?;
        let path = render(true, clip.clone())?;
        let different = aligned
            .pixels()
            .chunks_exact(4)
            .zip(path.pixels().chunks_exact(4))
            .position(|(aligned, path)| aligned != path);
        assert_eq!(different, None, "clip {clip:?}");
        assert_ne!(pixel(&aligned, 21, 21), &[0, 0, 0, 0]);
    }
    Ok(())
}

#[test]
fn vello_reuses_adjacent_clips_across_draws_and_restores_isolated_scopes() -> TestResult {
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Transform(Affine::IDENTITY))?;
    let outer = scene.insert(
        Some(root),
        0,
        NodeKind::Clip(Clip::RoundedRect(
            Rect::new(8.25, 8.25, 40.25, 40.25),
            CornerRadii::uniform(4.0),
        )),
    )?;
    let mask = RgbaImage::new(
        PixelSize {
            width: 1,
            height: 1,
        },
        vec![255, 0, 0, 255],
    )?;
    scene.insert(
        Some(outer),
        0,
        NodeKind::Draw(Primitive::Image(Rect::new(4.0, 10.0, 24.0, 30.0), mask)),
    )?;
    scene.insert(
        Some(outer),
        1,
        fill(Rect::new(6.0, 14.0, 22.0, 18.0), red()),
    )?;
    let opacity = scene.insert(Some(outer), 2, NodeKind::Opacity(0.5))?;
    let inner = scene.insert(
        Some(opacity),
        0,
        NodeKind::Clip(Clip::Rect(Rect::new(16.0, 0.0, 32.0, 32.0))),
    )?;
    scene.insert(
        Some(inner),
        0,
        fill(Rect::new(14.0, 4.0, 33.0, 26.0), blue()),
    )?;
    scene.insert(
        Some(inner),
        1,
        fill(Rect::new(20.0, 4.0, 31.0, 26.0), blue()),
    )?;
    scene.insert(
        Some(outer),
        3,
        fill(Rect::new(26.0, 32.0, 55.0, 36.0), red()),
    )?;
    scene.insert(
        Some(root),
        1,
        fill(Rect::new(46.0, 12.0, 54.0, 20.0), blue()),
    )?;

    let rendered = RenderContext::new()?.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&rendered, 6, 16), [0, 0, 0, 0]);
    assert_eq!(pixel(&rendered, 13, 16), [255, 0, 0, 255]);
    assert_eq!(pixel(&rendered, 29, 6), [0, 0, 0, 0]);
    assert_eq!(pixel(&rendered, 29, 15), [0, 0, 255, 128]);
    assert_eq!(pixel(&rendered, 37, 34), [255, 0, 0, 255]);
    assert_eq!(pixel(&rendered, 47, 34), [0, 0, 0, 0]);
    assert_eq!(pixel(&rendered, 48, 16), [0, 0, 255, 255]);
    Ok(())
}

#[test]
fn repeated_image_draws_keep_texture_residency_across_frames() -> TestResult {
    let raster = RgbaImage::new(
        PixelSize {
            width: 8,
            height: 8,
        },
        [161, 32, 199, 73].repeat(8 * 8),
    )?;
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Transform(Affine::IDENTITY))?;
    for (index, left) in [4.0, 24.0].into_iter().enumerate() {
        scene.insert(
            Some(root),
            index,
            NodeKind::Draw(Primitive::Image(
                Rect::new(left, 4.0, left + 8.0, 12.0),
                raster.clone(),
            )),
        )?;
    }
    let encoded = scene.encode(root)?;
    let context = RenderContext::new()?;
    let mut process = context.process.lock().map_err(|e| e.to_string())?;
    process.ensure_device()?;
    let gpu = process.gpu.as_mut().ok_or("missing GPU")?;
    let before = gpu
        .vector
        .as_ref()
        .map_or(0, |backend| backend.atlas.counters.uploads);
    let first = gpu
        .render_scene(&encoded, SIZE, None, true)?
        .ok_or("missing pixels")?;
    let warm = gpu.vector.as_ref().ok_or("missing Vello")?.atlas.counters;
    assert_eq!(warm.uploads - before, 1);
    assert_eq!(pixel(&first, 7, 7), [161, 32, 199, 73]);
    assert_eq!(pixel(&first, 27, 7), [161, 32, 199, 73]);
    // A frame on the fast path must not reset the GPU atlas.
    gpu.render_scene(&oxiview_scene::EncodedScene::default(), SIZE, None, false)?;
    for _ in 0..3 {
        let later = gpu
            .render_scene(&encoded, SIZE, None, true)?
            .ok_or("missing pixels")?;
        assert_eq!(later, first);
    }
    let steady = gpu.vector.as_ref().ok_or("missing Vello")?.atlas.counters;
    assert_eq!(steady.uploads, warm.uploads);
    assert_eq!(steady.overflow_uploads, warm.overflow_uploads);
    assert_eq!(steady.hits - warm.hits, 6);
    Ok(())
}

#[test]
fn atlas_evicts_unused_entries_and_draws_pinned_overflow() -> TestResult {
    let size = PixelSize {
        width: 1024,
        height: 1024,
    };
    let first = RgbaImage::new(size, [0, 255, 0, 255].repeat(1024 * 1024))?;
    let second = RgbaImage::new(size, [255, 0, 0, 255].repeat(1024 * 1024))?;
    let third = RgbaImage::new(
        PixelSize {
            width: 1025,
            height: 1,
        },
        [0, 0, 255, 255].repeat(1025),
    )?;
    let image = |left: f64, source: RgbaImage| {
        NodeKind::Draw(Primitive::Image(
            Rect::new(left, 0.0, left + 16.0, 16.0),
            source,
        ))
    };
    let context = RenderContext::new()?;
    let mut process = context.process.lock().map_err(|e| e.to_string())?;
    process.ensure_device()?;
    let gpu = process.gpu.as_mut().ok_or("missing GPU")?;
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Opacity(1.0))?;
    let old = scene.insert(Some(root), 0, image(0.0, first))?;
    gpu.render_scene(&scene.encode(root)?, SIZE, None, true)?;
    let before = gpu.vector.as_ref().ok_or("missing Vello")?.atlas.counters;

    // The resident entry is pinned even when it appears after a cache miss.
    scene.insert(Some(root), 0, image(17.0, second))?;
    scene.insert(Some(root), 2, image(34.0, third))?;
    let output = gpu
        .render_scene(&scene.encode(root)?, SIZE, None, true)?
        .ok_or("missing pixels")?;
    assert_eq!(pixel(&output, 8, 8), [0, 255, 0, 255]);
    assert_eq!(pixel(&output, 25, 8), [255, 0, 0, 255]);
    assert_eq!(pixel(&output, 42, 8), [0, 0, 255, 255]);
    let atlas = &gpu.vector.as_ref().ok_or("missing Vello")?.atlas;
    let overflow = atlas.counters;
    assert_eq!(overflow.uploads, before.uploads);
    assert_eq!(overflow.evictions, before.evictions);
    assert_eq!(overflow.overflow_uploads - before.overflow_uploads, 2);
    assert_eq!(atlas.resident_count(), 1);

    scene.remove(old)?;
    let output = gpu
        .render_scene(&scene.encode(root)?, SIZE, None, true)?
        .ok_or("missing pixels")?;
    assert_eq!(pixel(&output, 8, 8), [0, 0, 0, 0]);
    assert_eq!(pixel(&output, 25, 8), [255, 0, 0, 255]);
    let atlas = &gpu.vector.as_ref().ok_or("missing Vello")?.atlas;
    assert_eq!(atlas.counters.uploads - overflow.uploads, 1);
    assert_eq!(atlas.counters.evictions - overflow.evictions, 1);
    assert_eq!(atlas.resident_count(), 1);
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
        let foreground = border.to_linear();
        let behind = background.to_linear();
        let effective = Color::from_linear([
            foreground[0] * foreground[3] + behind[0] * (1.0 - foreground[3]),
            foreground[1] * foreground[3] + behind[1] * (1.0 - foreground[3]),
            foreground[2] * foreground[3] + behind[2] * (1.0 - foreground[3]),
            1.0,
        ]);
        let mut reference = Scene::new();
        let reference_root =
            reference.insert(None, 0, fill(Rect::new(0.0, 0.0, 65.0, 64.0), paper))?;
        reference.insert(
            Some(reference_root),
            0,
            NodeKind::Draw(Primitive::Fill(outer, radii, Brush::Solid(effective))),
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
    let encoded = scene.encode(root)?;
    let commands = encoding::Commands::new(&encoded, SIZE)?;
    let oracle = encoding::Commands::unbinned(&encoded, SIZE)?;
    let mut process = context.process.lock().map_err(|e| e.to_string())?;
    let tiled = process.render(&commands, SIZE)?;
    let unculled = process.render(&oracle, SIZE)?;
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

fn fixture_glyph(id: u16, kind: oxiview_base::GlyphKind, color: Color) -> NodeKind {
    const FONT: &[u8] = include_bytes!("../../oxiview-text/tests/fonts/OxiviewColorFixture.ttf");
    NodeKind::Draw(Primitive::Glyph(oxiview_base::Glyph {
        font: oxiview_base::GlyphFont::new(Arc::from(FONT)),
        face_index: 0,
        glyph_id: id,
        size: 50.0,
        origin: Point::new(8.0, 52.0),
        color,
        variations: Arc::from([]),
        kind,
    }))
}

#[test]
fn vello_draws_outline_through_transformed_clip() -> TestResult {
    let mut scene = Scene::new();
    let root = scene.insert(
        None,
        0,
        NodeKind::Transform(Affine::new([1.0, 0.0, 0.5, 1.0, -12.0, 0.0])),
    )?;
    let clip = scene.insert(
        Some(root),
        0,
        NodeKind::Clip(Clip::Rect(Rect::new(10.0, 12.0, 27.0, 52.0))),
    )?;
    scene.insert(
        Some(clip),
        0,
        fixture_glyph(
            2,
            oxiview_base::GlyphKind::Outline,
            Color::from_rgba8(130, 20, 48, 160),
        ),
    )?;
    let rendered = RenderContext::new()?.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&rendered, 26, 40), [130, 20, 48, 160]);
    assert_eq!(pixel(&rendered, 40, 45), [0, 0, 0, 0]);
    assert_eq!(pixel(&rendered, 16, 40), [0, 0, 0, 0]);
    assert_eq!(pixel(&rendered, 26, 55), [0, 0, 0, 0]);
    Ok(())
}

#[test]
fn vello_executes_colrv1_multiply_and_applies_glyph_opacity_once() -> TestResult {
    let mut scene = Scene::new();
    let root = scene.insert(
        None,
        0,
        fixture_glyph(
            4,
            oxiview_base::GlyphKind::Color,
            Color::from_rgba8(245, 70, 12, 128),
        ),
    )?;
    let rendered = RenderContext::new()?.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&rendered, 12, 32), [0, 255, 255, 128]);
    assert_eq!(pixel(&rendered, 28, 32), [0, 255, 0, 128]);
    assert_eq!(pixel(&rendered, 44, 32), [255, 255, 0, 128]);
    assert_eq!(pixel(&rendered, 52, 32), [0, 0, 0, 0]);
    Ok(())
}

#[test]
fn vello_executes_colrv1_sweep_in_font_orientation() -> TestResult {
    let mut scene = Scene::new();
    let root = scene.insert(
        None,
        0,
        fixture_glyph(5, oxiview_base::GlyphKind::Color, red()),
    )?;
    let rendered = RenderContext::new()?.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&rendered, 38, 22), [255, 0, 0, 255]);
    assert_eq!(pixel(&rendered, 18, 22), [0, 255, 0, 255]);
    assert_eq!(pixel(&rendered, 18, 42), [0, 0, 255, 255]);
    assert_eq!(pixel(&rendered, 38, 42), [255, 255, 0, 255]);
    Ok(())
}

#[test]
fn vello_composites_images_and_gradients_in_linear_light() -> TestResult {
    let image = RgbaImage::new(
        PixelSize {
            width: 2,
            height: 1,
        },
        vec![255, 0, 0, 255, 0, 0, 255, 255],
    )?;
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Transform(Affine::IDENTITY))?;
    scene.insert(
        Some(root),
        0,
        NodeKind::Draw(Primitive::Image(Rect::new(0.0, 0.0, 65.0, 12.0), image)),
    )?;
    scene.insert(
        Some(root),
        1,
        NodeKind::Draw(Primitive::Fill(
            Rect::new(0.0, 16.0, 65.0, 28.0),
            CornerRadii::default(),
            Brush::LinearGradient(
                Point::new(0.0, 16.0),
                Point::new(65.0, 16.0),
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
            ),
        )),
    )?;
    scene.insert(Some(root), 2, fill(Rect::new(0.0, 32.0, 65.0, 44.0), red()))?;
    let group = scene.insert(Some(root), 3, NodeKind::Opacity(0.5))?;
    for i in 0..2 {
        scene.insert(
            Some(group),
            i,
            fill(Rect::new(0.0, 32.0, 65.0, 44.0), blue()),
        )?;
    }
    let rendered = RenderContext::new()?.render(&scene, root, SIZE)?;
    for y in [6, 22, 38] {
        let [r, g, b, a] = pixel(&rendered, 32, y) else {
            return Err("invalid readback pixel".into());
        };
        assert!(r.abs_diff(188) <= 1, "red at row {y}: {r}");
        assert!(b.abs_diff(188) <= 1, "blue at row {y}: {b}");
        assert_eq!((*g, *a), (0, 255));
    }
    Ok(())
}

#[test]
fn vello_surface_output_preserves_alpha_and_device_recovery() -> TestResult {
    let raster = RgbaImage::new(
        PixelSize {
            width: 1,
            height: 1,
        },
        vec![221, 42, 103, 129],
    )?;
    let mut scene = Scene::new();
    let root = scene.insert(
        None,
        0,
        NodeKind::Draw(Primitive::Image(Rect::new(8.0, 8.0, 22.0, 22.0), raster)),
    )?;
    let encoded = scene.encode(root)?;
    let context = RenderContext::new()?;
    let mut process = context.process.lock().map_err(|e| e.to_string())?;
    process.lose_device();
    for _ in 0..2 {
        process.ensure_device()?;
        let gpu = process.gpu.as_mut().ok_or("missing GPU")?;
        assert!(gpu.vector.is_none());
        let output = gpu
            .render_scene(&encoded, SIZE, None, true)?
            .ok_or("missing pixels")?;
        assert_eq!(pixel(&output, 16, 16), [221, 42, 103, 129]);
        for format in copy::SURFACE_FORMATS {
            let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Vello surface-like target"),
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
            let surface = gpu
                .render_scene(&encoded, SIZE, Some(&target), true)?
                .ok_or("missing surface pixels")?;
            assert_eq!(pixel(&surface, 0, 0), [0, 0, 0, 0]);
            for (&actual, &expected) in pixel(&surface, 16, 16).iter().zip(pixel(&output, 16, 16)) {
                assert!(
                    actual.abs_diff(expected) <= 1,
                    "{format:?}: {actual} != {expected}"
                );
            }
        }
        let atlas = &gpu.vector.as_ref().ok_or("missing Vello")?.atlas;
        assert_eq!(atlas.counters.uploads, 1);
        assert_eq!(atlas.counters.hits, 4);
        process.lose_device();
    }
    Ok(())
}

#[test]
fn vello_uses_cpal_palette_zero_at_nonzero_record_offset() -> TestResult {
    let NodeKind::Draw(Primitive::Glyph(mut glyph)) =
        fixture_glyph(4, oxiview_base::GlyphKind::Color, red())
    else {
        return Err("missing fixture glyph".into());
    };
    let original = glyph.font.bytes();
    let face = skrifa::FontRef::from_index(original, 0)?;
    let cpal = face
        .table_data(skrifa::Tag::new(b"CPAL"))
        .ok_or("missing CPAL")?;
    let offset = cpal.as_bytes().as_ptr().addr() - original.as_ptr().addr();
    let mut bytes = original.to_vec();
    // Shift the single palette's start by one record and reduce its length
    // accordingly. The composite uses palette indices 2 and 3, still in range.
    bytes[offset + 2..offset + 4].copy_from_slice(&4u16.to_be_bytes());
    bytes[offset + 12..offset + 14].copy_from_slice(&1u16.to_be_bytes());
    glyph.font = oxiview_base::GlyphFont::new(bytes.into());
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Draw(Primitive::Glyph(glyph)))?;
    let output = RenderContext::new()?.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&output, 12, 32), [255, 255, 0, 255]);
    assert_eq!(pixel(&output, 28, 32), [0, 255, 0, 255]);
    assert_eq!(pixel(&output, 44, 32), [0, 255, 0, 255]);
    Ok(())
}
