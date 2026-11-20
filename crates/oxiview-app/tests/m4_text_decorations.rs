//! Headless rendering of wrapped, ranged text decorations through the retained view.

use oxiview_app::{RichText, TextSpan, TextSpanStyle, TextStyle, TextSystem, rich_text};
use oxiview_base::{Color, Point, Rect, RgbaImage, Size};
use oxiview_core::{PaintedScene, UiTree};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_render_wgpu::RenderContext;
use oxiview_scene::{Brush, EncodedNode, PixelSize, Primitive};
use oxiview_style::{Length, Position, Style};
use std::{cell::RefCell, error::Error, rc::Rc};

const LINE: &str = "سلام مرحبا";
const PARAGRAPH: &str = "سلام مرحبا سلام مرحبا";
const RED: [u8; 4] = [204, 29, 51, 255];
const BLUE: [u8; 4] = [24, 93, 202, 255];
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn content(color: [u8; 4], decorated: bool) -> TestResult<RichText> {
    let [r, g, b, a] = color;
    Ok(RichText::new(
        PARAGRAPH,
        vec![TextSpan {
            // The left-hand word of the first visual line and the right-hand word
            // of the second. The intervening wrap space hangs off the first line.
            range: "سلام ".len().."سلام مرحبا سلام".len(),
            style: TextSpanStyle {
                color: Some(Color::from_rgba8(r, g, b, a)),
                underline: decorated,
                strikethrough: decorated,
                ..Default::default()
            },
        }],
    )?)
}

fn encoded_rules(paint: &PaintedScene, color: [u8; 4], bitmap: bool) -> TestResult<Vec<Rect>> {
    let mut rules = Vec::new();
    let mut glyphs = 0;
    let [r, g, b, a] = color;
    let expected = Color::from_rgba8(r, g, b, a);
    for node in paint.scene.encode(paint.root)?.nodes {
        let EncodedNode::Draw {
            primitive,
            transform,
            ..
        } = node
        else {
            continue;
        };
        match primitive {
            Primitive::Fill(rect, _, Brush::Solid(brush)) if brush == expected => {
                assert_eq!(
                    glyphs == 0,
                    rules.len() < 2,
                    "underlines precede the glyphs"
                );
                let [x0, y0, x1, y1] = rect.edges();
                let p0 = transform.transform_point(Point::new(x0, y0));
                let p1 = transform.transform_point(Point::new(x1, y1));
                rules.push(Rect::new(p0.x(), p0.y(), p1.x(), p1.y()));
            }
            Primitive::Image(..) | Primitive::Glyph(_) => {
                assert_eq!(rules.len(), 2, "strikethroughs follow the glyphs");
                assert_eq!(matches!(primitive, Primitive::Image(..)), bitmap);
                glyphs += 1;
            }
            _ => {}
        }
    }
    assert!(glyphs > 0);
    assert_eq!(rules.len(), 4);
    Ok(rules)
}

fn assert_rule_pixels(rect: Rect, image: &RgbaImage, reference: &RgbaImage, color: [u8; 4]) {
    let [left, top, right, bottom] = rect.edges();
    let y = ((top + bottom) / 2.0).floor() as usize;
    let row = y * image.width() as usize;
    let changed = ((left.ceil() as usize + 2)..(right.floor() as usize - 2))
        .filter(|x| {
            let index = (row + x) * 4;
            image.pixels()[index..index + 4] == color
                && reference.pixels()[index..index + 4] != color
        })
        .count();
    assert!(
        changed > 5,
        "rule {rect:?} added only {changed} solid pixels"
    );
}

#[test]
fn wrapped_rtl_decorations_paint_at_fractional_dpi_and_update_without_relayout() -> TestResult {
    let gpu = RenderContext::new()?;
    let mut fonts = TextSystem::without_system_fonts();
    fonts.register_font(
        include_bytes!("../../oxiview-text/tests/fonts/NotoSansArabic.ttf").to_vec(),
    )?;
    let paragraph = TextStyle {
        family: Some("Noto Sans Arabic".into()),
        font_size: 40.0,
        ..Default::default()
    };
    let width = fonts.layout(LINE, &paragraph, None)?.size().width() + 1.0;
    let layout = fonts.layout(PARAGRAPH, &paragraph, Some(width))?;
    assert_eq!(layout.line_count(), 2);
    let first_baseline = layout.first_baseline().ok_or("missing baseline")?;
    let line_height = fonts.layout(LINE, &paragraph, None)?.size().height();
    let left_word = fonts.layout("مرحبا", &paragraph, None)?.size().width();
    let right_word = fonts.layout("سلام", &paragraph, None)?.size().width();
    let system = Rc::new(RefCell::new(fonts));
    let mut box_style = Style::default();
    box_style.layout.position = Position::Absolute;
    box_style.layout.inset.left = Length::Points(13.25);
    box_style.layout.inset.top = Length::Points(7.5);
    box_style.layout.padding.left = Length::Points(3.5);
    box_style.layout.padding.top = Length::Points(4.25);
    box_style.layout.width = Length::Points(width + 3.5);
    let mut tree = UiTree::new()?;
    let id = tree.mount(
        tree.root(),
        rich_text(system, content(RED, false)?, paragraph).style(box_style),
    )?;
    let viewport = Size::new(260.0, 132.0);
    for dpi in [1.0, 1.75] {
        tree.set_rich_text(id, content(RED, false)?)?;
        let before = tree.paint(viewport, dpi)?;
        let size = PixelSize {
            width: (viewport.width() * dpi).ceil() as u32,
            height: (viewport.height() * dpi).ceil() as u32,
        };
        let reference = CpuRenderer.render(&before.scene, before.root, size)?;
        let gpu_reference = gpu.render(&before.scene, before.root, size)?;
        let bounds = tree.bounds(id)?;
        let passes = tree.diagnostics().layout_passes;
        for color in [RED, BLUE] {
            let updated = content(color, true)?;
            tree.command_sender().set_rich_text(id, updated.clone())?;
            tree.flush_updates(viewport, |_, _, _| {})?;
            let paint = tree.paint(viewport, dpi)?;
            assert!(tree.contains(id));
            assert_eq!(tree.rich_text_source(id)?, updated);
            assert_eq!(tree.bounds(id)?, bounds);
            assert_eq!(tree.diagnostics().layout_passes, passes);
            let image = CpuRenderer.render(&paint.scene, paint.root, size)?;
            let gpu_image = gpu.render(&paint.scene, paint.root, size)?;
            let rules = encoded_rules(&paint, color, dpi == 1.0)?;
            let x = bounds.origin().x() + 3.5;
            let y = bounds.origin().y() + 4.25;
            for (index, rule) in rules.into_iter().enumerate() {
                let is_second_line = index % 2 == 1;
                let (left, word_width, baseline) = if is_second_line {
                    (
                        x + width - right_word,
                        right_word,
                        first_baseline + line_height,
                    )
                } else {
                    (x + 1.0, left_word, first_baseline)
                };
                // The Arabic fixture has -100/322-unit offsets and 50-unit
                // thickness at 1000 units per em.
                let offset = if index < 2 { 4.0 } else { -12.88 };
                let top = y + baseline + offset;
                for (actual, logical) in
                    rule.edges()
                        .into_iter()
                        .zip([left, top, left + word_width, top + 2.0])
                {
                    assert!(
                        (actual - logical * dpi).abs() < 0.01,
                        "{rule:?} edge {actual} at {dpi}x differs from {}",
                        logical * dpi
                    );
                }
                assert_rule_pixels(rule, &image, &reference, color);
                assert_rule_pixels(rule, &gpu_image, &gpu_reference, color);
            }
        }
    }
    Ok(())
}
