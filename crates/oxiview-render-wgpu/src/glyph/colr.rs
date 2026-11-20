//! Replay COLR graphs that need original gradient geometry or radial coverage.

use super::{PathPen, glyph_error, hard_stop, linear_channels};
use crate::{Error, vector::invalid};
use oxiview_base::Glyph;
use skrifa::{
    FontRef, GlyphId, MetadataProvider,
    color::{Brush, ColorPainter, ColorStop, CompositeMode, Extend, Transform},
    instance::{LocationRef, Size},
    outline::DrawSettings,
    raw::{tables::cpal::ColorRecord, types::BoundingBox},
};
use std::f64::consts::TAU;
use vello_gpu::{
    Scene,
    kurbo::{Affine, Point, Rect},
    peniko::{
        BlendMode, ColorStop as VelloStop, ColorStops, Compose, Gradient, Mix,
        RadialGradientPosition,
        color::{AlphaColor, DynamicColor, Srgb},
    },
};

pub(super) fn paint(
    scene: &mut Scene,
    face: &FontRef<'_>,
    glyph: &Glyph,
    location: LocationRef<'_>,
    placement: Affine,
    foreground: AlphaColor<Srgb>,
    palette: &[ColorRecord],
) -> Result<(), Error> {
    let color_glyph = face
        .color_glyphs()
        .get(GlyphId::from(glyph.glyph_id))
        .ok_or_else(|| invalid("COLR glyph is missing"))?;
    let bounds = Rect::new(
        0.0,
        0.0,
        f64::from(scene.width()),
        f64::from(scene.height()),
    );
    let mut painter = Painter {
        scene,
        face,
        location,
        foreground: linear_channels(foreground),
        palette,
        bounds,
        transforms: vec![placement],
        stack: Vec::new(),
        error: None,
        source: hard_stop::Source::new(glyph.font.bytes(), glyph.face_index, location.coords())?,
    };
    let result = color_glyph
        .paint(location, &mut painter)
        .map_err(glyph_error);
    // Skrifa may leave the root clip in place on a traversal error.
    while let Some(entry) = painter.stack.pop() {
        match entry {
            StackEntry::Clip => painter.scene.pop_clip(),
            StackEntry::Layer => painter.scene.pop_layer(),
        }
    }
    result.and(painter.error.map_or(Ok(()), Err))
}

enum StackEntry {
    Clip,
    Layer,
}

struct Painter<'a, 'font> {
    scene: &'a mut Scene,
    face: &'a FontRef<'font>,
    location: LocationRef<'a>,
    foreground: AlphaColor<Srgb>,
    palette: &'a [ColorRecord],
    bounds: Rect,
    transforms: Vec<Affine>,
    stack: Vec<StackEntry>,
    error: Option<Error>,
    source: hard_stop::Source<'a>,
}

impl Painter<'_, '_> {
    fn transform(&self) -> Affine {
        self.transforms.last().copied().unwrap_or_default()
    }

    fn color(&self, index: u16, alpha: f32) -> AlphaColor<Srgb> {
        let color = if index == u16::MAX {
            self.foreground
        } else if let Some(color) = self.palette.get(usize::from(index)) {
            linear_channels(AlphaColor::from_rgba8(
                color.red,
                color.green,
                color.blue,
                color.alpha,
            ))
        } else {
            // The validation callback checks every referenced palette index.
            AlphaColor::TRANSPARENT
        };
        color.multiply_alpha(alpha.clamp(0.0, 1.0))
    }

    fn stops(&self, stops: &[ColorStop]) -> ColorStops {
        ColorStops(
            stops
                .iter()
                .map(|stop| vello_gpu::peniko::ColorStop {
                    offset: stop.offset,
                    color: DynamicColor::from_alpha_color(
                        self.color(stop.palette_index, stop.alpha),
                    ),
                })
                .collect(),
        )
    }

    fn hard_stop(&mut self, marker: Brush<'_>) -> Result<(), Error> {
        use hard_stop::Geometry;
        let transition = self.source.read(marker, self.palette.len())?;
        let before = self.color(transition.before.palette_index, transition.before.alpha);
        let after = self.color(transition.after.palette_index, transition.after.alpha);
        let offset = transition.offset;
        let transform = self.transform();
        if transform.determinant() == 0.0 {
            return Ok(());
        }
        let gradient =
            match transition.geometry {
                Geometry::Linear([x0, y0, x1, y1, x2, y2]) => {
                    let origin = Point::new(x0, y0);
                    let normal = vello_gpu::kurbo::Vec2::new(y2 - y0, x0 - x2);
                    let projection = (Point::new(x1, y1) - origin).dot(normal);
                    if projection == 0.0 {
                        return Ok(());
                    }
                    let axis = normal * (projection / normal.hypot2());
                    let center = origin + axis * offset;
                    Some(
                        Gradient::new_linear(center - axis * 0.5, center + axis * 0.5)
                            .with_stops(hard_stops(0.5, before, after)),
                    )
                }
                Geometry::Radial([x0, y0, r0, x1, y1, r1]) => {
                    let c0 = Point::new(x0, y0);
                    let c1 = Point::new(x1, y1);
                    let radius = |t: f64| (r0 + t * (r1 - r0)) as f32;
                    // Keep a unit span centered at the original transition.
                    // Reparameterizing the same cone preserves its two roots.
                    let start = offset - 0.5;
                    let end = offset + 0.5;
                    radial(
                        c0.lerp(c1, start),
                        radius(start),
                        c0.lerp(c1, end),
                        radius(end),
                        hard_stops(0.5, before, after),
                        vello_gpu::peniko::Extend::Pad,
                    )
                }
                Geometry::Sweep([x, y, start, end]) => {
                    // Vello measures clockwise. Encode the transition in a full
                    // turn and order the colors by its direction.
                    let stop = 1.0 - (start + (end - start) * offset) / TAU;
                    let (before, after) = if end >= start {
                        (after, before)
                    } else {
                        (before, after)
                    };
                    if stop <= 0.0 || stop >= 1.0 {
                        self.solid(if stop <= 0.0 { after } else { before });
                        return Ok(());
                    }
                    self.gradient(
                        Gradient::new_sweep(Point::new(x, -y), 0.0, TAU as f32)
                            .with_stops(hard_stops(stop as f32, before, after)),
                        transform * Affine::scale_non_uniform(1.0, -1.0),
                    );
                    return Ok(());
                }
            };
        if let Some(gradient) = gradient {
            self.gradient(gradient, transform);
        }
        Ok(())
    }

    fn solid(&mut self, color: AlphaColor<Srgb>) {
        self.scene.set_transform(Affine::IDENTITY);
        self.scene.set_paint(color);
        self.scene.fill_rect(&self.bounds);
    }

    fn gradient(&mut self, gradient: Gradient, transform: Affine) {
        if transform.determinant() == 0.0 {
            return;
        }
        self.scene.set_transform(Affine::IDENTITY);
        self.scene.set_paint_transform(transform);
        self.scene.set_paint(
            gradient
                .with_interpolation_cs(vello_gpu::peniko::color::ColorSpaceTag::Srgb)
                .with_interpolation_alpha_space(
                    vello_gpu::peniko::InterpolationAlphaSpace::Premultiplied,
                ),
        );
        self.scene.fill_rect(&self.bounds);
    }
}

fn hard_stops(offset: f32, before: AlphaColor<Srgb>, after: AlphaColor<Srgb>) -> ColorStops {
    ColorStops(
        [
            (0.0, before),
            (offset, before),
            (offset, after),
            (1.0, after),
        ]
        .into_iter()
        .map(|(offset, color)| VelloStop {
            offset,
            color: DynamicColor::from_alpha_color(color),
        })
        .collect(),
    )
}

impl ColorPainter for Painter<'_, '_> {
    fn push_transform(&mut self, transform: Transform) {
        self.transforms.push(
            self.transform()
                * Affine::new([
                    f64::from(transform.xx),
                    f64::from(transform.yx),
                    f64::from(transform.xy),
                    f64::from(transform.yy),
                    f64::from(transform.dx),
                    f64::from(transform.dy),
                ]),
        );
    }

    fn pop_transform(&mut self) {
        self.transforms.pop();
    }

    fn push_clip_glyph(&mut self, id: GlyphId) {
        let mut pen = PathPen::default();
        let result = self
            .face
            .outline_glyphs()
            .get(id)
            .ok_or_else(|| invalid("COLR clip outline is missing"))
            .and_then(|outline| {
                outline
                    .draw(
                        DrawSettings::unhinted(Size::unscaled(), self.location),
                        &mut pen,
                    )
                    .map_err(glyph_error)
            });
        if let Err(error) = result {
            self.error = Some(error);
            pen = PathPen::default();
        }
        self.scene.set_transform(self.transform());
        self.scene.push_clip_path(&pen.0);
        self.stack.push(StackEntry::Clip);
    }

    fn push_clip_box(&mut self, bounds: BoundingBox<f32>) {
        self.scene.set_transform(self.transform());
        self.scene.push_clip_rect(&Rect::new(
            f64::from(bounds.x_min),
            f64::from(bounds.y_min),
            f64::from(bounds.x_max),
            f64::from(bounds.y_max),
        ));
        self.stack.push(StackEntry::Clip);
    }

    fn pop_clip(&mut self) {
        self.scene.pop_clip();
        self.stack.pop();
    }

    fn push_layer(&mut self, mode: CompositeMode) {
        self.scene.push_blend_layer(blend_mode(mode));
        self.stack.push(StackEntry::Layer);
    }

    fn pop_layer(&mut self) {
        self.scene.pop_layer();
        self.stack.pop();
    }

    fn fill(&mut self, brush: Brush<'_>) {
        let mut transform = self.transform();
        let gradient = match brush {
            Brush::Solid {
                palette_index,
                alpha,
            } => {
                self.solid(self.color(palette_index, alpha));
                return;
            }
            Brush::LinearGradient {
                p0,
                p1,
                color_stops,
                extend,
            } => Gradient::new_linear(point(p0), point(p1))
                .with_stops(self.stops(color_stops))
                .with_extend(extend_mode(extend)),
            Brush::RadialGradient {
                c0,
                r0,
                c1,
                r1,
                color_stops,
                extend,
            } => {
                let Some(gradient) = radial(
                    point(c0),
                    r0,
                    point(c1),
                    r1,
                    self.stops(color_stops),
                    extend_mode(extend),
                ) else {
                    return;
                };
                gradient
            }
            Brush::SweepGradient {
                c0,
                start_angle,
                mut end_angle,
                color_stops,
                extend,
            } => {
                if start_angle == end_angle {
                    end_angle += 0.01;
                }
                transform *= Affine::scale_non_uniform(1.0, -1.0);
                let center = point(c0);
                Gradient::new_sweep(
                    Point::new(center.x, -center.y),
                    start_angle.to_radians(),
                    end_angle.to_radians(),
                )
                .with_stops(self.stops(color_stops))
                .with_extend(extend_mode(extend))
            }
        };
        self.gradient(gradient, transform);
    }

    fn fill_glyph(&mut self, id: GlyphId, transform: Option<Transform>, brush: Brush<'_>) {
        if id == hard_stop::MARKER_GLYPH {
            if let Err(error) = self.hard_stop(brush) {
                self.error = Some(error);
            }
            return;
        }
        self.push_clip_glyph(id);
        if let Some(transform) = transform {
            self.push_transform(transform);
            self.fill(brush);
            self.pop_transform();
        } else {
            self.fill(brush);
        }
        self.pop_clip();
    }
}

fn point(p: skrifa::raw::types::Point<f32>) -> Point {
    Point::new(f64::from(p.x), f64::from(p.y))
}

fn radial(
    c0: Point,
    r0: f32,
    c1: Point,
    r1: f32,
    mut stops: ColorStops,
    extend: vello_gpu::peniko::Extend,
) -> Option<Gradient> {
    if (r0 == r1 && c0 == c1) || (r0 <= 0.0 && r1 == r0) {
        return None;
    }
    let mut circles = RadialGradientPosition {
        start_center: c0,
        start_radius: r0,
        end_center: c1,
        end_radius: r1,
    };
    // Vello picks the largest parameter with positive radius, as required by
    // OpenType. Keep the original direction even for a cone with two roots.
    // https://learn.microsoft.com/en-us/typography/opentype/spec/colr#radial-gradients
    // Vello needs nonnegative radii. Restrict Pad to the visible half of the
    // cone or shift Repeat/Reflect by whole periods into that half.
    if r0 < 0.0 || r1 < 0.0 {
        let radius_delta = f64::from(r1) - f64::from(r0);
        let tip = -f64::from(r0) / radius_delta;
        let growing = radius_delta > 0.0;
        use vello_gpu::peniko::Extend;
        let (start, end) = match extend {
            Extend::Pad => {
                let (start, end) = if growing {
                    (tip, if tip < 1.0 { 1.0 } else { tip + 1.0 })
                } else {
                    (if tip > 0.0 { 0.0 } else { tip - 1.0 }, tip)
                };
                trim_stops(&mut stops, start, end)?;
                (start, end)
            }
            Extend::Repeat | Extend::Reflect => {
                let period = if extend == Extend::Repeat { 1.0 } else { 2.0 };
                let start = if growing {
                    (tip / period).ceil() * period
                } else {
                    ((tip - 1.0) / period).floor() * period
                };
                (start, start + 1.0)
            }
        };
        circles = RadialGradientPosition {
            start_center: c0.lerp(c1, start),
            start_radius: ((start - tip) * radius_delta).max(0.0) as f32,
            end_center: c0.lerp(c1, end),
            end_radius: ((end - tip) * radius_delta).max(0.0) as f32,
        };
    }
    Some(Gradient {
        kind: circles.into(),
        stops,
        extend,
        ..Gradient::default()
    })
}

fn trim_stops(stops: &mut ColorStops, start: f64, end: f64) -> Option<()> {
    // At a hard stop on the tip, use the color approached from the side
    // with positive radius. Interpolate the endpoint in premultiplied linear light.
    let lower = stops.partition_point(|stop| f64::from(stop.offset) <= start);
    let upper = stops.partition_point(|stop| f64::from(stop.offset) < end);
    let first = color_at(stops, lower, start)?;
    let last = color_at(stops, upper, end)?;
    stops.retain(|stop| start < f64::from(stop.offset) && f64::from(stop.offset) < end);
    for stop in stops.iter_mut() {
        stop.offset = ((f64::from(stop.offset) - start) / (end - start)) as f32;
    }
    stops.insert(
        0,
        VelloStop {
            offset: 0.0,
            color: first,
        },
    );
    stops.push(VelloStop {
        offset: 1.0,
        color: last,
    });
    Some(())
}

fn color_at(stops: &[VelloStop], upper: usize, offset: f64) -> Option<DynamicColor> {
    let left = upper.checked_sub(1).and_then(|i| stops.get(i));
    let right = stops.get(upper);
    match (left, right) {
        (Some(left), Some(right)) => {
            let start = f64::from(left.offset);
            let weight = ((offset - start) / (f64::from(right.offset) - start)) as f32;
            let left = left.color.to_alpha_color::<Srgb>().premultiply();
            let right = right.color.to_alpha_color::<Srgb>().premultiply();
            Some(DynamicColor::from_alpha_color(
                left.lerp_rect(right, weight).un_premultiply(),
            ))
        }
        (Some(stop), None) | (None, Some(stop)) => Some(stop.color),
        (None, None) => None,
    }
}

fn extend_mode(extend: Extend) -> vello_gpu::peniko::Extend {
    use vello_gpu::peniko::Extend as Vello;
    match extend {
        Extend::Pad | Extend::Unknown => Vello::Pad,
        Extend::Repeat => Vello::Repeat,
        Extend::Reflect => Vello::Reflect,
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
