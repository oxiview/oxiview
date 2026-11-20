//! Font shaping, fallback, rasterization, and cache contracts using licensed fixtures.

use oxiview_base::{Affine, Color, GlyphKind, PathCommand, Point};
use oxiview_text::{
    Error, GlyphDraw, GlyphImage, RichText, TextDecorationKind, TextLayout, TextSpan,
    TextSpanStyle, TextStyle, TextSystem,
};

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

#[test]
fn raster_cache_preserves_variations_after_transient_font_churn() -> Result<(), Error> {
    let mut system = fonts()?;
    let normal = system.layout("OO", &style(), None)?;
    let bold = system.layout(
        "OO",
        &TextStyle {
            weight: 800,
            ..style()
        },
        None,
    )?;
    let origin = Point::new(0.25, 0.75);
    let normal_images = system.glyph_images(&normal, origin, 1.0)?;
    let bold_images = system.glyph_images(&bold, origin, 1.0)?;
    assert_ne!(normal_images, bold_images);

    // Exceed Swash's scaler-cache capacity with independently owned blobs.
    let initial_misses = system.cache_stats().misses;
    for _ in 0..16 {
        let mut transient = TextSystem::without_system_fonts();
        transient.register_font(include_bytes!("fonts/OxiviewColorFixture.ttf").to_vec())?;
        let layout = transient.layout(
            "O",
            &TextStyle {
                family: Some("Oxiview Color Fixture".into()),
                ..style()
            },
            None,
        )?;
        let images = system.glyph_images(&layout, origin, 1.0)?;
        assert_eq!(images.len(), 1);
    }
    let misses = system.cache_stats().misses;
    assert_eq!(misses - initial_misses, 16);
    assert_eq!(system.glyph_images(&normal, origin, 1.0)?, normal_images);
    assert_eq!(system.glyph_images(&bold, origin, 1.0)?, bold_images);
    assert_eq!(system.cache_stats().misses, misses);

    system.set_cache_budget(0);
    assert_eq!(system.glyph_images(&bold, origin, 1.0)?, bold_images);
    assert_eq!(system.glyph_images(&normal, origin, 1.0)?, normal_images);
    assert_eq!(system.glyph_images(&bold, origin, 1.0)?, bold_images);
    assert!(system.cache_stats().misses > misses);
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
    let has_color_ink = |glyph: &GlyphImage| {
        glyph
            .image
            .pixels()
            .chunks_exact(4)
            .any(|pixel| matches!(pixel, [r, g, b, a] if *a > 0 && (r != g || g != b)))
    };
    assert!(
        system
            .glyph_images(&layout, Point::default(), 2.0)?
            .iter()
            .any(has_color_ink)
    );
    for transform in [
        Affine::new([4.0, 0.0, 0.0, 4.0, 0.0, 0.0]),
        Affine::new([0.0, 1.0, -1.0, 0.0, 80.0, 0.0]),
    ] {
        assert!(
            system
                .glyph_draws(&layout, Point::default(), transform)?
                .iter()
                .any(|draw| matches!(draw, GlyphDraw::Image(image) if has_color_ink(image)))
        );
    }
    Ok(())
}

#[test]
fn hybrid_bitmap_emoji_fallback_has_colored_ink() -> Result<(), Error> {
    let mut system = fonts()?;
    system.register_font(include_bytes!("fonts/OxiviewBitmapFixture.ttf").to_vec())?;
    let layout = system.layout(
        "A🐍",
        &TextStyle {
            family: Some("Noto Sans, Oxiview Bitmap Fixture".into()),
            ..style()
        },
        None,
    )?;
    assert_eq!(layout.missing_glyph_count(), 0);
    let transform = Affine::new([0.0, 4.0, -4.0, 0.0, 200.0, 0.0]);
    let draws = system.glyph_draws(&layout, Point::default(), transform)?;
    let [GlyphDraw::Vector(latin), GlyphDraw::Image(emoji)] = draws.as_slice() else {
        return Err(Error::InvalidGlyphImage);
    };
    assert_eq!(latin.kind, GlyphKind::Outline);
    assert_eq!(emoji.image.width(), 4);
    assert!(
        emoji
            .image
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == [240, 40, 180, 200])
    );
    Ok(())
}

#[test]
fn an_outline_free_face_without_strikes_is_invalid() -> Result<(), Error> {
    let mut bytes = include_bytes!("fonts/OxiviewBitmapFixture.ttf").to_vec();
    for tag in [b"sbix", b"EBLC"] {
        let offset = bytes
            .windows(4)
            .position(|bytes| bytes == tag)
            .ok_or(Error::InvalidFont)?;
        bytes[offset] = b'x';
    }
    let mut system = TextSystem::without_system_fonts().with_vector_threshold(0.0)?;
    system.register_font(bytes)?;
    let layout = system.layout(
        " ",
        &TextStyle {
            family: Some("Oxiview Bitmap Fixture".into()),
            ..style()
        },
        None,
    )?;
    assert_eq!(layout.glyph_count(), 1);
    assert_eq!(
        system.glyph_draws(&layout, Point::default(), Affine::IDENTITY),
        Err(Error::InvalidFont)
    );
    Ok(())
}

#[test]
fn a_corrupt_bitmap_is_a_raster_error() -> Result<(), Error> {
    let mut bytes = include_bytes!("fonts/OxiviewBitmapFixture.ttf").to_vec();
    let signature = bytes
        .windows(8)
        .position(|bytes| bytes == b"\x89PNG\r\n\x1a\n")
        .ok_or(Error::InvalidFont)?;
    bytes[signature] = 0;
    let mut system = TextSystem::without_system_fonts();
    system.register_font(bytes)?;
    let layout = system.layout(
        "B",
        &TextStyle {
            family: Some("Oxiview Bitmap Fixture".into()),
            ..style()
        },
        None,
    )?;
    assert_eq!(
        system.glyph_draws(&layout, Point::default(), Affine::IDENTITY),
        Err(Error::InvalidGlyphImage)
    );
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

#[test]
fn hybrid_selection_uses_device_size_and_affine_for_outlines() -> Result<(), Error> {
    let mut system = TextSystem::without_system_fonts().with_vector_threshold(36.0)?;
    let data = include_bytes!("fonts/NotoSans.ttf").to_vec();
    let registered_bytes = data.as_ptr();
    system.register_font(data)?;
    let layout = system.layout("OO", &style(), None)?;
    let origin = Point::new(3.125, 9.25);
    let simple = Affine::new([1.5, 0.0, 0.0, 1.5, 12.0, 24.0]);
    let raster = system.glyph_draws(&layout, origin, simple)?;
    assert_eq!(raster.len(), 2);
    assert!(
        raster
            .iter()
            .all(|draw| matches!(draw, GlyphDraw::Image(_)))
    );
    for transform in [
        Affine::new([1.51, 0.0, 0.0, 1.51, 12.0, 24.0]),
        Affine::new([0.0, 1.0, -1.0, 0.0, 12.0, 24.0]),
        Affine::new([1.0, 0.0, 0.5, 1.0, 12.0, 24.0]),
        Affine::new([1.0, 0.0, 0.0, 0.75, 12.0, 24.0]),
    ] {
        let glyphs = system.glyph_draws(&layout, origin, transform)?;
        let [GlyphDraw::Vector(first), GlyphDraw::Vector(second)] = glyphs.as_slice() else {
            return Err(Error::InvalidGlyphImage);
        };
        assert_eq!(first.font.bytes().as_ptr(), registered_bytes);
        assert_eq!(first.kind, GlyphKind::Outline);
        assert_eq!(first.size, 24.0);
        assert_eq!(first.origin.x(), origin.x());
        assert_eq!(first.origin.y(), second.origin.y());
        assert!(second.origin.x() > first.origin.x());
        assert_eq!(first.font.bytes().as_ptr(), second.font.bytes().as_ptr());
    }
    assert!(matches!(
        system.glyph_draws(
            &layout,
            origin,
            Affine::new([0.0, 1.0, -1.0, 0.0, f64::NAN, 0.0])
        ),
        Err(Error::InvalidInput)
    ));
    Ok(())
}

#[test]
fn oblique_paths_keep_fractional_baselines_rtl_advances_and_variations() -> Result<(), Error> {
    use skrifa::{
        FontRef, GlyphId, MetadataProvider,
        instance::{LocationRef, NormalizedCoord, Size},
        outline::{DrawSettings, OutlinePen},
        raw::TableProvider,
    };

    #[derive(Default)]
    struct ContourStarts(Vec<(f32, f32)>);
    impl OutlinePen for ContourStarts {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.push((x, y));
        }
        fn line_to(&mut self, _: f32, _: f32) {}
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn close(&mut self) {}
    }

    let mut system = fonts()?;
    let source = "O O بِبّ";
    let italic = RichText::new(
        source,
        vec![TextSpan {
            range: 0..source.len(),
            style: TextSpanStyle {
                italic: true,
                ..Default::default()
            },
        }],
    )?;
    let style = TextStyle {
        font_size: 23.5,
        weight: 725,
        ..style()
    };
    let plain = system.layout(source, &style, None)?;
    let styled = system.layout_rich(&italic, &style, None)?;
    assert_eq!(plain.size(), styled.size());
    assert_eq!(plain.glyph_count(), styled.glyph_count());
    assert_eq!(styled.missing_glyph_count(), 0);
    let origin = Point::new(3.125, 9.375);
    let sheared = Affine::new([1.0, 0.3, -0.45, 0.75, 4.25, 7.5]);
    let magnified = Affine::new([128.0, 0.0, 0.0, 128.0, -3.75, 12.125]);
    let plain = system.glyph_draws(&plain, origin, sheared)?;
    let paths = system.glyph_draws(&styled, origin, sheared)?;
    // The same local paths remain scalable at 3,008 device pixels per em.
    assert_eq!(paths, system.glyph_draws(&styled, origin, magnified)?);
    let hinted = system.glyph_draws(&styled, origin, Affine::IDENTITY)?;
    assert!(!hinted.is_empty());
    assert!(
        hinted
            .iter()
            .all(|draw| matches!(draw, GlyphDraw::Image(_)))
    );

    let mut styled = paths.iter();
    let mut empty = 0;
    let mut curved = false;
    for glyph in plain {
        let GlyphDraw::Vector(glyph) = glyph else {
            return Err(Error::InvalidGlyphOutline);
        };
        assert!(glyph.variations.iter().any(|&value| value != 0));
        let face = FontRef::from_index(glyph.font.bytes(), glyph.face_index)
            .map_err(|_| Error::InvalidFont)?;
        let units_per_em = face.head().map_err(|_| Error::InvalidFont)?.units_per_em();
        let mut starts = ContourStarts::default();
        let variations: Vec<_> = glyph
            .variations
            .iter()
            .copied()
            .map(NormalizedCoord::from_bits)
            .collect();
        face.outline_glyphs()
            .get(GlyphId::from(glyph.glyph_id))
            .ok_or(Error::InvalidGlyphOutline)?
            .draw(
                DrawSettings::unhinted(Size::unscaled(), LocationRef::new(&variations)),
                &mut starts,
            )
            .map_err(|_| Error::InvalidGlyphOutline)?;
        if starts.0.is_empty() {
            empty += 1;
            continue;
        }
        let Some(GlyphDraw::Path(path, color)) = styled.next() else {
            return Err(Error::InvalidGlyphOutline);
        };
        let contour_starts: Vec<_> = path
            .commands()
            .iter()
            .filter_map(|command| match command {
                PathCommand::MoveTo(point) => Some(*point),
                _ => None,
            })
            .collect();
        curved |= path
            .commands()
            .iter()
            .any(|command| matches!(command, PathCommand::QuadTo(..)));
        assert_eq!(*color, glyph.color);
        assert_eq!(contour_starts.len(), starts.0.len());
        assert_eq!(
            path.commands()
                .iter()
                .filter(|command| matches!(command, PathCommand::Close))
                .count(),
            starts.0.len()
        );
        let scale = f64::from(glyph.size) / f64::from(units_per_em);
        let skew = 14_f64.to_radians().tan();
        for (actual, (x, y)) in contour_starts.into_iter().zip(starts.0) {
            let x = glyph.origin.x() + (f64::from(x) + skew * f64::from(y)) * scale;
            let y = glyph.origin.y() - f64::from(y) * scale;
            assert!((actual.x() - x).abs() < 0.0001, "{actual:?} vs ({x}, {y})");
            assert!((actual.y() - y).abs() < 0.0001, "{actual:?} vs ({x}, {y})");
        }
    }
    assert_eq!(empty, 2);
    assert!(curved);
    assert!(styled.next().is_none());
    Ok(())
}

#[test]
fn colr_and_embedded_bitmap_keep_color_above_and_below_the_threshold() -> Result<(), Error> {
    let mut system = TextSystem::without_system_fonts();
    system.register_font(include_bytes!("fonts/OxiviewColorFixture.ttf").to_vec())?;
    let style = TextStyle {
        family: Some("Oxiview Color Fixture".into()),
        ..TextStyle::default()
    };
    let layout = system.layout("LMSRFBV", &style, None)?;
    for transform in [
        Affine::new([0.0625, 0.0, 0.0, 0.0625, 0.0, 0.0]),
        Affine::IDENTITY,
        Affine::new([8.0, 0.0, 0.0, 8.0, 0.0, 0.0]),
        Affine::new([0.0, 8.0, -8.0, 0.0, 300.0, 100.0]),
    ] {
        let glyphs = system.glyph_draws(&layout, Point::default(), transform)?;
        assert_eq!(glyphs.len(), 7);
        for (index, glyph) in glyphs.iter().enumerate() {
            if index == 5 {
                let GlyphDraw::Image(bitmap) = glyph else {
                    return Err(Error::InvalidGlyphImage);
                };
                assert_eq!(bitmap.image.width(), 4);
                assert_eq!(bitmap.image.height(), 4);
                assert_eq!(
                    bitmap.image.pixels().get(..4),
                    Some(&[240, 40, 180, 255][..])
                );
            } else {
                assert!(
                    matches!(glyph, GlyphDraw::Vector(glyph) if glyph.kind == GlyphKind::Color)
                );
            }
        }
    }
    let bitmap_layout = system.layout("B", &style, None)?;
    let transform = Affine::new([8.0, 0.0, 0.0, 8.0, 13.0, 17.0]);
    let origin = Point::new(0.125, 0.375);
    let first = system.glyph_draws(&bitmap_layout, Point::default(), transform)?;
    let second = system.glyph_draws(&bitmap_layout, origin, transform)?;
    let ([GlyphDraw::Image(first)], [GlyphDraw::Image(second)]) =
        (first.as_slice(), second.as_slice())
    else {
        return Err(Error::InvalidGlyphImage);
    };
    assert_eq!(
        first.image.pixels().as_ptr(),
        second.image.pixels().as_ptr()
    );
    assert_eq!(
        second.rect.origin().x() - first.rect.origin().x(),
        origin.x()
    );
    assert_eq!(
        second.rect.origin().y() - first.rect.origin().y(),
        origin.y()
    );
    Ok(())
}

#[test]
fn rich_ranges_validate_utf8_and_inherit_paragraph_style() -> Result<(), Error> {
    use oxiview_text::{RichText, TextSpan, TextSpanStyle};

    let span = |range| TextSpan {
        range,
        style: TextSpanStyle::default(),
    };
    for invalid in [span(0..1), span(1..2), span(2..2), span(2..4)] {
        assert!(matches!(
            RichText::new("éA", vec![invalid]),
            Err(Error::InvalidSpan { .. })
        ));
    }
    assert!(matches!(
        RichText::new("éA", vec![span(0..3), span(2..3)]),
        Err(Error::InvalidSpan { .. })
    ));
    let unsupported = TextSpan {
        range: 0..2,
        style: TextSpanStyle {
            font_size: Some(f64::NAN),
            ..TextSpanStyle::default()
        },
    };
    assert_eq!(
        RichText::new("éA", vec![unsupported]),
        Err(Error::InvalidInput)
    );
    let content = RichText::new("éA", vec![span(2..3), span(0..2)])?;
    assert_eq!(content.spans()[0].range, 0..2);
    assert_eq!(content.as_str(), "éA");

    let mut system = fonts()?;
    let paragraph = style();
    let ranged = system.layout_rich(&content, &paragraph, None)?;
    let plain = system.layout(content.as_str(), &paragraph, None)?;
    let origin = Point::new(0.0, 0.0);
    assert_eq!(ranged.size(), plain.size());
    assert_eq!(ranged.first_baseline(), plain.first_baseline());
    assert_eq!(
        system.glyph_images(&ranged, origin, 1.0)?,
        system.glyph_images(&plain, origin, 1.0)?
    );
    assert_eq!(system.layout("", &paragraph, None)?.first_baseline(), None);
    Ok(())
}

#[test]
fn rich_spans_change_font_size_weight_family_and_foreground() -> Result<(), Error> {
    use oxiview_text::{RichText, TextSpan, TextSpanStyle};

    let mut system = fonts()?;
    let paragraph = style();
    let red = Color::from_rgba8(212, 28, 33, 255);
    let source = RichText::new(
        "A ق",
        vec![TextSpan {
            range: 2..4,
            style: TextSpanStyle {
                family: Some("Noto Sans Arabic".into()),
                font_size: Some(46.0),
                weight: Some(850),
                color: Some(red),
                ..TextSpanStyle::default()
            },
        }],
    )?;
    let plain = system.layout(source.as_str(), &paragraph, None)?;
    let mixed = system.layout_rich(&source, &paragraph, None)?;
    assert!(mixed.size().height() > plain.size().height());
    assert!(mixed.first_baseline() > plain.first_baseline());
    assert_eq!(mixed.missing_glyph_count(), 0);
    let rewrapped = mixed.rewrap(Some(30.0))?;
    let fresh = system.layout_rich(&source, &paragraph, Some(30.0))?;
    assert_eq!(rewrapped.size(), fresh.size());
    assert_eq!(rewrapped.first_baseline(), fresh.first_baseline());
    assert_eq!(rewrapped.line_count(), fresh.line_count());
    assert_eq!(rewrapped.rewrap(None)?.size(), mixed.size());
    let images = system.glyph_images(&mixed, Point::new(0.0, 0.0), 1.0)?;
    for rgb in [[255, 180, 90], [212, 28, 33]] {
        assert!(images.iter().any(|glyph| {
            glyph
                .image
                .pixels()
                .chunks_exact(4)
                .any(|pixel| pixel[..3] == rgb && pixel[3] != 0)
        }));
    }
    let single = RichText::new(
        "ق",
        vec![TextSpan {
            range: 0..2,
            style: source.spans()[0].style.clone(),
        }],
    )?;
    let expected = TextStyle {
        family: Some("Noto Sans Arabic".into()),
        font_size: 46.0,
        weight: 850,
        color: red,
        ..paragraph
    };
    let rich = system.layout_rich(&single, &style(), None)?;
    let plain = system.layout(single.as_str(), &expected, None)?;
    assert_eq!(rich.size(), plain.size());
    assert_eq!(
        system.glyph_images(&rich, Point::new(0.0, 0.0), 1.0)?,
        system.glyph_images(&plain, Point::new(0.0, 0.0), 1.0)?
    );
    Ok(())
}

#[test]
fn italic_spans_paint_oblique_glyphs_with_upright_only_fonts() -> Result<(), Error> {
    use oxiview_text::{RichText, TextSpan, TextSpanStyle};

    let mut system = fonts()?;
    let style = style();
    let paragraph = RichText::new(
        "HELLO",
        vec![TextSpan {
            range: 0..5,
            style: TextSpanStyle {
                italic: true,
                ..TextSpanStyle::default()
            },
        }],
    )?;
    let upright = system.layout(paragraph.as_str(), &style, None)?;
    let italic = system.layout_rich(&paragraph, &style, None)?;
    let origin = Point::new(0.0, 0.0);
    let upright = system.glyph_images(&upright, origin, 1.0)?;
    let italics = system.glyph_images(&italic, origin, 1.0)?;
    assert_eq!(upright.len(), italics.len());
    assert!(
        upright
            .iter()
            .zip(&italics)
            .any(|(normal, slanted)| normal.image != slanted.image),
        "italic must change the outline, even without a dedicated italic face"
    );
    let stats = system.cache_stats();
    assert_eq!(system.glyph_images(&italic, origin, 1.0)?, italics);
    assert_eq!(system.cache_stats().misses, stats.misses);
    Ok(())
}

#[test]
fn rich_span_spacing_and_line_height_validate_before_shaping() {
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
        for style in [
            TextSpanStyle {
                letter_spacing: Some(invalid),
                ..Default::default()
            },
            TextSpanStyle {
                word_spacing: Some(invalid),
                ..Default::default()
            },
            TextSpanStyle {
                line_height: Some(invalid),
                ..Default::default()
            },
        ] {
            assert_eq!(
                RichText::new("A", vec![TextSpan { range: 0..1, style }]),
                Err(Error::InvalidInput)
            );
        }
    }
    for invalid in [-1.0, 0.0, f64::MIN_POSITIVE, 16.01] {
        assert_eq!(
            RichText::new(
                "A",
                vec![TextSpan {
                    range: 0..1,
                    style: TextSpanStyle {
                        line_height: Some(invalid),
                        ..Default::default()
                    },
                }]
            ),
            Err(Error::InvalidInput)
        );
    }
}

#[test]
fn rich_span_spacing_changes_only_ranged_advances_and_can_wrap() -> Result<(), Error> {
    let mut system = fonts()?.with_vector_threshold(0.0)?;
    let paragraph = style();
    let text = "A A A A";
    let plain = system.layout(text, &paragraph, None)?;
    let positions = |system: &mut TextSystem, layout: &TextLayout| {
        system
            .glyph_draws(layout, Point::default(), Affine::IDENTITY)
            .map(|draws| {
                draws
                    .into_iter()
                    .filter_map(|draw| match draw {
                        GlyphDraw::Vector(glyph) => Some(glyph.origin.x()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
    };
    let plain_x = positions(&mut system, &plain)?;
    assert_eq!(plain_x.len(), 7);
    for (letter_spacing, word_spacing) in [(Some(3.0), None), (None, Some(9.0))] {
        let content = RichText::new(
            text,
            vec![TextSpan {
                range: 0..3,
                style: TextSpanStyle {
                    letter_spacing,
                    word_spacing,
                    ..Default::default()
                },
            }],
        )?;
        let spaced = system.layout_rich(&content, &paragraph, None)?;
        let spaced_x = positions(&mut system, &spaced)?;
        let growth = spaced.size().width() - plain.size().width();
        assert!(growth > 2.0);
        if word_spacing.is_some() {
            assert!((growth - 9.0).abs() < 0.001);
        }
        assert_eq!(spaced_x[0], plain_x[0]);
        assert!(spaced_x[2] > plain_x[2]);
        assert!((spaced_x[5] - plain_x[5] - growth).abs() < 0.001);
        assert!((spaced_x[6] - plain_x[6] - growth).abs() < 0.001);
        assert_eq!(spaced.size().height(), plain.size().height());
        let width = Some(plain.size().width() + growth / 2.0);
        assert_eq!(plain.rewrap(width)?.line_count(), 1);
        assert_eq!(spaced.rewrap(width)?.line_count(), 2);
    }
    let tightened = RichText::new(
        text,
        vec![TextSpan {
            range: 0..3,
            style: TextSpanStyle {
                word_spacing: Some(-2.0),
                ..Default::default()
            },
        }],
    )?;
    assert!(
        (system
            .layout_rich(&tightened, &paragraph, None)?
            .size()
            .width()
            - plain.size().width()
            + 2.0)
            .abs()
            < 0.001
    );
    Ok(())
}

#[test]
fn rich_span_line_height_changes_only_the_affected_line() -> Result<(), Error> {
    let mut system = fonts()?;
    let paragraph = TextStyle {
        line_height: 1.5,
        ..style()
    };
    let text = "AA\nAA";
    let source = RichText::new(
        text,
        vec![TextSpan {
            range: 3..5,
            style: TextSpanStyle {
                line_height: Some(2.5),
                ..Default::default()
            },
        }],
    )?;
    let plain = system.layout(text, &paragraph, None)?;
    let rich = system.layout_rich(&source, &paragraph, None)?;
    assert_eq!(rich.line_count(), 2);
    assert_eq!(rich.size().width(), plain.size().width());
    assert_eq!(rich.first_baseline(), plain.first_baseline());
    assert!((rich.size().height() - plain.size().height() - paragraph.font_size).abs() < 0.001);
    Ok(())
}

#[test]
fn rich_span_decoration_rectangles_use_foreground_and_font_metrics() -> Result<(), Error> {
    let mut system = fonts()?;
    let paragraph = TextStyle {
        font_size: 40.0,
        ..style()
    };
    let accent = Color::from_rgba8(54, 143, 212, 255);
    let source = RichText::new(
        "AA BB CC",
        vec![
            TextSpan {
                range: 0..2,
                style: TextSpanStyle {
                    underline: true,
                    strikethrough: true,
                    ..Default::default()
                },
            },
            TextSpan {
                range: 3..5,
                style: TextSpanStyle {
                    color: Some(accent),
                    font_size: Some(32.0),
                    underline: true,
                    ..Default::default()
                },
            },
        ],
    )?;
    let layout = system.layout_rich(&source, &paragraph, None)?;
    let mut undecorated = source.spans().to_vec();
    for span in &mut undecorated {
        span.style.underline = false;
        span.style.strikethrough = false;
    }
    let undecorated = RichText::new(source.as_str(), undecorated)?;
    let plain = system.layout_rich(&undecorated, &paragraph, None)?;
    assert_eq!(layout.size(), plain.size());
    assert_eq!(layout.first_baseline(), plain.first_baseline());
    assert_eq!(plain.decorations().count(), 0);
    assert_eq!(
        system.glyph_images(&layout, Point::default(), 2.0)?,
        system.glyph_images(&plain, Point::default(), 2.0)?
    );

    let baseline = layout.first_baseline().ok_or(Error::MissingFont)?;
    let rules: Vec<_> = layout.decorations().collect();
    assert_eq!(rules.len(), 3);
    let [underline, strike, second] = [rules[0], rules[1], rules[2]];
    assert_eq!(underline.kind, TextDecorationKind::Underline);
    assert_eq!(strike.kind, TextDecorationKind::Strikethrough);
    assert_eq!(second.kind, TextDecorationKind::Underline);
    assert_eq!(underline.color, paragraph.color);
    assert_eq!(strike.color, paragraph.color);
    assert_eq!(second.color, accent);
    // Noto Sans uses a 1000-unit em, -100/322-unit offsets, and 50-unit rules.
    assert!((underline.rect.origin().y() - baseline - 4.0).abs() < 0.001);
    assert!((strike.rect.origin().y() - baseline + 12.88).abs() < 0.001);
    assert!((second.rect.origin().y() - baseline - 3.2).abs() < 0.001);
    assert!((second.rect.size().height() - 1.6).abs() < 0.001);
    for rule in [underline, strike] {
        assert!((rule.rect.size().height() - 2.0).abs() < 0.001);
    }
    let first_width = system.layout("AA", &paragraph, None)?.size().width();
    assert_eq!(underline.rect.origin().x(), 0.0);
    assert!((underline.rect.size().width() - first_width).abs() < 0.001);
    assert_eq!(strike.rect.size().width(), underline.rect.size().width());
    assert!(second.rect.origin().x() > first_width);
    assert!(second.rect.edges()[2] < layout.size().width());
    Ok(())
}
