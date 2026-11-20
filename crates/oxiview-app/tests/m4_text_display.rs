//! Text display through the public view API, retained layout and Scene adapters.

use oxiview_app::{RichText, TextSpan, TextSpanStyle, TextStyle, TextSystem, rich_text, text};
use oxiview_base::{Color, GlyphKind, Size};
use oxiview_core::{PaintedScene, UiTree, node};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_scene::{EncodedNode, PixelSize, Primitive};
use oxiview_style::{Alignment, Edges, FlexDirection, Length, Style};
use oxiview_testing::{CompareConfig, SnapshotHarness};
use std::{cell::RefCell, error::Error, path::Path, rc::Rc};

const WIDTH: u32 = 288;
const HEIGHT: u32 = 132;
const BODY: &str = "Office café · العربية";
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn style(size: f64, family: &str) -> TextStyle {
    TextStyle {
        font_size: size,
        family: Some(family.into()),
        color: Color::from_rgba8(26, 37, 50, 255),
        ..TextStyle::default()
    }
}

fn specimen(scale: f64) -> TestResult<(PaintedScene, PixelSize)> {
    let mut fonts = TextSystem::without_system_fonts().with_vector_threshold(48.0)?;
    for bytes in [
        include_bytes!("../../oxiview-text/tests/fonts/NotoSans.ttf").as_slice(),
        include_bytes!("../../oxiview-text/tests/fonts/NotoSansArabic.ttf").as_slice(),
        include_bytes!("../../oxiview-text/tests/fonts/OxiviewColorFixture.ttf").as_slice(),
    ] {
        fonts.register_font(bytes.to_vec())?;
    }
    let system = Rc::new(RefCell::new(fonts));
    let mut tree = UiTree::new()?;
    let root = tree.root();
    let mut page = Style {
        background: Color::from_rgba8(247, 246, 241, 255),
        ..Style::default()
    };
    page.layout.flex_direction = FlexDirection::Column;
    page.layout.align_items = Alignment::Start;
    page.layout.padding = Edges::all(Length::Points(10.0));
    tree.set_style(root, page)?;
    let mut row = Style::default();
    row.layout.align_items = Alignment::Baseline;
    row.layout.column_gap = Length::Points(9.0);
    tree.mount(
        root,
        node()
            .style(row)
            .child(text(Rc::clone(&system), "42", style(68.0, "Noto Sans")))
            .child(text(Rc::clone(&system), "points", style(16.0, "Noto Sans")))
            .child(text(
                Rc::clone(&system),
                "MS",
                style(32.0, "Oxiview Color Fixture"),
            )),
    )?;
    let source = RichText::new(
        BODY,
        vec![
            TextSpan {
                range: 0.."Office".len(),
                style: TextSpanStyle {
                    weight: Some(780),
                    ..Default::default()
                },
            },
            TextSpan {
                range: "Office café · ".len()..BODY.len(),
                style: TextSpanStyle {
                    color: Some(Color::from_rgba8(34, 103, 117, 255)),
                    ..Default::default()
                },
            },
        ],
    )?;
    tree.mount(
        root,
        rich_text(system, source, style(18.0, "Noto Sans, Noto Sans Arabic")),
    )?;
    let paint = tree.paint(Size::new(f64::from(WIDTH), f64::from(HEIGHT)), scale)?;
    let encoded = paint.scene.encode(paint.root)?;
    let mut images = 0;
    let mut outlines = 0;
    let mut colors = 0;
    for operation in &encoded.nodes {
        if let EncodedNode::Draw { primitive, .. } = operation {
            match primitive {
                Primitive::Image(..) => images += 1,
                Primitive::Glyph(glyph) => match glyph.kind {
                    GlyphKind::Outline => outlines += 1,
                    GlyphKind::Color => colors += 1,
                },
                _ => {}
            }
        }
    }
    assert!(images > 5, "hinted body text");
    assert_eq!(outlines, 2, "large outline digits");
    assert_eq!(colors, 2, "multiply and sweep COLRv1 glyphs");
    let size = PixelSize {
        width: (f64::from(WIDTH) * scale) as u32,
        height: (f64::from(HEIGHT) * scale) as u32,
    };
    Ok((paint, size))
}

fn snapshot(name: &str, scale: f64) -> TestResult {
    let (paint, size) = specimen(scale)?;
    let pixels = CpuRenderer.render(&paint.scene, paint.root, size)?;
    let actual =
        oxiview_testing::RgbaImage::new(size.width, size.height, pixels.pixels().to_vec())?;
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    SnapshotHarness::new(
        &crate_root.join("../..").canonicalize()?,
        crate_root,
        env!("CARGO_PKG_NAME"),
    )?
    .assert_snapshot(name, &actual, &CompareConfig::default())?;
    Ok(())
}

#[test]
fn text_at_1x() -> TestResult {
    snapshot("m4-text-1x", 1.0)
}

#[test]
fn text_at_fractional_scale() -> TestResult {
    snapshot("m4-text-1_5x", 1.5)
}

#[test]
fn text_at_2x() -> TestResult {
    snapshot("m4-text-2x", 2.0)
}
