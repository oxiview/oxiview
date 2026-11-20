//! Prepare exceptional color lines for Skrifa's brush callbacks. Skrifa still
//! traverses the paint graph and resolves transforms, clips and ordinary brushes.

use super::{glyph_error, hard_stop};
use crate::{Error, vector::invalid};
use skrifa::{
    FontRef,
    color::Extend,
    instance::NormalizedCoord,
    raw::{
        FontData, TableProvider,
        tables::{
            colr::{ColorLine, Colr, Paint, VarColorLine},
            variations::{DeltaSetIndex, DeltaSetIndexMap, ItemVariationStore},
        },
        types::{F2Dot14, F48Dot16},
    },
};
use std::collections::HashSet;

pub(super) fn gradients(
    face: &FontRef<'_>,
    coords: &[NormalizedCoord],
    original: &[u8],
    adjusted: &mut Option<Vec<u8>>,
) -> Result<bool, Error> {
    let colr = face.colr().map_err(glyph_error)?;
    let Some(Ok(bases)) = colr.base_glyph_list() else {
        return Ok(false);
    };
    let colr_bytes = colr.offset_data().as_bytes();
    let deltas = Deltas::new(&colr, coords);
    let mut pending: Vec<_> = bases
        .base_glyph_paint_records()
        .iter()
        .filter_map(|record| record.paint(bases.offset_data()).ok())
        .collect();
    if let Some(Ok(layers)) = colr.layer_list() {
        pending.extend(layers.paints().iter().filter_map(Result::ok));
    }

    // Base and layer records are all seeded above, so only inline child
    // offsets need following. Ignore unreadable unrelated paints here.
    // Skrifa reports errors when it traverses the requested glyph.
    let mut seen = HashSet::new();
    let mut varies = false;
    macro_rules! descend {
        ($paint:ident) => {{
            pending.extend($paint.paint().ok());
            None
        }};
    }
    while let Some(paint) = pending.pop() {
        if !seen.insert(paint.offset_data().as_bytes().as_ptr()) {
            continue;
        }
        let line = match &paint {
            Paint::LinearGradient(p) => p.color_line().ok().map(Line::Fixed),
            Paint::RadialGradient(p) => p.color_line().ok().map(Line::Fixed),
            Paint::VarLinearGradient(p) => p.color_line().ok().map(Line::Variable),
            Paint::VarRadialGradient(p) => p.color_line().ok().map(Line::Variable),
            Paint::SweepGradient(p) => match p.color_line() {
                Ok(line)
                    if line.num_stops() == 1
                        && repeating(line.extend())
                        && p.start_angle() == p.end_angle() =>
                {
                    transparent_paint(p.offset_data(), original, adjusted)?;
                    None
                }
                line => line.ok().map(Line::Fixed),
            },
            Paint::VarSweepGradient(p) => {
                let Ok(line) = p.color_line() else {
                    continue;
                };
                let base = p.var_index_base();
                let singleton = line.num_stops() == 1 && repeating(line.extend());
                varies |= singleton && deltas.varies(base);
                if singleton
                    && p.start_angle().apply_delta(deltas.get(base, 2))
                        == p.end_angle().apply_delta(deltas.get(base, 3))
                {
                    transparent_paint(p.offset_data(), original, adjusted)?;
                    None
                } else {
                    Some(Line::Variable(line))
                }
            }
            Paint::Composite(p) => {
                pending.extend(p.backdrop_paint().ok());
                pending.extend(p.source_paint().ok());
                None
            }
            Paint::Glyph(p) => descend!(p),
            Paint::Glyph2(p) => descend!(p),
            Paint::Transform(p) => descend!(p),
            Paint::VarTransform(p) => descend!(p),
            Paint::Translate(p) => descend!(p),
            Paint::VarTranslate(p) => descend!(p),
            Paint::Scale(p) => descend!(p),
            Paint::VarScale(p) => descend!(p),
            Paint::ScaleAroundCenter(p) => descend!(p),
            Paint::VarScaleAroundCenter(p) => descend!(p),
            Paint::ScaleUniform(p) => descend!(p),
            Paint::VarScaleUniform(p) => descend!(p),
            Paint::ScaleUniformAroundCenter(p) => descend!(p),
            Paint::VarScaleUniformAroundCenter(p) => descend!(p),
            Paint::Rotate(p) => descend!(p),
            Paint::VarRotate(p) => descend!(p),
            Paint::RotateAroundCenter(p) => descend!(p),
            Paint::VarRotateAroundCenter(p) => descend!(p),
            Paint::Skew(p) => descend!(p),
            Paint::VarSkew(p) => descend!(p),
            Paint::SkewAroundCenter(p) => descend!(p),
            Paint::VarSkewAroundCenter(p) => descend!(p),
            _ => None,
        };
        if let Some(line) = line {
            varies |= normalize_line(
                paint.offset_data(),
                line,
                &deltas,
                colr_bytes,
                original,
                adjusted,
            )?;
        }
    }
    Ok(varies)
}

fn repeating(extend: Extend) -> bool {
    matches!(extend, Extend::Repeat | Extend::Reflect)
}

pub(super) enum Line<'a> {
    Fixed(ColorLine<'a>),
    Variable(VarColorLine<'a>),
}

fn normalize_line(
    paint: FontData<'_>,
    line: Line<'_>,
    deltas: &Deltas<'_>,
    colr: &[u8],
    original: &[u8],
    adjusted: &mut Option<Vec<u8>>,
) -> Result<bool, Error> {
    let (coincident, varies) = hard_stop::coincident(&line, deltas);
    if coincident {
        let bytes = patch(paint, hard_stop::MARKER_LEN, original, adjusted)?;
        let marker = hard_stop::marker(paint, colr)?;
        bytes.copy_from_slice(&marker);
        return Ok(varies);
    }

    // A singleton is constant over the gradient's defined domain. Pad and
    // offset zero make Skrifa emit two identical stops at 0 and 1 with the
    // original geometry. This matters for the limited coverage of a radial.
    let (data, offset, variable) = match line {
        Line::Fixed(line) => {
            let [stop] = line.color_stops() else {
                return Ok(varies);
            };
            if line.extend() == Extend::Pad && stop.stop_offset() == F2Dot14::ZERO {
                return Ok(varies);
            }
            (
                line.offset_data(),
                line.color_stops_byte_range().start,
                None,
            )
        }
        Line::Variable(line) => {
            let [stop] = line.color_stops() else {
                return Ok(varies);
            };
            let alpha = stop
                .alpha()
                .apply_delta(deltas.get(stop.var_index_base(), 1));
            (
                line.offset_data(),
                line.color_stops_byte_range().start,
                Some((
                    F2Dot14::from_f32(alpha.clamp(0.0, 1.0)),
                    deltas.varies(stop.var_index_base()),
                )),
            )
        }
    };
    let line = patch(
        data,
        offset + if variable.is_some() { 10 } else { 6 },
        original,
        adjusted,
    )?;
    line[0] = Extend::Pad as u8;
    // OpenType (Var)ColorStop begins with F2Dot14 stopOffset, then
    // uint16 paletteIndex and F2Dot14 alpha, followed by VarIndexBase.
    line[offset..offset + 2].fill(0);
    if let Some((alpha, _)) = variable {
        line[offset + 4..offset + 6].copy_from_slice(&alpha.to_bits().to_be_bytes());
        line[offset + 6..offset + 10].copy_from_slice(&u32::MAX.to_be_bytes());
    }
    Ok(variable.is_some_and(|(_, varies)| varies))
}

fn transparent_paint(
    data: FontData<'_>,
    original: &[u8],
    adjusted: &mut Option<Vec<u8>>,
) -> Result<(), Error> {
    // Rewrite the paint itself since its color line can be shared.
    const TRANSPARENT_SOLID: [u8; 5] = [2, 0xff, 0xff, 0, 0]; // PaintSolid, foreground, zero alpha.
    patch(data, TRANSPARENT_SOLID.len(), original, adjusted)?.copy_from_slice(&TRANSPARENT_SOLID);
    Ok(())
}

fn patch<'a>(
    data: FontData<'_>,
    len: usize,
    original: &[u8],
    adjusted: &'a mut Option<Vec<u8>>,
) -> Result<&'a mut [u8], Error> {
    let start = data
        .as_bytes()
        .as_ptr()
        .addr()
        .checked_sub(original.as_ptr().addr())
        .filter(|&start| len <= data.len() && len <= original.len().saturating_sub(start))
        .ok_or_else(|| invalid("COLR normalization exceeds font data"))?;
    Ok(&mut adjusted.get_or_insert_with(|| original.to_vec())[start..start + len])
}

pub(super) struct Deltas<'a> {
    coords: &'a [NormalizedCoord],
    map: Option<DeltaSetIndexMap<'a>>,
    store: Option<ItemVariationStore<'a>>,
}

impl<'a> Deltas<'a> {
    pub(super) fn new(colr: &Colr<'a>, coords: &'a [NormalizedCoord]) -> Self {
        Self {
            coords,
            map: colr.var_index_map().and_then(Result::ok),
            store: colr.item_variation_store().and_then(Result::ok),
        }
    }

    pub(super) fn varies(&self, base: u32) -> bool {
        self.store.is_some() && base != u32::MAX
    }

    pub(super) fn get(&self, base: u32, offset: u32) -> F48Dot16 {
        if self.coords.is_empty() {
            return F48Dot16::ZERO;
        }
        let Some(index) = base.checked_add(offset).filter(|&i| i != u32::MAX) else {
            return F48Dot16::ZERO;
        };
        let delta_index = match &self.map {
            Some(map) => map.get(index).ok(),
            None => Some(DeltaSetIndex {
                outer: (index >> 16) as u16,
                inner: index as u16,
            }),
        };
        delta_index
            .and_then(|index| self.store.as_ref()?.compute_delta(index, self.coords))
            .unwrap_or_default()
    }
}
