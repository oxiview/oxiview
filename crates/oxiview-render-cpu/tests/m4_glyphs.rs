//! Headless checks for explicit glyph descriptors, including non-axis-aligned Scene transforms.

use oxiview_base::{
    Affine, Color, CornerRadii, Glyph, GlyphFont, GlyphKind, PixelSize, Point, Rect, RgbaImage,
};
use oxiview_render_cpu::{CpuRenderer, Error, Renderer};
use oxiview_scene::{Brush, Clip, NodeKind, Primitive, Scene};
use std::sync::Arc;

const FIXTURE: &[u8] = include_bytes!("../../oxiview-text/tests/fonts/OxiviewColorFixture.ttf");
const ALPHA_GRADIENT: u16 = 13;
const EMPTY_GRADIENT: u16 = 14;
const ONE_STOP: u16 = 15;
const SCREEN: u16 = 16;
const VARIABLE: u16 = 17;
const TRANSPARENT_SOURCE: u16 = 18;
const ONE_STOP_RADIAL: u16 = 19;
const ONE_STOP_SWEEP: u16 = 20;
const SIZE: PixelSize = PixelSize {
    width: 128,
    height: 128,
};

fn outline() -> Glyph {
    Glyph {
        font: GlyphFont::new(Arc::from(FIXTURE)),
        face_index: 0,
        glyph_id: 2,
        size: 50.0,
        origin: Point::new(10.25, 40.5),
        color: Color::from_rgba8(12, 80, 220, 255),
        variations: Arc::from([]),
        kind: GlyphKind::Outline,
    }
}

fn color_glyph(id: u16) -> Glyph {
    Glyph {
        glyph_id: id,
        kind: GlyphKind::Color,
        size: 100.0,
        origin: Point::new(0.0, 100.0),
        color: Color::from_rgba8(0, 0, 0, 255),
        ..outline()
    }
}

fn render(glyph: Glyph, transform: Affine) -> Result<RgbaImage, Box<dyn std::error::Error>> {
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Transform(transform))?;
    scene.insert(Some(root), 0, NodeKind::Draw(Primitive::Glyph(glyph)))?;
    Ok(CpuRenderer.render(&scene, root, SIZE)?)
}

fn pixel(image: &RgbaImage, x: usize, y: usize) -> &[u8] {
    let index = (y * image.width() as usize + x) * 4;
    &image.pixels()[index..index + 4]
}

#[test]
fn transformed_outline_uses_the_baseline_and_complete_scene_affine()
-> Result<(), Box<dyn std::error::Error>> {
    let glyph = outline();
    let transform = Affine::new([0.0, 2.0, -1.5, 0.5, 99.0, 6.0]);
    let image = render(glyph, transform)?;
    // (15.25, 35.5) lies inside the font's triangle.
    assert_eq!(pixel(&image, 46, 54), &[12, 80, 220, 255]);
    // (40.25, 15.5) is inside the bounding box but outside the triangle.
    assert_eq!(pixel(&image, 76, 94), &[0, 0, 0, 0]);
    assert_eq!(pixel(&image, 15, 35), &[0, 0, 0, 0]);
    Ok(())
}

#[test]
fn malformed_or_missing_font_face_returns_a_render_error() -> Result<(), Box<dyn std::error::Error>>
{
    let mut glyph = outline();
    glyph.font = GlyphFont::new(Arc::from([1, 2, 3]));
    let error = render(glyph, Affine::IDENTITY).err();
    assert!(
        error.is_some_and(|error| matches!(error.downcast_ref::<Error>(), Some(Error::Glyph(_))))
    );
    Ok(())
}

#[test]
fn color_sweep_rotates_with_the_scene() -> Result<(), Box<dyn std::error::Error>> {
    let mut glyph = outline();
    glyph.glyph_id = 5;
    glyph.kind = GlyphKind::Color;
    glyph.origin = Point::new(10.0, 45.0);
    let rotation = Affine::new([0.0, 1.0, -1.0, 0.0, 100.0, 20.0]);
    let image = render(glyph, rotation)?;
    // The two original upper quadrants rotate about the glyph with the Scene.
    assert_eq!(pixel(&image, 85, 60), &[255, 0, 0, 255]);
    assert_eq!(pixel(&image, 85, 40), &[0, 255, 0, 255]);
    assert_eq!(pixel(&image, 65, 40), &[0, 0, 255, 255]);
    assert_eq!(pixel(&image, 65, 60), &[255, 255, 0, 255]);
    Ok(())
}

#[test]
fn colr_gradient_interpolation_matches_the_linear_light_spec()
-> Result<(), Box<dyn std::error::Error>> {
    let image = render(color_glyph(3), Affine::IDENTITY)?;
    let midpoint = pixel(&image, 40, 69);
    // Near the middle of a red-to-blue ramp both channels should approach 188.
    assert_pixel(midpoint, [188, 0, 188, 255], 3);
    Ok(())
}

#[test]
fn vector_outlines_apply_normalized_variation_coordinates() -> Result<(), Box<dyn std::error::Error>>
{
    let mut glyph = outline();
    glyph.font = GlyphFont::new(Arc::from(
        &include_bytes!("../../oxiview-text/tests/fonts/NotoSans.ttf")[..],
    ));
    glyph.glyph_id = 36; // U+0041 in the pinned Noto Sans font.
    glyph.origin = Point::new(20.0, 75.0);
    glyph.size = 80.0;
    glyph.variations = Arc::from([0, 0]);
    let normal = render(glyph.clone(), Affine::IDENTITY)?;
    glyph.variations = Arc::from([16384, 0]);
    let bold = render(glyph, Affine::IDENTITY)?;
    let ink = |image: &RgbaImage| {
        image
            .pixels()
            .chunks_exact(4)
            .map(|pixel| usize::from(pixel[3]))
            .sum::<usize>()
    };
    assert!(ink(&bold) > ink(&normal) * 3 / 2);
    Ok(())
}

fn assert_pixel(actual: &[u8], expected: [u8; 4], tolerance: u8) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(&a, e)| a.abs_diff(e) <= tolerance),
        "expected {expected:?}, got {actual:?}"
    );
}

#[test]
fn interpolates_premultiplied_stops_with_cpal_and_paint_alpha()
-> Result<(), Box<dyn std::error::Error>> {
    let image = render(color_glyph(ALPHA_GRADIENT), Affine::IDENTITY)?;
    // [128, 30, 90] has CPAL alpha 128/255 and paint alpha 1/2.
    // Its contribution to the midpoint is about one quarter of the other stop's.
    assert_pixel(pixel(&image, 39, 60), [83, 136, 171, 158], 1);
    assert_pixel(pixel(&image, 0, 60), [126, 39, 94, 65], 2);
    Ok(())
}

#[test]
fn screens_nongray_midtones_in_linear_light() -> Result<(), Box<dyn std::error::Error>> {
    let image = render(color_glyph(SCREEN), Affine::IDENTITY)?;
    assert_eq!(pixel(&image, 10, 60), [128, 30, 90, 255]);
    assert_pixel(pixel(&image, 40, 60), [139, 152, 194, 255], 1);
    assert_eq!(pixel(&image, 70, 60), [65, 150, 185, 255]);
    Ok(())
}

#[test]
fn empty_lines_are_transparent_and_single_stops_are_constant()
-> Result<(), Box<dyn std::error::Error>> {
    // The empty source in SrcIn must erase the glyph's internal backdrop.
    for id in [EMPTY_GRADIENT, TRANSPARENT_SOURCE] {
        let image = render(color_glyph(id), Affine::IDENTITY)?;
        assert!(image.pixels().iter().all(|&byte| byte == 0));
    }
    let image = render(color_glyph(ONE_STOP), Affine::IDENTITY)?;
    for x in [10, 40, 70] {
        assert_pixel(pixel(&image, x, 60), [128, 30, 90, 128], 1);
    }
    let image = render(color_glyph(ONE_STOP_RADIAL), Affine::IDENTITY)?;
    for x in [10, 40, 70] {
        assert_eq!(pixel(&image, x, 60), [128, 30, 90, 255]);
    }
    // A constant radial color line still only paints inside its strip.
    assert_eq!(pixel(&image, 40, 30), [0, 0, 0, 0]);
    let image = render(color_glyph(ONE_STOP_SWEEP), Affine::IDENTITY)?;
    for (x, y) in [(20, 40), (60, 40), (60, 80), (20, 80)] {
        assert_eq!(pixel(&image, x, y), [128, 30, 90, 255]);
    }
    Ok(())
}

#[test]
fn variable_colr_stop_uses_the_glyphs_normalized_coordinates()
-> Result<(), Box<dyn std::error::Error>> {
    let mut glyph = color_glyph(VARIABLE);
    glyph.variations = Arc::from([0]);
    let normal = render(glyph.clone(), Affine::IDENTITY)?;
    glyph.variations = Arc::from([16384]);
    let heavy = render(glyph, Affine::IDENTITY)?;
    assert_pixel(pixel(&normal, 40, 60), [128, 30, 90, 128], 1);
    assert_eq!(pixel(&heavy, 40, 60), [128, 30, 90, 255]);
    Ok(())
}

#[test]
fn uses_the_first_palette_mapping_and_rejects_invalid_cpal()
-> Result<(), Box<dyn std::error::Error>> {
    use skrifa::{FontRef, raw::TableProvider};
    let palette_table = FontRef::new(FIXTURE)?.cpal()?;
    let data = palette_table.offset_data().as_ref().as_ptr().addr() - FIXTURE.as_ptr().addr();
    let mut bytes = FIXTURE.to_vec();
    // CPAL's first palette points at the second palette's magenta records.
    let second = palette_table.color_record_indices()[1].get();
    bytes[data + 12..data + 14].copy_from_slice(&second.to_be_bytes());
    let mut glyph = color_glyph(9);
    glyph.font = GlyphFont::new(Arc::from(bytes.clone()));
    let image = render(glyph.clone(), Affine::IDENTITY)?;
    assert_eq!(pixel(&image, 40, 60), [255, 0, 255, 255]);

    // Shrink each palette to one entry while leaving all color records in bounds.
    bytes[data + 2..data + 4].copy_from_slice(&1_u16.to_be_bytes());
    glyph.font = GlyphFont::new(Arc::from(bytes.clone()));
    assert!(matches!(
        render(glyph.clone(), Affine::IDENTITY).err().as_deref()
            .and_then(|e| e.downcast_ref::<Error>()),
        Some(Error::Glyph(message)) if message.contains("palette index")
    ));

    bytes[data + 12..data + 14].copy_from_slice(&u16::MAX.to_be_bytes());
    glyph.font = GlyphFont::new(Arc::from(bytes));
    assert!(matches!(
        render(glyph, Affine::IDENTITY).err().as_deref()
            .and_then(|e| e.downcast_ref::<Error>()),
        Some(Error::Glyph(message)) if message.contains("usable palette")
    ));
    Ok(())
}

#[test]
fn color_glyph_composites_inside_the_retained_clip_and_opacity()
-> Result<(), Box<dyn std::error::Error>> {
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
    let fill = |rect, color| {
        NodeKind::Draw(Primitive::Fill(
            rect,
            CornerRadii::default(),
            Brush::Solid(color),
        ))
    };
    let blue = Color::from_rgba8(0, 0, 255, 255);
    let red = Color::from_rgba8(255, 0, 0, 255);
    scene.insert(Some(clip), 0, fill(Rect::new(5.0, 30.0, 55.0, 36.0), blue))?;
    let glyph = Glyph {
        color: Color::from_rgba8(32, 40, 60, 128),
        ..color_glyph(SCREEN)
    };
    scene.insert(Some(clip), 1, NodeKind::Draw(Primitive::Glyph(glyph)))?;
    scene.insert(Some(clip), 2, fill(Rect::new(44.0, 75.0, 55.0, 90.0), red))?;
    scene.insert(
        Some(root),
        1,
        fill(Rect::new(95.0, 95.0, 105.0, 105.0), blue),
    )?;
    let image = CpuRenderer.render(&scene, root, SIZE)?;
    assert_eq!(pixel(&image, 12, 55), [0, 0, 0, 0]);
    assert_eq!(pixel(&image, 70, 55), [0, 0, 0, 0]);
    assert_pixel(pixel(&image, 50, 55), [139, 152, 194, 64], 2);
    // Screen gives linear G = 1 - (1 - C6) × (1 - C7), with C6/C7 the
    // decoded palette colors. Encode (128/255) × G + (127/255) × blue.
    // The retained group halves the opaque composite's alpha.
    assert_pixel(pixel(&image, 50, 23), [101, 111, 227, 128], 1);
    assert_eq!(pixel(&image, 58, 68), [255, 0, 0, 128]);
    assert_eq!(pixel(&image, 100, 100), [0, 0, 255, 255]);
    Ok(())
}

#[test]
fn clipped_glyph_samples_the_same_gradient_across_tile_edges()
-> Result<(), Box<dyn std::error::Error>> {
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Transform(Affine::IDENTITY))?;
    let glyph = Glyph {
        size: 400.0,
        origin: Point::new(-32.0, 300.0),
        ..color_glyph(ALPHA_GRADIENT)
    };
    scene.insert(Some(root), 0, NodeKind::Draw(Primitive::Glyph(glyph)))?;
    let image = CpuRenderer.render(
        &scene,
        root,
        PixelSize {
            width: 257,
            height: 145,
        },
    )?;
    assert_pixel(pixel(&image, 127, 127), [82, 137, 171, 159], 2);
    assert_pixel(pixel(&image, 128, 128), [82, 137, 171, 160], 2);
    assert!(pixel(&image, 256, 144)[3] > 230);
    Ok(())
}
