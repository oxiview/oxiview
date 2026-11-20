//! Recover source gradients whose Pad stops coincide after variation.

use super::{
    glyph_error,
    normalize::{Deltas, Line},
};
use crate::{Error, vector::invalid};
use skrifa::{
    FontRef, GlyphId,
    color::{Brush, Extend},
    instance::NormalizedCoord,
    raw::{
        FontData, FontRead, TableProvider,
        tables::colr::{ColorStop, Paint, VarColorStop},
        types::{F2Dot14, FWord, UfWord},
    },
};

// The maximum 24-bit ID is outside even an extended maxp glyph count.
// A private PaintGlyph2 wrapping PaintSolid carries the original paint
// offset through Skrifa's fill_glyph callback in the solid's palette and
// alpha fields. Shared ColorLines and the graph's transforms and clips are
// kept intact.
pub(super) const MARKER_GLYPH: GlyphId = GlyphId::new(0xff_ffff);
pub(super) const MARKER_LEN: usize = 12;

pub(super) fn marker(data: FontData<'_>, colr: &[u8]) -> Result<[u8; MARKER_LEN], Error> {
    let offset = data
        .as_bytes()
        .as_ptr()
        .addr()
        .checked_sub(colr.as_ptr().addr())
        .filter(|offset| colr.len().saturating_sub(*offset) >= MARKER_LEN)
        .and_then(|offset| u32::try_from(offset).ok())
        .ok_or_else(|| invalid("COLR source paint offset is out of range"))?;
    let mut bytes = [33, 0, 0, 7, 0xff, 0xff, 0xff, 2, 0, 0, 0, 0];
    bytes[8..].copy_from_slice(&offset.to_be_bytes());
    Ok(bytes)
}

pub(super) fn coincident(line: &Line<'_>, deltas: &Deltas<'_>) -> (bool, bool) {
    match line {
        Line::Fixed(line) if line.extend() == Extend::Pad => {
            let stops = line.color_stops();
            let same = stops.len() >= 2
                && stops
                    .iter()
                    .all(|stop| stop.stop_offset() == stops[0].stop_offset());
            (same, false)
        }
        Line::Variable(line) if line.extend() == Extend::Pad => {
            let [first, rest @ ..] = line.color_stops() else {
                return (false, false);
            };
            if rest.is_empty() {
                return (false, false);
            }
            // Record the location dependency even when this location has a
            // nonzero span. The same stops can coincide in another instance.
            let varies = line
                .color_stops()
                .iter()
                .any(|stop| deltas.varies(stop.var_index_base()));
            let offset = first
                .stop_offset()
                .apply_delta(deltas.get(first.var_index_base(), 0));
            let same = rest.iter().all(|stop| {
                offset
                    == stop
                        .stop_offset()
                        .apply_delta(deltas.get(stop.var_index_base(), 0))
            });
            (same, varies)
        }
        _ => (false, false),
    }
}

pub(super) struct Source<'a> {
    colr: &'a [u8],
    deltas: Deltas<'a>,
}

pub(super) enum Geometry {
    Linear([f64; 6]),
    Radial([f64; 6]),
    Sweep([f64; 4]),
}

pub(super) struct Stop {
    pub palette_index: u16,
    pub alpha: f32,
}

pub(super) struct Transition {
    pub geometry: Geometry,
    pub offset: f64,
    pub before: Stop,
    pub after: Stop,
}

impl<'a> Source<'a> {
    pub(super) fn new(
        font: &'a [u8],
        face_index: u32,
        coords: &'a [NormalizedCoord],
    ) -> Result<Self, Error> {
        let face = FontRef::from_index(font, face_index).map_err(glyph_error)?;
        let colr = face.colr().map_err(glyph_error)?;
        Ok(Self {
            colr: colr.offset_data().as_bytes(),
            deltas: Deltas::new(&colr, coords),
        })
    }

    pub(super) fn read(&self, marker: Brush<'_>, palette_len: usize) -> Result<Transition, Error> {
        let Brush::Solid {
            palette_index,
            alpha,
        } = marker
        else {
            return Err(invalid("invalid COLR source paint marker"));
        };
        let offset =
            (u32::from(palette_index) << 16) | u32::from(F2Dot14::from_f32(alpha).to_bits() as u16);
        let data = self
            .colr
            .get(offset as usize..)
            .ok_or_else(|| invalid("invalid COLR source paint offset"))?;
        let paint = Paint::read(FontData::new(data)).map_err(glyph_error)?;

        macro_rules! fixed {
            ($paint:ident, $geometry:ident, $values:ident) => {{
                let line = Line::Fixed($paint.color_line().map_err(glyph_error)?);
                (Geometry::$geometry($values!($paint, u32::MAX)), line)
            }};
        }
        macro_rules! variable {
            ($paint:ident, $geometry:ident, $values:ident) => {{
                let line = Line::Variable($paint.color_line().map_err(glyph_error)?);
                (
                    Geometry::$geometry($values!($paint, $paint.var_index_base())),
                    line,
                )
            }};
        }
        let units = |word: FWord, base, index| {
            f64::from(word.to_i16()) + self.deltas.get(base, index).to_f64()
        };
        let radius = |word: UfWord, base, index| {
            f64::from(word.to_u16()) + self.deltas.get(base, index).to_f64()
        };
        let angle = |word: F2Dot14, base, index| {
            (f64::from(word.to_f32()) + self.deltas.get(base, index).to_f64() / 16384.0 + 1.0)
                * std::f64::consts::PI
        };
        macro_rules! linear {
            ($p:ident, $base:expr) => {
                [
                    units($p.x0(), $base, 0),
                    units($p.y0(), $base, 1),
                    units($p.x1(), $base, 2),
                    units($p.y1(), $base, 3),
                    units($p.x2(), $base, 4),
                    units($p.y2(), $base, 5),
                ]
            };
        }
        macro_rules! radial {
            ($p:ident, $base:expr) => {
                [
                    units($p.x0(), $base, 0),
                    units($p.y0(), $base, 1),
                    radius($p.radius0(), $base, 2),
                    units($p.x1(), $base, 3),
                    units($p.y1(), $base, 4),
                    radius($p.radius1(), $base, 5),
                ]
            };
        }
        macro_rules! sweep {
            ($p:ident, $base:expr) => {
                [
                    units($p.center_x(), $base, 0),
                    units($p.center_y(), $base, 1),
                    angle($p.start_angle(), $base, 2),
                    angle($p.end_angle(), $base, 3),
                ]
            };
        }
        let (geometry, line) = match paint {
            Paint::LinearGradient(p) => fixed!(p, Linear, linear),
            Paint::VarLinearGradient(p) => variable!(p, Linear, linear),
            Paint::RadialGradient(p) => fixed!(p, Radial, radial),
            Paint::VarRadialGradient(p) => variable!(p, Radial, radial),
            Paint::SweepGradient(p) => fixed!(p, Sweep, sweep),
            Paint::VarSweepGradient(p) => variable!(p, Sweep, sweep),
            _ => return Err(invalid("COLR source paint is not a gradient")),
        };
        let valid = |index| index == u16::MAX || usize::from(index) < palette_len;
        // With all offsets equal, encoded order determines the two Pad
        // colors. Intermediate stops have no interval in which to paint.
        macro_rules! endpoints {
            ($line:ident, $stop:ident : $ty:ty, $offset:expr, $alpha:expr) => {{
                let stops = $line.color_stops();
                let [first, .., last] = stops else {
                    return Err(invalid("invalid COLR hard stop"));
                };
                if !stops.iter().all(|stop| valid(stop.palette_index())) {
                    return Err(invalid("invalid COLR palette index"));
                }
                let $stop = first;
                let offset = $offset;
                let endpoint = |$stop: &$ty| Stop {
                    palette_index: $stop.palette_index(),
                    alpha: $alpha,
                };
                (offset, endpoint(first), endpoint(last))
            }};
        }
        let (offset, before, after) = match line {
            Line::Fixed(line) => endpoints!(
                line,
                stop: ColorStop,
                stop.stop_offset().to_f32(),
                stop.alpha().to_f32()
            ),
            Line::Variable(line) => endpoints!(
                line,
                stop: VarColorStop,
                stop.stop_offset().apply_delta(self.deltas.get(stop.var_index_base(), 0)),
                stop.alpha().apply_delta(self.deltas.get(stop.var_index_base(), 1))
            ),
        };
        Ok(Transition {
            geometry,
            offset: f64::from(offset),
            before,
            after,
        })
    }
}
