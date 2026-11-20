//! Compare GPU COLRv1 cone sampling with the CPU adapter on derived fonts.

use oxiview_base::{Color, Glyph, GlyphFont, GlyphKind, PixelSize, Point, RgbaImage};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_render_wgpu::RenderContext;
use oxiview_scene::{NodeKind, Primitive, Scene};
use std::sync::Arc;

const FIXTURE: &[u8] = include_bytes!("../../oxiview-text/tests/fonts/OxiviewColorFixture.ttf");
const SIZE: PixelSize = PixelSize {
    width: 100,
    height: 100,
};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const LINEAR: u8 = 4;
const RADIAL: u8 = 6;
const SWEEP: u8 = 8;

/// Replace glyph 6's radial brush while keeping the fixture and its square clip intact.
fn radial_font(extend: u8, reverse: bool, variable: bool, shifted: bool) -> TestResult<GlyphFont> {
    gradient_font(RADIAL, extend, reverse, variable, shifted, false)
}

fn gradient_font(
    format: u8,
    extend: u8,
    reverse: bool,
    variable: bool,
    shifted: bool,
    coincident: bool,
) -> TestResult<GlyphFont> {
    let record_start = (12..12 + 16 * usize::from(u16::from_be_bytes([FIXTURE[4], FIXTURE[5]])))
        .step_by(16)
        .find(|&offset| &FIXTURE[offset..offset + 4] == b"COLR")
        .ok_or("missing COLR directory entry")?;
    let table_start = read_u32(FIXTURE, record_start + 8)?;
    let table_len = read_u32(FIXTURE, record_start + 12)?;
    let mut table = FIXTURE[table_start..table_start + table_len].to_vec();
    let bases = read_u32(&table, 14)?; // COLRv1 baseGlyphListOffset
    if coincident && variable {
        // Replace the fixture's unrelated variable singleton so the new stop
        // positions alone trigger location-specific font normalization.
        replace_brush(&mut table, bases, 17)?;
        table.extend_from_slice(&[2, 0xff, 0xff, 0, 0]); // Transparent foreground.
    }
    replace_brush(&mut table, bases, 6)?;
    if shifted {
        // PaintTranslate moves the brush fifty font units without moving the clip.
        table.extend_from_slice(&[14, 0, 0, 8, 0, 50, 0, 0]);
    }
    let brush_start = table.len();
    table.extend_from_slice(&[format + u8::from(variable), 0, 0, 0]);
    let tip_radius = if variable { 100 } else { 0 };
    let mut circles = [[600_i16, 400, 200], [200, 400, tip_radius]];
    if reverse {
        circles.reverse();
    }
    let geometry: &[i16] = match format {
        LINEAR => &[200, 400, 600, 400, 200, 800],
        SWEEP => &[400, 400, -16384, 16384],
        _ => circles.as_flattened(),
    };
    for value in geometry {
        table.extend_from_slice(&value.to_be_bytes());
    }
    if variable {
        table.extend_from_slice(&0_u32.to_be_bytes()); // varIndexBase
    }
    if coincident && format == RADIAL {
        // Glyph 3 shares the same ColorLine but projects it along a line.
        // Both paints must recover their own geometry.
        replace_brush(&mut table, bases, 3)?;
        table.extend_from_slice(&[
            LINEAR + u8::from(variable),
            0,
            0,
            if variable { 20 } else { 16 },
        ]);
        for value in [200_i16, 400, 600, 400, 200, 800] {
            table.extend_from_slice(&value.to_be_bytes());
        }
        if variable {
            table.extend_from_slice(&u32::MAX.to_be_bytes());
        }
    }
    set_paint_offset(&mut table, brush_start)?;
    table.extend_from_slice(&[extend, 0, 2]);
    // Reverse the stops too. Inside the cone, OpenType still selects a
    // different intersection when the circles are exchanged.
    let indices = if reverse { [1_u16, 0] } else { [0, 1] };
    let offsets = if coincident {
        [8192_i16, if variable { 12288 } else { 8192 }]
    } else if shifted {
        [4096_i16, 20480]
    } else {
        [0, 16384]
    };
    for (i, (offset, index)) in offsets.into_iter().zip(indices).enumerate() {
        table.extend_from_slice(&offset.to_be_bytes());
        table.extend_from_slice(&index.to_be_bytes());
        table.extend_from_slice(&16384_i16.to_be_bytes());
        if variable {
            let base = if coincident && i == 1 { 6 } else { u32::MAX };
            table.extend_from_slice(&base.to_be_bytes());
        }
    }
    if variable {
        // The tip radius falls from 100 to zero at ALPH=0.5 and -100 at ALPH=1.
        // The coincident fixture also moves its last stop and reduces its alpha.
        let store_offset = u32::try_from(table.len())?.to_be_bytes();
        table[30..34].copy_from_slice(&store_offset);
        table.extend_from_slice(&[0, 1, 0, 0, 0, 12, 0, 1, 0, 0, 0, 22]);
        table.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0x40, 0, 0x40, 0]);
        let count = if coincident { 8 } else { 6 };
        table.extend_from_slice(&[0, count, 0, 1, 0, 1, 0, 0]);
        for index in 0..count {
            let delta = if index == (if reverse { 2 } else { 5 }) {
                -200_i16
            } else if index == 6 {
                -4096_i16
            } else if index == 7 {
                -8192_i16
            } else {
                0
            };
            table.extend_from_slice(&delta.to_be_bytes());
        }
    }
    let mut bytes = FIXTURE.to_vec();
    bytes.resize(bytes.len().next_multiple_of(4), 0);
    let offset = u32::try_from(bytes.len())?.to_be_bytes();
    let len = u32::try_from(table.len())?.to_be_bytes();
    bytes[record_start + 8..record_start + 12].copy_from_slice(&offset);
    bytes[record_start + 12..record_start + 16].copy_from_slice(&len);
    bytes.extend(table);
    Ok(GlyphFont::new(bytes.into()))
}

fn replace_brush(table: &mut [u8], bases: usize, glyph_id: u16) -> TestResult {
    let base_record = (bases + 4..bases + 4 + 6 * read_u32(table, bases)?)
        .step_by(6)
        .find(|&offset| table[offset..offset + 2] == glyph_id.to_be_bytes())
        .ok_or("missing base glyph")?;
    let clip_start = bases + read_u32(table, base_record + 2)?;
    assert_eq!(table[clip_start], 10); // PaintGlyph
    set_paint_offset(table, clip_start)
}

fn set_paint_offset(table: &mut [u8], paint_start: usize) -> TestResult {
    let offset = u32::try_from(table.len() - paint_start)?.to_be_bytes();
    table[paint_start + 1..paint_start + 4].copy_from_slice(&offset[1..]);
    Ok(())
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

#[test]
fn coincident_pad_stops_keep_radial_cone_and_hard_transition() -> TestResult {
    let gpu = RenderContext::new()?;
    let mut cpu = CpuRenderer;
    for (variable, locations) in [(false, &[0][..]), (true, &[0, 16384, 0, 16384])] {
        let font = gradient_font(RADIAL, 0, false, variable, true, true)?;
        for &location in locations {
            let (scene, root) = radial_scene(font.clone(), location)?;
            let reference = cpu.render(&scene, root, SIZE)?;
            let actual = gpu.render(&scene, root, SIZE)?;
            let blue = if variable && location == 16384 {
                [0, 0, 255, 128]
            } else {
                BLUE
            };
            for (x, y, expected) in [(40, 60, blue), (67, 60, RED), (60, 32, [0; 4])] {
                assert_eq!(sample(&reference, x, y), expected);
                let actual = sample(&actual, x, y);
                assert!(
                    actual
                        .into_iter()
                        .zip(expected)
                        .all(|(a, b)| a.abs_diff(b) <= 3),
                    "({x}, {y}) at location {location}, variable={variable}: CPU={expected:?}, GPU={actual:?}"
                );
            }
            if variable && location == 0 {
                let expected = sample(&reference, 48, 60);
                assert!(expected[0] > 0 && expected[2] > 0 && expected[3] == 255);
                let actual = sample(&actual, 48, 60);
                assert!(
                    actual
                        .into_iter()
                        .zip(expected)
                        .all(|(a, b)| a.abs_diff(b) <= 3),
                    "noncoincident location at (48, 60): CPU={expected:?}, GPU={actual:?}"
                );
            }
            if !variable || location == 16384 {
                let (scene, root) = glyph_scene(font.clone(), 3, location)?;
                let reference = cpu.render(&scene, root, SIZE)?;
                let actual = gpu.render(&scene, root, SIZE)?;
                for (x, expected) in [(30, RED), (55, blue)] {
                    assert_eq!(sample(&reference, x, 60), expected);
                    assert_eq!(
                        sample(&actual, x, 60),
                        expected,
                        "shared ColorLine at ({x}, 60)"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn coincident_pad_stops_keep_linear_and_sweep_transitions() -> TestResult {
    let gpu = RenderContext::new()?;
    let mut cpu = CpuRenderer;
    for (format, samples) in [
        (LINEAR, [(30, 60, RED), (55, 60, BLUE)]),
        (SWEEP, [(45, 42, RED), (45, 77, BLUE)]),
    ] {
        let (scene, root) = radial_scene(gradient_font(format, 0, false, false, true, true)?, 0)?;
        let reference = cpu.render(&scene, root, SIZE)?;
        let actual = gpu.render(&scene, root, SIZE)?;
        for (x, y, expected) in samples {
            assert_eq!(sample(&reference, x, y), expected);
            assert_eq!(
                sample(&actual, x, y),
                expected,
                "format {format} at ({x}, {y})"
            );
        }
    }
    Ok(())
}

fn read_u32(data: &[u8], offset: usize) -> TestResult<usize> {
    Ok(u32::from_be_bytes(data[offset..offset + 4].try_into()?) as usize)
}

fn sample(image: &RgbaImage, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * SIZE.width as usize + x) * 4;
    let mut pixel = [0; 4];
    pixel.copy_from_slice(&image.pixels()[offset..offset + 4]);
    pixel
}

fn radial_scene(font: GlyphFont, location: i16) -> TestResult<(Scene, oxiview_scene::NodeId)> {
    glyph_scene(font, 6, location)
}

fn glyph_scene(
    font: GlyphFont,
    glyph_id: u16,
    location: i16,
) -> TestResult<(Scene, oxiview_scene::NodeId)> {
    let glyph = Glyph {
        font,
        face_index: 0,
        glyph_id,
        size: 100.0,
        origin: Point::new(0.0, 100.0),
        color: Color::from_rgba8(0, 0, 0, 255),
        variations: Arc::from([location]),
        kind: GlyphKind::Color,
    };
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Draw(Primitive::Glyph(glyph)))?;
    Ok((scene, root))
}

#[test]
fn gpu_colr_radial_preserves_cone_and_color_order() -> TestResult {
    let gpu = RenderContext::new()?;
    let mut cpu = CpuRenderer;
    for extend in 0..=2 {
        for reverse in [false, true] {
            let mut fixed_reference = None;
            for (variable, locations) in [(false, &[0][..]), (true, &[0, 8192, 16384, 8192, 0])] {
                let font = radial_font(extend, reverse, variable, false)?;
                let mut prior_reference = None;
                for &location in locations {
                    let (scene, root) = radial_scene(font.clone(), location)?;
                    let reference = cpu.render(&scene, root, SIZE)?;
                    let actual = gpu.render(&scene, root, SIZE)?;
                    if variable && location == 8192 {
                        assert_eq!(
                            Some(reference.pixels()),
                            fixed_reference.as_deref(),
                            "the selected variable location must match the fixed zero radius"
                        );
                    }
                    for (x, y) in [(5, 30), (40, 30), (85, 60)] {
                        assert_eq!(sample(&reference, x, y), [0; 4]);
                        assert_eq!(
                            sample(&actual, x, y),
                            [0; 4],
                            "outside cone at ({x}, {y}), extend={extend}, reverse={reverse}, location={location}"
                        );
                    }
                    // Mid-cone and repeated-period samples, away from the
                    // clip edge, cone tip, and 8-bit ramp endpoints.
                    for (x, y) in [
                        (36, 60),
                        (42, 62),
                        (52, 62),
                        (62, 60),
                        (67, 65),
                        (72, 60),
                        (76, 67),
                    ] {
                        let expected = sample(&reference, x, y);
                        let actual = sample(&actual, x, y);
                        assert_eq!(expected[3], 255);
                        assert_eq!(actual[3], 255);
                        assert!(
                            actual
                                .into_iter()
                                .zip(expected)
                                .all(|(a, b)| a.abs_diff(b) <= 3),
                            "({x}, {y}) extend={extend}, reverse={reverse}, location={location}: CPU={expected:?}, GPU={actual:?}"
                        );
                    }
                    let color = sample(&reference, 36, 60);
                    if let Some(prior) = prior_reference {
                        assert_ne!(color, prior, "the selected coordinate must change geometry");
                    }
                    prior_reference = Some(color);
                    if !variable {
                        fixed_reference = Some(reference.pixels().to_vec());
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn shifted_color_line_and_brush_affine_keep_the_same_cone() -> TestResult {
    let gpu = RenderContext::new()?;
    let mut cpu = CpuRenderer;
    for extend in 0..=2 {
        let (scene, root) = radial_scene(radial_font(extend, false, false, true)?, 0)?;
        let reference = cpu.render(&scene, root, SIZE)?;
        let actual = gpu.render(&scene, root, SIZE)?;
        for (x, y) in [(44, 27), (81, 60)] {
            assert_eq!(sample(&reference, x, y), [0; 4]);
            assert_eq!(sample(&actual, x, y), [0; 4]);
        }
        for (x, y) in [(41, 60), (47, 62), (57, 62), (67, 60), (72, 65), (77, 60)] {
            let expected = sample(&reference, x, y);
            let actual = sample(&actual, x, y);
            assert_eq!(expected[3], 255);
            assert_eq!(actual[3], 255);
            assert!(
                actual
                    .into_iter()
                    .zip(expected)
                    .all(|(a, b)| a.abs_diff(b) <= 3),
                "({x}, {y}) extend={extend}: CPU={expected:?}, GPU={actual:?}"
            );
        }
    }
    Ok(())
}
