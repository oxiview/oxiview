//! M4 paragraph shaping and small-glyph preparation on pinned font fixtures.
#![allow(clippy::expect_used)] // Invalid fixtures must fail the benchmark.

use oxiview_base::{Affine, Color, Point};
use oxiview_text::{RichText, TextSpan, TextSpanStyle, TextStyle, TextSystem};

const PARAGRAPH: &str = "Office hours — café, e\u{301}lan, 09:45. العربية مرحبا. \
    A short paragraph wraps across several lines with mixed weights and scripts.";
const LABEL: &str = "Score:0123456789";
const WIDTH: Option<f64> = Some(188.0);

fn main() {
    divan::main();
}

fn fixture() -> (TextSystem, TextStyle) {
    let mut fonts = TextSystem::without_system_fonts();
    for bytes in [
        include_bytes!("../crates/oxiview-text/tests/fonts/NotoSans.ttf").as_slice(),
        include_bytes!("../crates/oxiview-text/tests/fonts/NotoSansArabic.ttf").as_slice(),
    ] {
        fonts
            .register_font(bytes.to_vec())
            .expect("register pinned font");
    }
    let style = TextStyle {
        font_size: 16.0,
        family: Some("Noto Sans, Noto Sans Arabic".into()),
        ..TextStyle::default()
    };
    (fonts, style)
}

#[divan::bench(sample_count = 80, sample_size = 64)]
fn shape_wrapped_paragraph(bencher: divan::Bencher) {
    let (mut fonts, style) = fixture();
    let layout = fonts
        .layout(PARAGRAPH, &style, WIDTH)
        .expect("shape paragraph");
    assert_eq!(layout.missing_glyph_count(), 0);
    assert!(layout.line_count() > 1);
    bencher.bench_local(|| {
        fonts
            .layout(divan::black_box(PARAGRAPH), &style, WIDTH)
            .expect("shape paragraph")
    });
}

#[divan::bench(sample_count = 80, sample_size = 64)]
fn shape_rich_paragraph(bencher: divan::Bencher) {
    let (mut fonts, style) = fixture();
    let spans = [
        (
            "Office hours",
            TextSpanStyle {
                weight: Some(720),
                ..Default::default()
            },
        ),
        (
            "العربية",
            TextSpanStyle {
                font_size: Some(20.0),
                ..Default::default()
            },
        ),
        (
            "mixed",
            TextSpanStyle {
                color: Some(Color::from_rgba8(25, 82, 120, 255)),
                ..Default::default()
            },
        ),
    ]
    .into_iter()
    .map(|(word, style)| {
        let start = PARAGRAPH.find(word).expect("span in fixture");
        TextSpan {
            range: start..start + word.len(),
            style,
        }
    })
    .collect();
    let source = RichText::new(PARAGRAPH, spans).expect("valid styled paragraph");
    let layout = fonts
        .layout_rich(&source, &style, WIDTH)
        .expect("shape rich paragraph");
    assert_eq!(layout.missing_glyph_count(), 0);
    assert!(layout.line_count() > 1);
    bencher.bench_local(|| {
        fonts
            .layout_rich(divan::black_box(&source), &style, WIDTH)
            .expect("shape rich paragraph")
    });
}

fn label_glyphs(bencher: divan::Bencher, cached: bool) {
    let (mut fonts, style) = fixture();
    let layout = fonts.layout(LABEL, &style, None).expect("shape label");
    if !cached {
        fonts.set_cache_budget(0);
    }
    let origin = Point::new(0.25, 0.5);
    let scale = Affine::new([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    let warmup = fonts
        .glyph_draws(&layout, origin, scale)
        .expect("prepare label");
    assert_eq!(warmup.len(), LABEL.len());
    let before = fonts.cache_stats();
    bencher.bench_local(|| {
        fonts
            .glyph_draws(divan::black_box(&layout), origin, scale)
            .expect("prepare small glyphs")
    });
    let after = fonts.cache_stats();
    if cached {
        assert!(after.hits > before.hits);
        assert_eq!(after.misses, before.misses);
    } else {
        assert!(after.misses > before.misses);
        assert_eq!(after.entries, 0);
    }
}

#[divan::bench(sample_count = 80, sample_size = 32)]
fn cold_label_glyphs_2x(bencher: divan::Bencher) {
    label_glyphs(bencher, false);
}

#[divan::bench(sample_count = 100, sample_size = 512)]
fn warm_label_glyphs_2x(bencher: divan::Bencher) {
    label_glyphs(bencher, true);
}
