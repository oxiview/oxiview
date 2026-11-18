//! Font shaping, fallback, rasterization, and cache contracts using licensed fixtures.

use oxiview_base::{Color, Point};
use oxiview_text::{Error, GlyphImage, TextStyle, TextSystem};

fn fonts() -> Result<TextSystem, Error> {
    let mut system = TextSystem::without_system_fonts();
    system.register_font(include_bytes!("fonts/NotoSans.ttf").to_vec())?;
    system.register_font(include_bytes!("fonts/NotoSansArabic.ttf").to_vec())?;
    Ok(system)
}
fn style() -> TextStyle {
    TextStyle {
        font_size: 24.0,
        family: Some("Noto Sans, Noto Sans Arabic".into()),
        color: Color::from_rgba8(255, 180, 90, 200),
        ..TextStyle::default()
    }
}
fn pixel_bytes(glyphs: &[GlyphImage]) -> usize {
    glyphs.iter().map(|glyph| glyph.image.pixels().len()).sum()
}

#[test]
fn unicode_shaping_fallback_and_wrapping_use_registered_fonts() -> Result<(), Error> {
    let mut system = fonts()?;
    let text = "Office café e\u{301} — العربية مرحبا";
    let layout = system.layout(text, &style(), Some(160.0))?;
    assert_eq!(layout.missing_glyph_count(), 0);
    assert!(layout.line_count() >= 2);
    assert!(layout.size().height() > 30.0);
    assert!(layout.size().width() <= 161.0);
    let composed = system.layout("e\u{301}", &style(), None)?;
    assert_eq!(composed.glyph_count(), 1);
    let ligature = system.layout("ffi", &style(), None)?;
    assert!(ligature.glyph_count() < 3);
    let images = system.glyph_images(&layout, Point::new(0.0, 0.0), 2.0)?;
    assert!(!images.is_empty());
    assert!(images.iter().any(|glyph| {
        glyph
            .image
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel[3] > 0)
    }));
    Ok(())
}

#[test]
fn dpi_rasterization_retains_logical_layout_and_reuses_bounded_cache() -> Result<(), Error> {
    let mut system = fonts()?;
    let layout = system.layout("SNAKE 012345", &style(), None)?;
    let dimensions = layout.size();
    let one = system.glyph_images(&layout, Point::new(0.0, 0.0), 1.0)?;
    let first = system.cache_stats();
    let reused = system.glyph_images(&layout, Point::new(0.0, 0.0), 1.0)?;
    assert_eq!(one, reused);
    assert!(system.cache_stats().hits > first.hits);
    assert_eq!(system.cache_stats().misses, first.misses);
    let two = system.glyph_images(&layout, Point::new(0.0, 0.0), 2.0)?;
    assert_eq!(layout.size(), dimensions);
    assert!(pixel_bytes(&two) > pixel_bytes(&one) * 2);
    system.set_cache_budget(64);
    assert!(system.cache_stats().bytes <= 64);
    assert!(
        pixel_bytes(&one) > 64,
        "existing scene glyph images remain owned after cache eviction"
    );
    let rebuilt = system.glyph_images(&layout, Point::new(0.0, 0.0), 1.0)?;
    assert_eq!(one, rebuilt);
    assert!(system.cache_stats().bytes <= 64);
    Ok(())
}

#[test]
fn variable_weight_changes_glyphs_and_invalid_inputs_return_errors() -> Result<(), Error> {
    let mut system = fonts()?;
    let normal = system.layout("Readable", &style(), None)?;
    let bold_style = TextStyle {
        weight: 800,
        ..style()
    };
    let bold = system.layout("Readable", &bold_style, None)?;
    let normal_images = system.glyph_images(&normal, Point::new(0.0, 0.0), 1.0)?;
    let bold_images = system.glyph_images(&bold, Point::new(0.0, 0.0), 1.0)?;
    assert_ne!(normal_images, bold_images);
    let invalid = TextStyle {
        font_size: f64::NAN,
        ..style()
    };
    assert!(matches!(
        system.layout("x", &invalid, None),
        Err(Error::InvalidInput)
    ));
    assert!(matches!(
        system.layout("x", &style(), Some(f64::INFINITY)),
        Err(Error::InvalidInput)
    ));
    assert!(matches!(
        system.glyph_images(&normal, Point::new(0.0, 0.0), 0.0),
        Err(Error::InvalidInput)
    ));
    assert_eq!(system.register_font(vec![1, 2, 3]), Err(Error::InvalidFont));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn system_font_fallback_renders_chinese_and_color_emoji_without_a_window() -> Result<(), Error> {
    let mut system = TextSystem::new();
    let layout = system.layout(
        "贪吃蛇 Snake 🐍",
        &TextStyle {
            font_size: 24.0,
            ..TextStyle::default()
        },
        None,
    )?;
    assert_eq!(layout.missing_glyph_count(), 0);
    assert!(pixel_bytes(&system.glyph_images(&layout, Point::new(0.0, 0.0), 2.0)?) > 0);
    Ok(())
}

#[test]
fn whitespace_keeps_advance_without_transparent_draws_at_fractional_phases() -> Result<(), Error> {
    let mut system = fonts()?;
    let layout = system.layout(" ", &style(), None)?;
    let logical_size = layout.size();
    assert_eq!(layout.glyph_count(), 1);
    assert!(logical_size.width() > 0.0);
    for scale in [1.0, 1.5, 2.0] {
        for phase in [0.0, 0.25, 0.5, 0.75] {
            assert!(
                system
                    .glyph_images(&layout, Point::new(phase, phase), scale)?
                    .is_empty()
            );
            assert_eq!(layout.glyph_count(), 1);
            assert_eq!(layout.size(), logical_size);
        }
    }
    Ok(())
}
