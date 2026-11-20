use super::*;
use oxiview_base::{Color, GlyphKind, Point};

const FIXTURE: &[u8] = include_bytes!("../../../oxiview-text/tests/fonts/OxiviewColorFixture.ttf");

fn variable_glyph(bytes: Arc<[u8]>) -> Glyph {
    Glyph {
        font: GlyphFont::new(bytes),
        face_index: 0,
        glyph_id: 17,
        size: 100.0,
        origin: Point::new(0.0, 100.0),
        color: Color::from_rgba8(0, 0, 0, 255),
        variations: Arc::from([0]),
        kind: GlyphKind::Color,
    }
}

#[test]
fn normalized_font_cache_limits_retained_bytes_and_releases_source() -> Result<(), Error> {
    let mut bytes = FIXTURE.to_vec();
    bytes.resize(10 * 1024 * 1024, 0);
    let bytes: Arc<[u8]> = bytes.into();
    let source = Arc::downgrade(&bytes);
    let mut glyph = variable_glyph(bytes);
    let mut painter = GlyphPainter::default();
    let (normal, cached) = painter.color_font(&glyph, &coordinates(&glyph))?;
    assert!(cached);
    let (again, cached) = painter.color_font(&glyph, &coordinates(&glyph))?;
    assert!(cached);
    assert_eq!(normal.id(), again.id());
    assert!(painter.font_bytes >= 2 * glyph.font.bytes().len());

    // A different location needs a new copy. The byte budget is reached
    // long before the entry limit. Keep the active result after the eviction.
    glyph.variations = Arc::from([16384]);
    let (heavy, cached) = painter.color_font(&glyph, &coordinates(&glyph))?;
    assert!(cached);
    assert_ne!(heavy.id(), normal.id());
    assert_eq!(painter.fonts.len(), 1);
    assert!(painter.font_bytes <= MAX_NORMALIZED_FONT_BYTES);

    drop((glyph, normal, again, heavy));
    assert!(
        source.upgrade().is_some(),
        "the key's source must stay alive"
    );
    let mut other = variable_glyph(FIXTURE.into());
    let mut bytes = FIXTURE.to_vec();
    bytes.resize(10 * 1024 * 1024, 0);
    other.font = GlyphFont::new(bytes.into());
    painter.color_font(&other, &coordinates(&other))?;
    assert!(source.upgrade().is_none());
    Ok(())
}

#[test]
fn normalized_font_cache_limits_entries() -> Result<(), Error> {
    let mut glyph = variable_glyph(FIXTURE.into());
    let mut painter = GlyphPainter::default();
    for location in 0..MAX_NORMALIZED_FONTS {
        glyph.variations = Arc::from([location as i16]);
        painter.color_font(&glyph, &coordinates(&glyph))?;
    }
    assert_eq!(painter.fonts.len(), MAX_NORMALIZED_FONTS);
    glyph.variations = Arc::from([16384]);
    let (_, cached) = painter.color_font(&glyph, &coordinates(&glyph))?;
    assert!(cached);
    assert_eq!(painter.fonts.len(), 1);
    Ok(())
}

#[test]
fn oversized_normalized_font_is_transient() -> Result<(), Error> {
    let mut bytes = FIXTURE.to_vec();
    bytes.resize(MAX_NORMALIZED_FONT_BYTES / 2 + 1, 0);
    let bytes: Arc<[u8]> = bytes.into();
    let source = Arc::downgrade(&bytes);
    let glyph = variable_glyph(bytes);
    let mut painter = GlyphPainter::default();
    let (font, cached) = painter.color_font(&glyph, &coordinates(&glyph))?;
    assert!(!cached);
    assert_eq!(font.as_ref().len(), glyph.font.bytes().len());
    assert_ne!(font.as_ref(), glyph.font.bytes());
    assert!(painter.fonts.is_empty());
    assert_eq!(painter.font_bytes, 0);
    drop(glyph);
    assert!(source.upgrade().is_some());
    drop(font);
    assert!(source.upgrade().is_none());
    Ok(())
}
