//! Resolve Fontations' typed COLR paint tables at the glyph's variation location.
//! Skrifa's high-level brush callbacks normalize stop ranges and omit some
//! degenerate lines. Replay needs the original stops and geometry.

use super::{Command, Gradient, Graph, Paint, Pixel, invalid, outline};
use crate::Error;
use skrifa::{
    FontRef, GlyphId, MetadataProvider,
    color::{ColorGlyphFormat, CompositeMode, Extend},
    instance::{LocationRef, Size},
    raw::{
        TableProvider,
        tables::{
            colr::{ColorLine, Colr, Paint as FontPaint, VarColorLine},
            variations::{DeltaSetIndex, DeltaSetIndexMap, ItemVariationStore},
        },
    },
};
use vello_cpu::{
    kurbo::{Affine, Point, Rect, Shape, Vec2},
    peniko::{BlendMode, Compose, Mix},
};

pub(super) fn record(
    graph: &mut Graph<'_>,
    face: &FontRef<'_>,
    id: GlyphId,
    location: LocationRef<'_>,
    placement: Affine,
) -> Result<(), Error> {
    let colr = face.colr().map_err(invalid)?;
    let reader = Reader {
        face,
        location,
        index_map: colr.var_index_map().transpose().map_err(invalid)?,
        store: colr.item_variation_store().transpose().map_err(invalid)?,
        colr,
    };
    if reader.colr.v1_base_glyph(id).is_some() {
        reader.glyph(graph, id, placement, 0)
    } else {
        let layers = reader
            .colr
            .v0_base_glyph(id)
            .ok_or_else(|| invalid("COLR glyph is missing"))?;
        for layer in layers {
            let (id, index) = reader
                .colr
                .v0_layer(layer)
                .ok_or_else(|| invalid("invalid COLRv0 layer"))?;
            reader.glyph_clip(graph, id.into(), placement, |graph| {
                graph.fill(Paint::Solid(graph.color(index, 1.0)?));
                Ok(())
            })?;
        }
        Ok(())
    }
}

struct Reader<'a> {
    face: &'a FontRef<'a>,
    location: LocationRef<'a>,
    colr: Colr<'a>,
    index_map: Option<DeltaSetIndexMap<'a>>,
    store: Option<ItemVariationStore<'a>>,
}

const F2DOT14: f64 = 16_384.0;
const FIXED: f64 = 65_536.0;
const NO_VARIATION: u32 = u32::MAX;

// Geometry fields are in design units; angles, scales, offsets and alpha
// are F2DOT14. Variation deltas are in units of the field's least bit.
macro_rules! units {
    ($table:ident, $($field:ident),+) => {
        [$(f64::from($table.$field().to_i16())),+]
    };
}
macro_rules! fixed_values {
    ($table:ident, $($field:ident),+) => {
        [$(f64::from($table.$field().to_f32())),+]
    };
}

impl Reader<'_> {
    fn delta(&self, base: u32, offset: u32) -> f64 {
        let Some(index) = base
            .checked_add(offset)
            .filter(|index| *index != NO_VARIATION)
        else {
            return 0.0;
        };
        let Some(store) = &self.store else { return 0.0 };
        let index = match &self.index_map {
            Some(map) => map.get(index).ok(),
            None => Some(DeltaSetIndex {
                outer: (index >> 16) as u16,
                inner: index as u16,
            }),
        };
        index
            .and_then(|index| store.compute_delta(index, self.location.coords()))
            .map_or(0.0, |delta| f64::from(delta.to_f32()))
    }

    fn geometry<const N: usize>(
        &self,
        values: [f64; N],
        base: u32,
        divisors: [f64; N],
    ) -> [f64; N] {
        std::array::from_fn(|i| values[i] + self.delta(base, i as u32) / divisors[i])
    }

    fn glyph_clip(
        &self,
        graph: &mut Graph<'_>,
        id: GlyphId,
        transform: Affine,
        draw: impl FnOnce(&mut Graph<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        graph.clip(transform * outline(self.face, id, self.location)?, draw)
    }

    fn glyph(
        &self,
        graph: &mut Graph<'_>,
        id: GlyphId,
        transform: Affine,
        depth: usize,
    ) -> Result<(), Error> {
        let (paint, _) = self
            .colr
            .v1_base_glyph(id)
            .ok_or_else(|| invalid("invalid COLRv1 glyph reference"))?;
        let clip = self
            .face
            .color_glyphs()
            .get_with_format(id, ColorGlyphFormat::ColrV1)
            .and_then(|glyph| glyph.bounding_box(self.location, Size::unscaled()));
        if let Some(bounds) = clip {
            let rect = Rect::new(
                bounds.x_min.into(),
                bounds.y_min.into(),
                bounds.x_max.into(),
                bounds.y_max.into(),
            );
            graph.clip(transform * rect.to_path(0.1), |graph| {
                self.paint(graph, paint, transform, depth)
            })
        } else {
            self.paint(graph, paint, transform, depth)
        }
    }

    fn paint(
        &self,
        graph: &mut Graph<'_>,
        paint: FontPaint<'_>,
        transform: Affine,
        depth: usize,
    ) -> Result<(), Error> {
        // Bound recursion and expansion of shared subgraphs, including nodes
        // that emit no drawing commands.
        graph.charge(16)?;
        if depth >= 64 {
            return Err(invalid("COLRv1 paint graph exceeds replay limits"));
        }
        if !transform.as_coeffs().into_iter().all(f64::is_finite) {
            return Err(invalid("non-finite COLR transform"));
        }
        let depth = depth + 1;
        macro_rules! child {
            ($paint:ident, $local:expr) => {
                self.paint(
                    graph,
                    $paint.paint().map_err(invalid)?,
                    transform * $local,
                    depth,
                )
            };
        }
        macro_rules! shift {
            ($paint:ident, $values:expr, $divisors:expr) => {
                self.geometry($values, $paint.var_index_base(), $divisors)
            };
        }
        macro_rules! clipped {
            ($paint:ident) => {
                self.glyph_clip(graph, $paint.glyph_id().into(), transform, |graph| {
                    self.paint(graph, $paint.paint().map_err(invalid)?, transform, depth)
                })
            };
        }
        macro_rules! radial_values {
            ($p:ident) => {{
                let [x0, y0, x1, y1] = units!($p, x0, y0, x1, y1);
                [
                    x0,
                    y0,
                    $p.radius0().to_u16().into(),
                    x1,
                    y1,
                    $p.radius1().to_u16().into(),
                ]
            }};
        }
        macro_rules! sweep_values {
            ($p:ident) => {{
                let [x, y] = units!($p, center_x, center_y);
                let [start, end] = fixed_values!($p, start_angle, end_angle);
                [x, y, start, end]
            }};
        }

        match paint {
            FontPaint::ColrLayers(p) => {
                let start = p.first_layer_index() as usize;
                let end = start
                    .checked_add(p.num_layers().into())
                    .ok_or_else(|| invalid("invalid COLRv1 layer range"))?;
                for index in start..end {
                    let (child, _) = self
                        .colr
                        .v1_layer(index)
                        .ok_or_else(|| invalid("invalid COLRv1 layer"))?;
                    self.paint(graph, child, transform, depth)?;
                }
                Ok(())
            }
            FontPaint::Solid(p) => {
                graph.fill(Paint::Solid(
                    graph.color(p.palette_index(), p.alpha().to_f32())?,
                ));
                Ok(())
            }
            FontPaint::VarSolid(p) => {
                let [alpha] = shift!(p, fixed_values!(p, alpha), [F2DOT14]);
                graph.fill(Paint::Solid(graph.color(p.palette_index(), alpha as f32)?));
                Ok(())
            }
            FontPaint::LinearGradient(p) => {
                let geometry = linear(units!(p, x0, y0, x1, y1, x2, y2));
                self.gradient(
                    graph,
                    transform,
                    geometry,
                    Line::Fixed(p.color_line().map_err(invalid)?),
                )
            }
            FontPaint::VarLinearGradient(p) => {
                let geometry = linear(shift!(p, units!(p, x0, y0, x1, y1, x2, y2), [1.0; 6]));
                self.gradient(
                    graph,
                    transform,
                    geometry,
                    Line::Variable(p.color_line().map_err(invalid)?),
                )
            }
            FontPaint::RadialGradient(p) => self.gradient(
                graph,
                transform,
                Some(radial(radial_values!(p))),
                Line::Fixed(p.color_line().map_err(invalid)?),
            ),
            FontPaint::VarRadialGradient(p) => self.gradient(
                graph,
                transform,
                Some(radial(shift!(p, radial_values!(p), [1.0; 6]))),
                Line::Variable(p.color_line().map_err(invalid)?),
            ),
            FontPaint::SweepGradient(p) => self.gradient(
                graph,
                transform,
                Some(sweep(sweep_values!(p))),
                Line::Fixed(p.color_line().map_err(invalid)?),
            ),
            FontPaint::VarSweepGradient(p) => self.gradient(
                graph,
                transform,
                Some(sweep(shift!(
                    p,
                    sweep_values!(p),
                    [1.0, 1.0, F2DOT14, F2DOT14]
                ))),
                Line::Variable(p.color_line().map_err(invalid)?),
            ),
            FontPaint::Glyph(p) => clipped!(p),
            FontPaint::Glyph2(p) => clipped!(p),
            FontPaint::ColrGlyph(p) => self.glyph(graph, p.glyph_id().into(), transform, depth),
            FontPaint::Transform(p) => {
                let a = p.transform().map_err(invalid)?;
                child!(p, Affine::new(fixed_values!(a, xx, yx, xy, yy, dx, dy)))
            }
            FontPaint::VarTransform(p) => {
                let a = p.transform().map_err(invalid)?;
                let matrix = shift!(a, fixed_values!(a, xx, yx, xy, yy, dx, dy), [FIXED; 6]);
                child!(p, Affine::new(matrix))
            }
            FontPaint::Translate(p) => {
                let [dx, dy] = units!(p, dx, dy);
                child!(p, Affine::translate((dx, dy)))
            }
            FontPaint::VarTranslate(p) => {
                let [dx, dy] = shift!(p, units!(p, dx, dy), [1.0; 2]);
                child!(p, Affine::translate((dx, dy)))
            }
            FontPaint::Scale(p) => {
                let [sx, sy] = fixed_values!(p, scale_x, scale_y);
                child!(p, Affine::scale_non_uniform(sx, sy))
            }
            FontPaint::VarScale(p) => {
                let [sx, sy] = shift!(p, fixed_values!(p, scale_x, scale_y), [F2DOT14; 2]);
                child!(p, Affine::scale_non_uniform(sx, sy))
            }
            FontPaint::ScaleAroundCenter(p) => {
                let [sx, sy] = fixed_values!(p, scale_x, scale_y);
                let [x, y] = units!(p, center_x, center_y);
                child!(p, around(Affine::scale_non_uniform(sx, sy), x, y))
            }
            FontPaint::VarScaleAroundCenter(p) => {
                let [sx, sy] = fixed_values!(p, scale_x, scale_y);
                let [x, y] = units!(p, center_x, center_y);
                let [sx, sy, x, y] = shift!(p, [sx, sy, x, y], [F2DOT14, F2DOT14, 1.0, 1.0]);
                child!(p, around(Affine::scale_non_uniform(sx, sy), x, y))
            }
            FontPaint::ScaleUniform(p) => {
                let [s] = fixed_values!(p, scale);
                child!(p, Affine::scale(s))
            }
            FontPaint::VarScaleUniform(p) => {
                let [s] = shift!(p, fixed_values!(p, scale), [F2DOT14]);
                child!(p, Affine::scale(s))
            }
            FontPaint::ScaleUniformAroundCenter(p) => {
                let [s] = fixed_values!(p, scale);
                let [x, y] = units!(p, center_x, center_y);
                child!(p, around(Affine::scale(s), x, y))
            }
            FontPaint::VarScaleUniformAroundCenter(p) => {
                let [s] = fixed_values!(p, scale);
                let [x, y] = units!(p, center_x, center_y);
                let [s, x, y] = shift!(p, [s, x, y], [F2DOT14, 1.0, 1.0]);
                child!(p, around(Affine::scale(s), x, y))
            }
            FontPaint::Rotate(p) => {
                let [angle] = fixed_values!(p, angle);
                child!(p, rotation(angle))
            }
            FontPaint::VarRotate(p) => {
                let [angle] = shift!(p, fixed_values!(p, angle), [F2DOT14]);
                child!(p, rotation(angle))
            }
            FontPaint::RotateAroundCenter(p) => {
                let [angle] = fixed_values!(p, angle);
                let [x, y] = units!(p, center_x, center_y);
                child!(p, around(rotation(angle), x, y))
            }
            FontPaint::VarRotateAroundCenter(p) => {
                let [angle] = fixed_values!(p, angle);
                let [x, y] = units!(p, center_x, center_y);
                let [angle, x, y] = shift!(p, [angle, x, y], [F2DOT14, 1.0, 1.0]);
                child!(p, around(rotation(angle), x, y))
            }
            FontPaint::Skew(p) => {
                let [ax, ay] = fixed_values!(p, x_skew_angle, y_skew_angle);
                child!(p, skew(ax, ay))
            }
            FontPaint::VarSkew(p) => {
                let [ax, ay] = shift!(
                    p,
                    fixed_values!(p, x_skew_angle, y_skew_angle),
                    [F2DOT14; 2]
                );
                child!(p, skew(ax, ay))
            }
            FontPaint::SkewAroundCenter(p) => {
                let [ax, ay] = fixed_values!(p, x_skew_angle, y_skew_angle);
                let [x, y] = units!(p, center_x, center_y);
                child!(p, around(skew(ax, ay), x, y))
            }
            FontPaint::VarSkewAroundCenter(p) => {
                let [ax, ay] = fixed_values!(p, x_skew_angle, y_skew_angle);
                let [x, y] = units!(p, center_x, center_y);
                let [ax, ay, x, y] = shift!(p, [ax, ay, x, y], [F2DOT14, F2DOT14, 1.0, 1.0]);
                child!(p, around(skew(ax, ay), x, y))
            }
            FontPaint::Composite(p) => {
                graph.commands.push(Command::Layer);
                self.paint(
                    graph,
                    p.backdrop_paint().map_err(invalid)?,
                    transform,
                    depth,
                )?;
                graph.commands.push(Command::Layer);
                self.paint(graph, p.source_paint().map_err(invalid)?, transform, depth)?;
                graph
                    .commands
                    .push(Command::EndLayer(blend_mode(p.composite_mode())));
                graph.commands.push(Command::EndLayer(BlendMode::default()));
                Ok(())
            }
        }
    }

    fn gradient(
        &self,
        graph: &mut Graph<'_>,
        transform: Affine,
        geometry: Option<Gradient>,
        color_line: Line<'_>,
    ) -> Result<(), Error> {
        let (extend, mut stops) = match color_line {
            Line::Fixed(line) => {
                graph.charge(line.color_stops().len())?;
                let stops = line
                    .color_stops()
                    .iter()
                    .map(|stop| {
                        Ok((
                            f64::from(stop.stop_offset().to_f32()),
                            graph.color(stop.palette_index(), stop.alpha().to_f32())?,
                        ))
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                (line.extend(), stops)
            }
            Line::Variable(line) => {
                graph.charge(line.color_stops().len())?;
                let stops = line
                    .color_stops()
                    .iter()
                    .map(|stop| {
                        let [offset, alpha] = self.geometry(
                            [
                                stop.stop_offset().to_f32().into(),
                                stop.alpha().to_f32().into(),
                            ],
                            stop.var_index_base(),
                            [F2DOT14; 2],
                        );
                        Ok((offset, graph.color(stop.palette_index(), alpha as f32)?))
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                (line.extend(), stops)
            }
        };
        let Some(geometry) = geometry.filter(|_| transform.determinant() != 0.0) else {
            return Ok(());
        };
        if stops.is_empty() {
            return Ok(());
        }
        if matches!(geometry, Gradient::Sweep { start, end, .. } if start == end)
            && matches!(extend, Extend::Repeat | Extend::Reflect)
        {
            return Ok(());
        }
        // Stable ordering preserves the encoded order of hard stops even when
        // two variable offsets become equal after applying their deltas.
        stops.sort_by(|(left, _): &(f64, Pixel), (right, _)| left.total_cmp(right));
        let inverse = transform.inverse();
        if !inverse.as_coeffs().into_iter().all(f64::is_finite) {
            return Err(invalid("non-finite COLR paint transform"));
        }
        graph.fill(Paint::Gradient {
            inverse,
            geometry,
            stops,
            extend,
        });
        Ok(())
    }
}

enum Line<'a> {
    Fixed(ColorLine<'a>),
    Variable(VarColorLine<'a>),
}

fn around(transform: Affine, x: f64, y: f64) -> Affine {
    let center = Vec2::new(x, y);
    Affine::translate(center) * transform * Affine::translate(-center)
}

fn rotation(angle: f64) -> Affine {
    Affine::rotate(angle * std::f64::consts::PI)
}

fn skew(x: f64, y: f64) -> Affine {
    Affine::skew(
        -(x * std::f64::consts::PI).tan(),
        (y * std::f64::consts::PI).tan(),
    )
}

fn linear([x0, y0, x1, y1, x2, y2]: [f64; 6]) -> Option<Gradient> {
    let start = Point::new(x0, y0);
    let axis = Point::new(x1, y1) - start;
    let projection = Vec2::new(y2 - y0, x0 - x2);
    let denominator = axis.dot(projection);
    (denominator != 0.0).then(|| Gradient::Linear {
        start,
        projection: projection / denominator,
    })
}

fn radial([x0, y0, r0, x1, y1, r1]: [f64; 6]) -> Gradient {
    Gradient::Radial {
        start: Point::new(x0, y0),
        delta: Vec2::new(x1 - x0, y1 - y0),
        radius: r0,
        radius_delta: r1 - r0,
    }
}

fn sweep([x, y, start, end]: [f64; 4]) -> Gradient {
    Gradient::Sweep {
        center: Point::new(x, y),
        start: (start + 1.0) * std::f64::consts::PI,
        end: (end + 1.0) * std::f64::consts::PI,
    }
}

fn blend_mode(mode: CompositeMode) -> BlendMode {
    use CompositeMode as Colr;
    let (mix, compose) = match mode {
        Colr::Clear => (Mix::Normal, Compose::Clear),
        Colr::Src => (Mix::Normal, Compose::Copy),
        Colr::Dest => (Mix::Normal, Compose::Dest),
        Colr::SrcOver | Colr::Unknown => (Mix::Normal, Compose::SrcOver),
        Colr::DestOver => (Mix::Normal, Compose::DestOver),
        Colr::SrcIn => (Mix::Normal, Compose::SrcIn),
        Colr::DestIn => (Mix::Normal, Compose::DestIn),
        Colr::SrcOut => (Mix::Normal, Compose::SrcOut),
        Colr::DestOut => (Mix::Normal, Compose::DestOut),
        Colr::SrcAtop => (Mix::Normal, Compose::SrcAtop),
        Colr::DestAtop => (Mix::Normal, Compose::DestAtop),
        Colr::Xor => (Mix::Normal, Compose::Xor),
        Colr::Plus => (Mix::Normal, Compose::Plus),
        Colr::Screen => (Mix::Screen, Compose::SrcOver),
        Colr::Overlay => (Mix::Overlay, Compose::SrcOver),
        Colr::Darken => (Mix::Darken, Compose::SrcOver),
        Colr::Lighten => (Mix::Lighten, Compose::SrcOver),
        Colr::ColorDodge => (Mix::ColorDodge, Compose::SrcOver),
        Colr::ColorBurn => (Mix::ColorBurn, Compose::SrcOver),
        Colr::HardLight => (Mix::HardLight, Compose::SrcOver),
        Colr::SoftLight => (Mix::SoftLight, Compose::SrcOver),
        Colr::Difference => (Mix::Difference, Compose::SrcOver),
        Colr::Exclusion => (Mix::Exclusion, Compose::SrcOver),
        Colr::Multiply => (Mix::Multiply, Compose::SrcOver),
        Colr::HslHue => (Mix::Hue, Compose::SrcOver),
        Colr::HslSaturation => (Mix::Saturation, Compose::SrcOver),
        Colr::HslColor => (Mix::Color, Compose::SrcOver),
        Colr::HslLuminosity => (Mix::Luminosity, Compose::SrcOver),
    };
    BlendMode::new(mix, compose)
}
