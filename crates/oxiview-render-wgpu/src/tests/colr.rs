use super::*;

#[test]
fn uses_palette_zero_from_the_second_record_range_and_foreground() -> TestResult {
    use skrifa::raw::TableProvider;

    let context = RenderContext::new()?;
    let mut glyph = colr_fixture(9)?;
    let original = glyph.font.bytes();
    let palette = skrifa::FontRef::new(original)?.cpal()?;
    let offset = palette.offset_data().as_bytes().as_ptr().addr() - original.as_ptr().addr();
    let mut bytes = original.to_vec();
    // Point palette zero at the second palette's magenta records, as in the
    // CPU conformance test. Keep all color records in bounds.
    let second = palette.color_record_indices()[1].get();
    bytes[offset + 12..offset + 14].copy_from_slice(&second.to_be_bytes());
    glyph.font = oxiview_base::GlyphFont::new(bytes.clone().into());
    let output = render_colr(&context, glyph.clone())?;
    colr_pixel(&output, 40, 60, [255, 0, 255, 255], 0);
    glyph.glyph_id = 7;
    glyph.color = Color::from_rgba8(0, 255, 255, 128);
    let output = render_colr(&context, glyph.clone())?;
    colr_pixel(&output, 40, 60, [0, 255, 255, 128], 0);

    // The foreground still works with a short palette, but index 4 must be rejected.
    bytes[offset + 2..offset + 4].copy_from_slice(&1_u16.to_be_bytes());
    glyph.glyph_id = 9;
    glyph.font = oxiview_base::GlyphFont::new(bytes.into());
    let error = render_colr(&context, glyph).err();
    assert!(matches!(
        error.as_deref().and_then(|error| error.downcast_ref::<Error>()),
        Some(Error::SceneEncoding(message)) if message.contains("palette index")
    ));
    Ok(())
}

const COLR_SIZE: PixelSize = PixelSize {
    width: 128,
    height: 128,
};

fn colr_fixture(id: u16) -> Result<oxiview_base::Glyph, Box<dyn std::error::Error>> {
    let NodeKind::Draw(Primitive::Glyph(mut glyph)) = fixture_glyph(
        id,
        oxiview_base::GlyphKind::Color,
        Color::from_rgba8(0, 0, 0, 255),
    ) else {
        return Err("missing color fixture glyph".into());
    };
    glyph.size = 100.0;
    glyph.origin = Point::new(0.0, 100.0);
    Ok(glyph)
}

fn colr_pixel(image: &RgbaImage, x: usize, y: usize, expected: [u8; 4], tolerance: u8) {
    let actual = pixel(image, x, y);
    // Vello 0.4 uses 8-bit premultiplied linear gradient ramps and layer
    // buffers. A step can span several straight sRGB codes at low alpha.
    assert!(
        actual[..3]
            .iter()
            .zip(expected)
            .all(|(&a, e)| a.abs_diff(e) <= tolerance)
            && actual[3].abs_diff(expected[3]) <= tolerance.min(1),
        "at ({x}, {y}) expected {expected:?}, got {actual:?}"
    );
}

fn render_colr(
    context: &RenderContext,
    glyph: oxiview_base::Glyph,
) -> Result<RgbaImage, Box<dyn std::error::Error>> {
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Draw(Primitive::Glyph(glyph)))?;
    Ok(context.render(&scene, root, COLR_SIZE)?)
}

#[test]
fn empty_lines_are_transparent_even_in_src_in() -> TestResult {
    let context = RenderContext::new()?;
    for id in [14, 18] {
        let image = render_colr(&context, colr_fixture(id)?)?;
        assert!(image.pixels().iter().all(|&byte| byte == 0), "glyph {id}");
    }
    Ok(())
}

#[test]
fn single_stop_repeat_reflect_and_sweep_keep_coverage() -> TestResult {
    let context = RenderContext::new()?;
    let linear = render_colr(&context, colr_fixture(15)?)?;
    let radial = render_colr(&context, colr_fixture(19)?)?;
    let sweep = render_colr(&context, colr_fixture(20)?)?;
    for x in [10, 40, 70] {
        colr_pixel(&linear, x, 60, [128, 30, 90, 128], 4);
        colr_pixel(&radial, x, 60, [128, 30, 90, 255], 2);
        colr_pixel(&radial, x, 30, [0, 0, 0, 0], 0);
    }
    for (x, y) in [(20, 40), (60, 40), (60, 80), (20, 80)] {
        colr_pixel(&sweep, x, y, [128, 30, 90, 255], 2);
    }
    Ok(())
}

#[test]
fn variable_stop_uses_normalized_coordinates() -> TestResult {
    let context = RenderContext::new()?;
    let mut glyph = colr_fixture(17)?;
    glyph.variations = Arc::from([0]);
    let normal = render_colr(&context, glyph.clone())?;
    glyph.variations = Arc::from([16384]);
    let heavy = render_colr(&context, glyph.clone())?;
    colr_pixel(&normal, 40, 60, [128, 30, 90, 128], 4);
    colr_pixel(&heavy, 40, 60, [128, 30, 90, 255], 2);

    glyph.variations = Arc::from([0]);
    let again = render_colr(&context, glyph)?;
    assert_eq!(again.pixels(), normal.pixels());
    Ok(())
}

#[test]
fn oversized_colr_collection_keeps_face_and_variation_on_each_draw() -> TestResult {
    use skrifa::raw::TableProvider;

    let context = RenderContext::new()?;
    let mut glyph = colr_fixture(17)?;
    let font = glyph.font.bytes();
    let palette = skrifa::FontRef::new(font)?.cpal()?;
    let palette_index = palette.offset_data().as_bytes().as_ptr().addr() - font.as_ptr().addr()
        + palette.color_record_indices_byte_range().start;
    let magenta = palette.color_record_indices()[1].get().to_be_bytes();
    let table_count = u16::from_be_bytes(font[4..6].try_into()?);
    let face_len = font.len().next_multiple_of(4);
    let mut collection = b"ttcf\0\x01\0\0\0\0\0\x02".to_vec();
    for face_offset in [20, 20 + face_len] {
        collection.extend_from_slice(&u32::try_from(face_offset)?.to_be_bytes());
    }
    for face_offset in [20, 20 + face_len] {
        collection.resize(face_offset, 0);
        collection.extend_from_slice(font);
        // TTC table offsets are relative to the collection, not the face.
        for record in 0..usize::from(table_count) {
            let offset = face_offset + 12 + record * 16 + 8;
            let old = u32::from_be_bytes(collection[offset..offset + 4].try_into()?);
            let new = old + u32::try_from(face_offset)?;
            collection[offset..offset + 4].copy_from_slice(&new.to_be_bytes());
        }
    }
    let second_palette = 20 + face_len + palette_index;
    collection[second_palette..second_palette + 2].copy_from_slice(&magenta);

    // Draw the same requests through the persistent cache and the transient
    // path. A 17 MiB original plus its normalized copy exceeds the budget.
    for font in [collection.clone(), {
        collection.resize(17 * 1024 * 1024, 0);
        collection
    }] {
        glyph.font = oxiview_base::GlyphFont::new(font.into());
        for (face, location, expected) in [
            (0, 0, [128, 30, 90, 128]),
            (1, 16384, [255, 0, 255, 255]),
            (1, 0, [255, 0, 255, 128]),
            (0, 16384, [128, 30, 90, 255]),
            (0, 0, [128, 30, 90, 128]),
        ] {
            glyph.face_index = face;
            glyph.variations = Arc::from([location]);
            let output = render_colr(&context, glyph.clone())?;
            colr_pixel(&output, 40, 60, expected, 4);
        }
    }
    Ok(())
}

#[test]
fn interpolates_premultiplied_linear_stops() -> TestResult {
    let context = RenderContext::new()?;
    let image = render_colr(&context, colr_fixture(13)?)?;
    colr_pixel(&image, 39, 60, [83, 136, 171, 158], 2);
    colr_pixel(&image, 0, 60, [126, 39, 94, 65], 6);
    Ok(())
}

#[test]
fn screens_in_linear_light() -> TestResult {
    let context = RenderContext::new()?;
    let image = render_colr(&context, colr_fixture(16)?)?;
    colr_pixel(&image, 10, 60, [128, 30, 90, 255], 2);
    colr_pixel(&image, 40, 60, [139, 152, 194, 255], 1);
    colr_pixel(&image, 70, 60, [65, 150, 185, 255], 1);
    Ok(())
}

#[test]
fn interpolates_with_variable_stop_positions() -> TestResult {
    use skrifa::{
        Tag,
        raw::{TableProvider, tables::colr::Paint},
    };
    let context = RenderContext::new()?;
    let mut glyph = colr_fixture(17)?;
    let original = glyph.font.bytes();
    let face = skrifa::FontRef::new(original)?;
    let tag = Tag::new(b"COLR");
    let colr_data = face.table_data(tag).ok_or("missing COLR table")?;
    let colr = face.colr()?;
    let Some((Paint::Glyph(clip), _)) = colr.v1_base_glyph(17_u16.into()) else {
        return Err("missing variable glyph clip".into());
    };
    let Paint::VarLinearGradient(paint) = clip.paint()? else {
        return Err("missing variable gradient".into());
    };
    let paint_offset =
        paint.offset_data().as_bytes().as_ptr().addr() - colr_data.as_bytes().as_ptr().addr();

    // Add a second line to the fixture's COLR table. Delta index 1 moves
    // the red stop from 0 to 0.5, with no alpha change.
    let mut table = colr_data.as_bytes().to_vec();
    let new_line = u32::try_from(table.len() - paint_offset)?.to_be_bytes();
    table[paint_offset + 1..paint_offset + 4].copy_from_slice(&new_line[1..]);
    table.extend_from_slice(&[0, 0, 2]); // Pad, two VarColorStops.
    for (stop, palette, var_index) in [(0_i16, 0_u16, 1_u32), (16384, 1, u32::MAX)] {
        table.extend_from_slice(&stop.to_be_bytes());
        table.extend_from_slice(&palette.to_be_bytes());
        table.extend_from_slice(&16384_i16.to_be_bytes());
        table.extend_from_slice(&var_index.to_be_bytes());
    }
    let record = face
        .table_directory()
        .table_records()
        .iter()
        .find(|record| record.tag() == tag)
        .ok_or("missing COLR directory record")?;
    let record_offset = std::ptr::from_ref(record).addr() - original.as_ptr().addr();
    let mut bytes = original.to_vec();
    bytes.resize(bytes.len().next_multiple_of(4), 0);
    let offset = u32::try_from(bytes.len())?.to_be_bytes();
    bytes[record_offset + 8..record_offset + 12].copy_from_slice(&offset);
    bytes[record_offset + 12..record_offset + 16]
        .copy_from_slice(&u32::try_from(table.len())?.to_be_bytes());
    bytes.extend(table);
    glyph.font = oxiview_base::GlyphFont::new(bytes.into());

    let normal = render_colr(&context, glyph.clone())?;
    glyph.variations = Arc::from([16384]);
    let heavy = render_colr(&context, glyph)?;
    colr_pixel(&normal, 39, 60, [187, 0, 186, 255], 2);
    colr_pixel(&heavy, 39, 60, [255, 0, 0, 255], 0);
    colr_pixel(&heavy, 59, 60, [190, 0, 185, 255], 2);
    Ok(())
}

#[test]
fn keeps_retained_state_and_matches_presentation_output() -> TestResult {
    let context = RenderContext::new()?;
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Transform(Affine::IDENTITY))?;
    let opacity = scene.insert(Some(root), 0, NodeKind::Opacity(0.5))?;
    let placement = scene.insert(
        Some(opacity),
        0,
        NodeKind::Transform(Affine::new([1.0, 0.0, 0.0, 1.0, 10.0, -10.0])),
    )?;
    let clip = scene.insert(
        Some(placement),
        0,
        NodeKind::Clip(Clip::Rect(Rect::new(5.0, 30.0, 55.0, 90.0))),
    )?;
    scene.insert(
        Some(clip),
        0,
        fill(Rect::new(5.0, 30.0, 55.0, 36.0), blue()),
    )?;
    scene.insert(
        Some(clip),
        1,
        NodeKind::Draw(Primitive::Glyph(colr_fixture(18)?)),
    )?;
    let mut glyph = colr_fixture(16)?;
    glyph.color = Color::from_rgba8(32, 40, 60, 128);
    scene.insert(Some(clip), 2, NodeKind::Draw(Primitive::Glyph(glyph)))?;
    scene.insert(
        Some(clip),
        3,
        fill(Rect::new(44.0, 75.0, 55.0, 90.0), red()),
    )?;
    scene.insert(
        Some(root),
        1,
        fill(Rect::new(95.0, 95.0, 105.0, 105.0), blue()),
    )?;

    let output = context.render(&scene, root, COLR_SIZE)?;
    colr_pixel(&output, 12, 55, [0, 0, 0, 0], 0);
    colr_pixel(&output, 70, 55, [0, 0, 0, 0], 0);
    colr_pixel(&output, 50, 55, [139, 152, 194, 64], 3);
    // Screen is isolated from the earlier blue Scene sibling. Their result
    // composites once at the glyph's half opacity and again at group opacity.
    colr_pixel(&output, 50, 23, [101, 110, 228, 128], 4);
    colr_pixel(&output, 58, 68, [255, 0, 0, 128], 0);
    colr_pixel(&output, 100, 100, [0, 0, 255, 255], 0);

    // Exercise the same texture path as WindowSurface without opening a window.
    let encoded = scene.encode(root)?;
    let mut process = context.process.lock().map_err(|e| e.to_string())?;
    let gpu = process.gpu.as_mut().ok_or("missing GPU")?;
    for format in copy::SURFACE_FORMATS {
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("COLR presentation target"),
            size: wgpu::Extent3d {
                width: COLR_SIZE.width,
                height: COLR_SIZE.height,
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
            .render_scene(&encoded, COLR_SIZE, Some(&target), true)?
            .ok_or("missing surface pixels")?;
        for (x, y) in [(12, 55), (70, 55), (50, 55), (50, 23), (58, 68), (100, 100)] {
            let expected = pixel(&output, x, y).try_into()?;
            colr_pixel(&surface, x, y, expected, 2);
        }
    }
    Ok(())
}

#[test]
fn zero_angle_repeat_sweep_is_transparent() -> TestResult {
    use skrifa::raw::{TableProvider, tables::colr::Paint};

    let mut glyph = colr_fixture(20)?;
    let original = glyph.font.bytes();
    let colr = skrifa::FontRef::new(original)?.colr()?;
    let Some((Paint::Glyph(clip), _)) = colr.v1_base_glyph(20_u16.into()) else {
        return Err("missing sweep glyph clip".into());
    };
    let Paint::SweepGradient(paint) = clip.paint()? else {
        return Err("missing sweep paint".into());
    };
    let paint_start = paint.offset_data().as_bytes().as_ptr().addr() - original.as_ptr().addr();
    let line = paint.color_line()?;
    let line_start = line.offset_data().as_bytes().as_ptr().addr() - original.as_ptr().addr();
    let mut bytes = original.to_vec();
    let end_angle = paint.end_angle_byte_range();
    bytes[paint_start + end_angle.start..paint_start + end_angle.end]
        .copy_from_slice(&paint.start_angle().to_bits().to_be_bytes());
    bytes[line_start] = skrifa::color::Extend::Repeat as u8;

    glyph.font = oxiview_base::GlyphFont::new(bytes.into());
    let rendered = render_colr(&RenderContext::new()?, glyph)?;
    assert!(rendered.pixels().iter().all(|&byte| byte == 0));
    Ok(())
}

#[test]
fn single_stop_radial_with_zero_end_radius_keeps_conical_coverage() -> TestResult {
    use skrifa::raw::{TableProvider, tables::colr::Paint};

    let mut glyph = colr_fixture(19)?;
    let original = glyph.font.bytes();
    let colr = skrifa::FontRef::new(original)?.colr()?;
    let Some((Paint::Glyph(clip), _)) = colr.v1_base_glyph(19_u16.into()) else {
        return Err("missing radial glyph clip".into());
    };
    let Paint::RadialGradient(paint) = clip.paint()? else {
        return Err("missing radial paint".into());
    };
    let paint_start = paint.offset_data().as_bytes().as_ptr().addr() - original.as_ptr().addr();
    let mut bytes = original.to_vec();
    let end_radius = paint.radius1_byte_range();
    bytes[paint_start + end_radius.start..paint_start + end_radius.end].fill(0);

    let face = skrifa::FontRef::new(original)?;
    let cpal = face.cpal()?;
    let cpal_start = cpal.offset_data().as_bytes().as_ptr().addr() - original.as_ptr().addr();
    let index = cpal_start + cpal.color_record_indices_byte_range().start;
    let line = paint.color_line()?;
    let line_start = line.offset_data().as_bytes().as_ptr().addr() - original.as_ptr().addr();
    let stop_index = line_start + line.color_stops_byte_range().start + 2;
    let context = RenderContext::new()?;
    glyph.font = oxiview_base::GlyphFont::new(bytes.clone().into());
    let image = render_colr(&context, glyph.clone())?;
    colr_pixel(&image, 40, 60, [128, 30, 90, 255], 2);
    colr_pixel(&image, 40, 30, [0, 0, 0, 0], 0);
    // Point palette zero at a range that overlaps the original range.
    // Entry zero in the selected palette is blue.
    bytes[index..index + 2].copy_from_slice(&1_u16.to_be_bytes());
    bytes[stop_index..stop_index + 2].fill(0);
    glyph.font = oxiview_base::GlyphFont::new(bytes.into());
    let image = render_colr(&context, glyph)?;
    colr_pixel(&image, 40, 60, [0, 0, 255, 255], 0);
    colr_pixel(&image, 40, 30, [0, 0, 0, 0], 0);
    Ok(())
}
