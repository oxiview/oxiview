//! Headless pixel checks for public Scene paths and hybrid rich text.

use oxiview_app::{RichText, TextSpan, TextSpanStyle, TextStyle, TextSystem, rich_text};
use oxiview_base::{Affine, Color, Point, Rect, Size};
use oxiview_core::{PaintedScene, UiTree, node};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_render_wgpu::RenderContext;
use oxiview_scene::{
    BezierPath, Brush, Clip, ColorStop, EncodedNode, FillRule, NodeKind, PathCommand::*, PixelSize,
    Primitive, RgbaImage, Scene,
};
use oxiview_style::{Alignment, Length, Style};
use std::{cell::RefCell, error::Error, rc::Rc};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn sample(image: &RgbaImage, point: Point) -> &[u8] {
    let index = (point.y() as usize * image.width() as usize + point.x() as usize) * 4;
    &image.pixels()[index..index + 4]
}

#[test]
fn public_scene_paths_fill_curves_and_preserve_winding_clip_and_opacity() -> TestResult {
    let gpu = RenderContext::new()?;
    let p = Point::new;
    // Both contours wind clockwise. The inner contour is a hole under even-odd.
    let outline = BezierPath::new(vec![
        MoveTo(p(4.0, 20.0)),
        QuadTo(p(4.0, 4.0), p(20.0, 4.0)),
        CubicTo(p(38.0, 4.0), p(44.0, 9.0), p(44.0, 20.0)),
        LineTo(p(44.0, 44.0)),
        LineTo(p(4.0, 44.0)),
        Close,
        MoveTo(p(14.0, 16.0)),
        LineTo(p(28.0, 16.0)),
        LineTo(p(28.0, 32.0)),
        LineTo(p(14.0, 32.0)),
        Close,
    ])?;
    let gradient = Brush::LinearGradient(
        p(9.0, 0.0),
        p(39.0, 0.0),
        vec![
            ColorStop {
                position: 0.0,
                color: Color::from_rgba8(255, 0, 0, 255),
            },
            ColorStop {
                position: 1.0,
                color: Color::from_rgba8(0, 0, 255, 255),
            },
        ],
    );
    let solid = BezierPath::new(vec![
        MoveTo(p(30.0, 22.0)),
        LineTo(p(37.0, 22.0)),
        LineTo(p(37.0, 27.0)),
        LineTo(p(30.0, 27.0)),
    ])?;

    for rule in [FillRule::EvenOdd, FillRule::NonZero] {
        let mut scene = Scene::new();
        let definition = scene.insert(None, 0, NodeKind::Opacity(0.5))?;
        let clip = scene.insert(
            Some(definition),
            0,
            NodeKind::Clip(Clip::Rect(Rect::new(7.0, 6.0, 42.0, 37.0))),
        )?;
        scene.insert(
            Some(clip),
            0,
            NodeKind::Draw(Primitive::FillPath(outline.clone(), rule, gradient.clone())),
        )?;
        scene.insert(
            Some(clip),
            1,
            NodeKind::Draw(Primitive::FillPath(
                solid.clone(),
                FillRule::NonZero,
                Brush::Solid(Color::from_rgba8(255, 255, 255, 255)),
            )),
        )?;
        for placement in [
            Affine::IDENTITY,
            Affine::new([-1.5, 0.25, 0.25, 1.5, 80.375, 7.625]),
        ] {
            let root = scene.insert(None, 1, NodeKind::Reference(definition, placement))?;
            let size = PixelSize {
                width: 96,
                height: 86,
            };
            let images = [
                CpuRenderer.render(&scene, root, size)?,
                gpu.render(&scene, root, size)?,
            ];
            for (adapter, image) in ["CPU", "GPU"].into_iter().zip(images) {
                let at = |x, y| sample(&image, placement.transform_point(p(x, y)));
                let quadratic = at(10.0, 10.0);
                let cubic = at(39.0, 12.0);
                // The curve samples are above the chords of their respective segments.
                assert!(
                    quadratic[0] > quadratic[2],
                    "{adapter} quadratic {quadratic:?}"
                );
                assert!(cubic[2] > cubic[0], "{adapter} cubic {cubic:?}");
                assert_eq!(at(43.0, 30.0), [0; 4], "{adapter} clip");
                assert_eq!(at(24.0, 39.0), [0; 4], "{adapter} clip");
                let hole = at(20.0, 24.0);
                if rule == FillRule::EvenOdd {
                    assert_eq!(hole, [0; 4], "{adapter} even-odd hole");
                } else {
                    assert!(
                        hole[0] > 0 && hole[2] > 0 && hole[3] == 128,
                        "{adapter} nonzero fill {hole:?}"
                    );
                }
                assert_eq!(
                    at(34.0, 24.0),
                    [255, 255, 255, 128],
                    "{adapter} isolated overlap"
                );
            }
            scene.remove(root)?;
        }
    }
    Ok(())
}

#[test]
fn out_of_range_path_geometry_returns_a_render_error() -> TestResult {
    let mut scene = Scene::new();
    let path = BezierPath::new(vec![
        MoveTo(Point::new(0.0, 0.0)),
        LineTo(Point::new(f64::MAX, 0.0)),
        LineTo(Point::new(1.0, 1.0)),
        Close,
    ])?;
    let root = scene.insert(
        None,
        0,
        NodeKind::Draw(Primitive::FillPath(
            path,
            FillRule::NonZero,
            Brush::Solid(Color::from_rgba8(255, 255, 255, 255)),
        )),
    )?;
    let size = PixelSize {
        width: 16,
        height: 16,
    };
    assert!(matches!(
        CpuRenderer.render(&scene, root, size),
        Err(oxiview_render_cpu::Error::SceneEncoding(_))
    ));
    assert!(matches!(
        RenderContext::new()?.render(&scene, root, size),
        Err(oxiview_render_wgpu::Error::SceneEncoding(_))
    ));
    Ok(())
}

fn paragraph(
    system: &Rc<RefCell<TextSystem>>,
    chars: &str,
    family: &str,
    span: TextSpanStyle,
    dpi: f64,
) -> TestResult<PaintedScene> {
    let mut tree = UiTree::new()?;
    let mut style = Style::default();
    style.layout.align_items = Alignment::Start;
    style.layout.padding.left = Length::Points(5.125);
    style.layout.padding.top = Length::Points(3.375);
    let paragraph = RichText::new(
        chars,
        vec![TextSpan {
            range: 0..chars.len(),
            style: span,
        }],
    )?;
    tree.mount(
        tree.root(),
        node().style(style).child(rich_text(
            Rc::clone(system),
            paragraph,
            TextStyle {
                family: Some(family.into()),
                font_size: 48.0,
                ..Default::default()
            },
        )),
    )?;
    Ok(tree.paint(Size::new(174.0, 74.0), dpi)?)
}

fn alpha(image: &RgbaImage) -> u64 {
    image
        .pixels()
        .chunks_exact(4)
        .map(|p| u64::from(p[3]))
        .sum()
}

#[test]
fn rich_italic_and_synthetic_bold_paint_as_vectors_at_fractional_dpi() -> TestResult {
    let gpu = RenderContext::new()?;
    let mut system = TextSystem::without_system_fonts();
    system.register_font(include_bytes!("../../oxiview-text/tests/fonts/NotoSans.ttf").to_vec())?;
    system.register_font(
        include_bytes!("../../oxiview-text/tests/fonts/OxiviewColorFixture.ttf").to_vec(),
    )?;
    let system = Rc::new(RefCell::new(system));
    let dpi = 2.25;
    for (chars, family, span, bold) in [
        (
            "O O",
            "Noto Sans",
            TextSpanStyle {
                italic: true,
                ..Default::default()
            },
            false,
        ),
        (
            "O O",
            "Oxiview Color Fixture",
            TextSpanStyle {
                weight: Some(800),
                ..Default::default()
            },
            true,
        ),
    ] {
        let reference = paragraph(&system, chars, family, TextSpanStyle::default(), dpi)?;
        let first_baseline = reference
            .scene
            .encode(reference.root)?
            .nodes
            .iter()
            .find_map(|node| match node {
                EncodedNode::Draw {
                    primitive: Primitive::Glyph(glyph),
                    transform,
                    ..
                } => Some(transform.transform_point(glyph.origin)),
                _ => None,
            })
            .ok_or("plain paragraph has no vector baseline")?;
        let styled = paragraph(&system, chars, family, span, dpi)?;
        let encoded = styled.scene.encode(styled.root)?;
        let glyphs: Vec<_> = encoded
            .nodes
            .iter()
            .filter_map(|node| match node {
                EncodedNode::Draw {
                    primitive: Primitive::FillPath(path, FillRule::NonZero, Brush::Solid(_)),
                    transform,
                    ..
                } => Some((path, *transform)),
                _ => None,
            })
            .collect();
        assert_eq!(glyphs.len(), 2, "{family} whitespace draws no path");
        assert_eq!(glyphs[0].1, glyphs[1].1);
        assert_eq!(&glyphs[0].1.coefficients()[..4], &[dpi, 0.0, 0.0, dpi]);
        assert!(!encoded.nodes.iter().any(|node| matches!(
            node,
            EncodedNode::Draw {
                primitive: Primitive::Image(..) | Primitive::Glyph(..),
                ..
            }
        )));

        let size = PixelSize {
            width: 420,
            height: 192,
        };
        for (adapter, plain, styled) in [
            (
                "CPU",
                CpuRenderer.render(&reference.scene, reference.root, size)?,
                CpuRenderer.render(&styled.scene, styled.root, size)?,
            ),
            (
                "GPU",
                gpu.render(&reference.scene, reference.root, size)?,
                gpu.render(&styled.scene, styled.root, size)?,
            ),
        ] {
            let changed = plain
                .pixels()
                .chunks_exact(4)
                .zip(styled.pixels().chunks_exact(4))
                .filter(|(before, after)| before[3].abs_diff(after[3]) > 128)
                .count();
            assert!(
                changed > 200,
                "{adapter} {family}: only {changed} changed pixels"
            );
            assert!(alpha(&styled) > 500_000, "{adapter} {family} painted ink");
            let at_em = |x: f64, y: f64| {
                sample(
                    &styled,
                    Point::new(
                        first_baseline.x() + x * 48.0 * dpi,
                        first_baseline.y() + y * 48.0 * dpi,
                    ),
                )
            };
            assert_eq!(
                at_em(0.9, -0.45),
                [0; 4],
                "{adapter} preserved space advance"
            );
            if bold {
                assert!(
                    alpha(&styled) > alpha(&plain),
                    "{adapter} synthetic bold expands the glyph"
                );
            } else {
                assert_eq!(
                    at_em(0.45, -0.35),
                    [0; 4],
                    "{adapter} preserved the counter"
                );
            }
        }
    }
    Ok(())
}
