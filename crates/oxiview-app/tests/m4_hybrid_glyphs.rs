//! Hybrid glyphs through text layout, Scene encoding and offscreen rendering.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use oxiview_base::{Affine, Color, GlyphKind, PixelSize, Point, RgbaImage, Size};
use oxiview_core::{UiTree, text};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_render_wgpu::RenderContext;
use oxiview_scene::{EncodedNode, NodeKind, Primitive, Scene};
use oxiview_style::{Length, Position, Style};
use oxiview_text::{GlyphDraw, GlyphImage, TextStyle, TextSystem};
use std::{cell::RefCell, rc::Rc};

const FIXTURE: &[u8] = include_bytes!("../../oxiview-text/tests/fonts/OxiviewColorFixture.ttf");

fn sample(image: &RgbaImage, baseline: Point, em: f64, x: f64, y: f64) -> [u8; 4] {
    pixel_at(
        image,
        Point::new(baseline.x() + x * em, baseline.y() - y * em),
    )
}

fn pixel_at(image: &RgbaImage, point: Point) -> [u8; 4] {
    let x = point.x().floor() as usize;
    let y = point.y().floor() as usize;
    let start = (y * image.width() as usize + x) * 4;
    image.pixels()[start..start + 4].try_into().unwrap()
}

#[test]
fn real_tree_paints_colr_gradients_transforms_and_multiply_at_both_scales() {
    let mut system = TextSystem::without_system_fonts();
    system.register_font(FIXTURE.to_vec()).unwrap();
    let system = Rc::new(RefCell::new(system));
    let mut tree = UiTree::new().unwrap();
    let mut placement = Style::default();
    placement.layout.position = Position::Absolute;
    placement.layout.inset.left = Length::Points(12.25);
    placement.layout.inset.top = Length::Points(9.125);
    placement.layout.padding.left = Length::Points(3.0);
    placement.layout.padding.top = Length::Points(4.0);
    let text_style = TextStyle {
        family: Some("Oxiview Color Fixture".into()),
        font_size: 48.0,
        color: Color::from_rgba8(120, 35, 210, 128),
        ..TextStyle::default()
    };
    tree.mount(
        tree.root(),
        text(system, "LMSRFV", text_style).style(placement),
    )
    .unwrap();
    for dpi in [1.0, 2.0] {
        let scene = tree.paint(Size::new(360.0, 100.0), dpi).unwrap();
        let encoded = scene.scene.encode(scene.root).unwrap();
        let glyphs: Vec<_> = encoded
            .nodes
            .iter()
            .filter_map(|node| match node {
                EncodedNode::Draw {
                    primitive: Primitive::Glyph(glyph),
                    transform,
                    ..
                } => {
                    assert_eq!(glyph.kind, GlyphKind::Color);
                    Some(transform.transform_point(glyph.origin))
                }
                _ => None,
            })
            .collect();
        assert_eq!(glyphs.len(), 6);
        assert_eq!(glyphs[0].x(), (12.25 + 3.0) * dpi);
        let image = CpuRenderer
            .render(
                &scene.scene,
                scene.root,
                PixelSize {
                    width: (360.0 * dpi) as u32,
                    height: (100.0 * dpi) as u32,
                },
            )
            .unwrap();
        let at = |glyph, x, y| sample(&image, glyphs[glyph], 48.0 * dpi, x, y);

        let gradient_low = at(0, 0.4, 0.10);
        let gradient_high = at(0, 0.4, 0.72);
        assert!(gradient_low[0] > gradient_low[2], "{gradient_low:?}");
        assert!(gradient_high[2] > gradient_high[0], "{gradient_high:?}");
        assert_eq!(gradient_low[3], 128);
        assert_eq!(at(1, 0.10, 0.40), [0, 255, 255, 128]);
        assert_eq!(at(1, 0.40, 0.40), [0, 255, 0, 128]);
        assert_eq!(at(1, 0.70, 0.40), [255, 255, 0, 128]);
        for (x, y, expected) in [
            (0.6, 0.6, [255, 0, 0, 128]),
            (0.2, 0.6, [0, 255, 0, 128]),
            (0.2, 0.2, [0, 0, 255, 128]),
            (0.6, 0.2, [255, 255, 0, 128]),
        ] {
            assert_eq!(at(2, x, y), expected, "sweep sample {x}, {y}, DPI {dpi}");
        }
        let radial_center = at(3, 0.4, 0.4);
        let radial_edge = at(3, 0.08, 0.08);
        assert!(radial_center[0] > radial_center[2], "{radial_center:?}");
        assert!(radial_edge[2] > radial_edge[0], "{radial_edge:?}");
        for (actual, expected) in at(4, 0.4, 0.4).into_iter().zip([120_u8, 35, 210, 128]) {
            assert!(actual.abs_diff(expected) <= 1);
        }
        assert_eq!(at(5, 0.4, 0.4), [0, 255, 0, 128]);
    }
}

#[test]
fn real_tree_preserves_the_embedded_color_bitmap_above_the_vector_threshold() {
    let mut system = TextSystem::without_system_fonts()
        .with_vector_threshold(0.0)
        .unwrap();
    system.register_font(FIXTURE.to_vec()).unwrap();
    let mut tree = UiTree::new().unwrap();
    tree.mount(
        tree.root(),
        text(
            Rc::new(RefCell::new(system)),
            "B",
            TextStyle {
                family: Some("Oxiview Color Fixture".into()),
                font_size: 32.0,
                ..TextStyle::default()
            },
        ),
    )
    .unwrap();
    for dpi in [1.0, 2.0] {
        let scene = tree.paint(Size::new(80.0, 80.0), dpi).unwrap();
        let encoded = scene.scene.encode(scene.root).unwrap();
        assert!(encoded.nodes.iter().any(|node| matches!(
            node,
            EncodedNode::Draw { primitive: Primitive::Image(_, image), .. }
                if image.width() == 4 && image.height() == 4
        )));
        let image = CpuRenderer
            .render(
                &scene.scene,
                scene.root,
                PixelSize {
                    width: (80.0 * dpi) as u32,
                    height: (80.0 * dpi) as u32,
                },
            )
            .unwrap();
        assert!(
            image
                .pixels()
                .chunks_exact(4)
                .any(|pixel| pixel == [240, 40, 180, 255])
        );
    }
}

#[test]
fn bitmap_only_text_paints_both_strikes_and_keeps_space_advance_in_public_scenes()
-> Result<(), Box<dyn std::error::Error>> {
    let mut system = TextSystem::without_system_fonts();
    system.register_font(
        include_bytes!("../../oxiview-text/tests/fonts/OxiviewBitmapFixture.ttf").to_vec(),
    )?;
    let em = 24.0;
    let text_style = TextStyle {
        family: Some("Oxiview Bitmap Fixture".into()),
        font_size: em,
        color: Color::from_rgba8(20, 160, 40, 255),
        ..TextStyle::default()
    };
    let layout = system.layout("B M B", &text_style, None)?;
    assert_eq!(layout.missing_glyph_count(), 0);
    assert_eq!(layout.glyph_count(), 5);
    assert_eq!(layout.size().width(), 5.0 * em);
    let gpu = RenderContext::new()?;
    let origin = Point::new(3.25, 4.5);
    for transform in [
        Affine::IDENTITY,
        Affine::new([3.0, 0.0, 0.0, 3.0, 0.0, 0.0]),
        Affine::new([0.0, 1.0, -1.0, 0.0, 60.0, 0.0]),
    ] {
        let draws = system.glyph_draws(&layout, origin, transform)?;
        let [
            GlyphDraw::Image(first),
            GlyphDraw::Image(mono),
            GlyphDraw::Image(last),
        ] = draws.as_slice()
        else {
            return Err(format!("expected three bitmap draws, got {draws:?}").into());
        };
        assert_eq!(mono.rect.origin().x() - first.rect.origin().x(), 2.0 * em);
        assert_eq!(last.rect.origin().x() - mono.rect.origin().x(), 2.0 * em);
        assert_eq!(first.image.width(), 4);
        assert_eq!(mono.image.width(), 4);

        let mut scene = Scene::new();
        let root = scene.insert(None, 0, NodeKind::Transform(transform))?;
        for (index, glyph) in [first, mono, last].into_iter().enumerate() {
            scene.insert(
                Some(root),
                index,
                NodeKind::Draw(Primitive::Image(glyph.rect, glyph.image.clone())),
            )?;
        }
        let size = PixelSize {
            width: 336,
            height: 128,
        };
        let cpu_image = CpuRenderer.render(&scene, root, size)?;
        let gpu_image = gpu.render(&scene, root, size)?;
        for image in [cpu_image, gpu_image] {
            let at = |point| pixel_at(&image, transform.transform_point(point));
            let ink = |glyph: &GlyphImage| {
                at(Point::new(
                    glyph.rect.origin().x() + 3.0,
                    glyph.rect.origin().y() + 3.0,
                ))
            };
            assert_eq!(ink(first), [240, 40, 180, 255]);
            assert_eq!(ink(mono), [20, 160, 40, 255]);
            assert_eq!(ink(last), [240, 40, 180, 255]);
            assert_eq!(
                at(Point::new(
                    first.rect.origin().x() + em,
                    first.rect.origin().y() + 3.0
                ))[3],
                0
            );
        }
    }
    Ok(())
}
