//! Sampled CPU and native GPU coverage of asymmetric rounded geometry.

use oxiview_base::{Affine, Color, CornerRadii, Point, Rect, Size};
use oxiview_core::UiTree;
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_render_wgpu::RenderContext;
use oxiview_scene::{Brush, Clip, NodeKind, PixelSize, Primitive, RgbaImage, Scene};
use oxiview_style::{Length, Overflow, Position, Style};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Rgba = [u8; 4];

const VIEWPORT: Size = Size::new(96.0, 72.0);
const TRANSPARENT: Rgba = [0; 4];
const CANVAS: Rgba = [12, 24, 36, 255];
const PANEL: Rgba = [40, 80, 120, 255];
const BORDER: Rgba = [244, 196, 48, 255];
const RED: Rgba = [200, 40, 20, 255];
const BLUE: Rgba = [20, 40, 200, 255];

fn color([r, g, b, a]: Rgba) -> Color {
    Color::from_rgba8(r, g, b, a)
}

fn absolute([left, top, width, height]: [f64; 4], background: Rgba) -> Style {
    let mut style = Style {
        background: color(background),
        ..Style::default()
    };
    style.layout.position = Position::Absolute;
    style.layout.width = Length::Points(width);
    style.layout.height = Length::Points(height);
    style.layout.inset.left = Length::Points(left);
    style.layout.inset.top = Length::Points(top);
    style
}

fn pixel_size(scale: f64) -> PixelSize {
    PixelSize {
        width: (VIEWPORT.width() * scale) as u32,
        height: (VIEWPORT.height() * scale) as u32,
    }
}

fn assert_sample(images: [&RgbaImage; 2], physical: Point, expected: Rgba, sample: &str) {
    let x = physical.x() as usize;
    let y = physical.y() as usize;
    for (renderer, image) in ["CPU", "GPU"].into_iter().zip(images) {
        let offset = (y * image.width() as usize + x) * 4;
        let actual = &image.pixels()[offset..offset + 4];
        assert!(
            actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
            "{renderer} {sample} at ({x}, {y}): {actual:?} != {expected:?}"
        );
    }
}

#[test]
fn fitted_ui_border_clip_opacity_and_hits_match_at_each_scale() -> TestResult {
    let gpu = RenderContext::new()?;
    let mut tree = UiTree::new()?;
    let root = tree.root();
    let mut root_style = tree.style(root)?.clone();
    root_style.background = color(CANVAS);
    root_style.hit_test = false;
    tree.set_style(root, root_style)?;

    let mut panel = absolute([8.0, 8.0, 64.0, 40.0], PANEL);
    // The right edge limits the common factor to 40 / (36 + 54).
    // The fitted top-left radius is 32, well past the vertical midpoint.
    panel.border_radii = CornerRadii::new(72.0, 36.0, 54.0, 8.0);
    panel.layout.border_width = 4.0;
    panel.border_color = color(BORDER);
    panel.overflow = Overflow::Clip;
    let panel = tree.append(root, panel)?;
    let mut group = absolute([-4.0, -4.0, 72.0, 48.0], TRANSPARENT);
    group.opacity = 0.5;
    group.hit_test = false;
    let group = tree.append(panel, group)?;
    let red = tree.append(group, absolute([0.0, 0.0, 40.0, 48.0], RED))?;
    let blue = tree.append(group, absolute([24.0, 0.0, 35.0, 48.0], BLUE))?;

    for scale in [1.0, 1.5, 2.0] {
        for shear in [0.0, 0.25] {
            let transform = Affine::new([scale, 0.0, shear * scale, scale, 0.0, 0.0]);
            let inverse = transform.inverse()?;
            let frame = tree.paint(VIEWPORT, scale)?;
            let mut scene = frame.scene;
            // Apply shear after UI painting to exercise the affine fallback
            // with the same style normalization and clip/hit geometry.
            let root = scene.insert(
                None,
                0,
                NodeKind::Reference(frame.root, Affine::new([1.0, 0.0, shear, 1.0, 0.0, 0.0])),
            )?;
            let cpu = CpuRenderer.render(&scene, root, pixel_size(scale))?;
            let image = gpu.render(&scene, root, pixel_size(scale))?;
            for (x, y, expected, hit, sample) in [
                (8.0, 29.0, CANVAS, None, "large arc below midpoint"),
                (9.0, 39.0, BORDER, Some(panel), "straight border"),
                (16.0, 22.0, BORDER, Some(panel), "rounded padding clip"),
                (22.0, 35.0, [120, 60, 70, 255], Some(red), "isolated red"),
                (42.0, 29.0, [30, 60, 160, 255], Some(blue), "overlap"),
                (57.0, 29.0, [30, 60, 160, 255], Some(blue), "isolated blue"),
            ] {
                let physical = transform.transform_point(Point::new(x, y));
                let label = format!("{sample} at {scale}x, shear={shear}");
                assert_sample([&cpu, &image], physical, expected, &label);
                let pointer = inverse.transform_point(Point::new(
                    physical.x().floor() + 0.5,
                    physical.y().floor() + 0.5,
                ));
                assert_eq!(tree.hit_test(pointer)?, hit, "{label}");
            }
        }
    }
    Ok(())
}

#[test]
fn direct_scene_draws_and_clips_match_under_similarity_and_shear() -> TestResult {
    let gpu = RenderContext::new()?;
    // Scene accepts reversed edges and signed radii. The CPU adapter uses
    // their absolute geometry and retains its half-side clamp on overlap.
    let rect = Rect::new(72.0, 48.0, 8.0, 8.0);
    for scale in [1.0, 1.5, 2.0] {
        for shear in [0.0, 0.25] {
            let transform = Affine::new([scale, 0.0, shear * scale, scale, 0.0, 0.0]);
            for clip in [false, true] {
                for (radii, cutout) in [
                    (CornerRadii::new(-32.0, 16.0, 24.0, 4.0), TRANSPARENT),
                    (CornerRadii::new(-60.0, 16.0, 24.0, 4.0), RED),
                ] {
                    let mut scene = Scene::new();
                    let root = scene.insert(None, 0, NodeKind::Transform(transform))?;
                    let (parent, draw_rect, draw_radii) = if clip {
                        let parent = scene.insert(
                            Some(root),
                            0,
                            NodeKind::Clip(Clip::RoundedRect(rect, radii)),
                        )?;
                        (
                            parent,
                            Rect::new(0.0, 0.0, 80.0, 56.0),
                            CornerRadii::default(),
                        )
                    } else {
                        (root, rect, radii)
                    };
                    scene.insert(
                        Some(parent),
                        0,
                        NodeKind::Draw(Primitive::Fill(
                            draw_rect,
                            draw_radii,
                            Brush::Solid(color(RED)),
                        )),
                    )?;
                    let cpu = CpuRenderer.render(&scene, root, pixel_size(scale))?;
                    let image = gpu.render(&scene, root, pixel_size(scale))?;
                    for (x, y, expected) in
                        [(10.0, 24.0, cutout), (9.0, 39.0, RED), (27.0, 18.0, RED)]
                    {
                        let physical = transform.transform_point(Point::new(x, y));
                        let label =
                            format!("clip={clip}, shear={shear}, scale={scale}, radii={radii:?}");
                        assert_sample([&cpu, &image], physical, expected, &label);
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn asymmetric_stroke_inset_can_trim_an_arc_against_the_opposite_edge() -> TestResult {
    let gpu = RenderContext::new()?;
    for scale in [1.0, 1.5, 2.0] {
        for shear in [0.0, 0.25] {
            let transform = Affine::new([scale, 0.0, shear * scale, scale, 0.0, 0.0]);
            let mut scene = Scene::new();
            let root = scene.insert(None, 0, NodeKind::Transform(transform))?;
            scene.insert(
                Some(root),
                0,
                NodeKind::Draw(Primitive::Stroke(
                    Rect::new(8.0, 8.0, 72.0, 48.0),
                    CornerRadii::new(40.0, 0.0, 0.0, 0.0),
                    12.0,
                    Brush::Solid(color(BORDER)),
                )),
            )?;
            let cpu = CpuRenderer.render(&scene, root, pixel_size(scale))?;
            let image = gpu.render(&scene, root, pixel_size(scale))?;
            for (x, y, expected) in [
                (15.0, 30.0, BORDER),
                (17.0, 39.0, TRANSPARENT),
                (24.0, 46.0, BORDER),
                (24.0, 36.0, TRANSPARENT),
            ] {
                let physical = transform.transform_point(Point::new(x, y));
                let label = format!("stroke with shear={shear} at {scale}x");
                assert_sample([&cpu, &image], physical, expected, &label);
            }
        }
    }
    Ok(())
}
