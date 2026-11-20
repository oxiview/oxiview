//! CPU replay of a COLR graph. Vello rasterizes clip coverage and implements the
//! blend operators. Color stays premultiplied linear-light until the glyph is
//! flattened to device-space tiles in the Scene compositor.

use super::outline;
use crate::Error;
use fearless_simd::{Level, Simd, SimdBase, dispatch, f32x16};
use oxiview_base::Glyph;
use skrifa::{
    FontRef, GlyphId, MetadataProvider, color::Extend, instance::LocationRef,
    raw::tables::cpal::ColorRecord,
};
use vello_cpu::{
    LinearImages, Pixmap, RenderContext, Resources,
    color::{AlphaColor, LinearSrgb, Srgb},
    fine::{F32Kernel, FineKernel},
    kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2},
    peniko::BlendMode,
};

mod read;

type Pixel = [f32; 4];
const TRANSPARENT: Pixel = [0.0; 4];
const TILE_SIZE: u16 = 128;

#[derive(Default)]
pub(super) struct ColorRasterizer {
    masks: Option<RenderContext>,
    resources: Resources,
    buffers: Vec<Vec<f32>>,
}

impl ColorRasterizer {
    pub(super) fn paint(
        &mut self,
        context: &mut RenderContext,
        images: &mut LinearImages<'_>,
        face: &FontRef<'_>,
        glyph: &Glyph,
        location: LocationRef<'_>,
        placement: Affine,
    ) -> Result<(), Error> {
        let foreground =
            AlphaColor::<LinearSrgb>::new(glyph.color.to_linear().map(|c| c.clamp(0.0, 1.0)));
        let opacity = foreground.components[3];
        if opacity == 0.0 || placement.determinant() == 0.0 {
            return Ok(());
        }
        let palettes = face.color_palettes();
        let palette = palettes
            .get(0)
            .ok_or_else(|| invalid("color glyph has no usable palette"))?;
        let viewport = Rect::new(0.0, 0.0, context.width().into(), context.height().into());
        let mut graph = Graph {
            commands: Vec::new(),
            remaining_work: 1 << 20,
            clips: vec![viewport],
            bounds: None,
            palette: palette.colors(),
            foreground: foreground.with_alpha(1.0),
        };
        read::record(
            &mut graph,
            face,
            GlyphId::from(glyph.glyph_id),
            location,
            placement,
        )?;
        let Some(bounds) = graph.bounds else {
            return Ok(());
        };

        let saved = context.save_current_state();
        context.reset_paint_transform();
        let bounds = bounds.expand();
        let (left, top, right, bottom) = (
            bounds.x0 as u16,
            bounds.y0 as u16,
            bounds.x1 as u16,
            bounds.y1 as u16,
        );
        for y in (top..bottom).step_by(TILE_SIZE.into()) {
            for x in (left..right).step_by(TILE_SIZE.into()) {
                let width = TILE_SIZE.min(right - x);
                let height = TILE_SIZE.min(bottom - y);
                if let Some(tile) = self.tile(&graph.commands, x, y, width, height, opacity) {
                    let image = images
                        .insert_tile(width, height, tile)
                        .ok_or_else(|| invalid("invalid COLR tile size"))?;
                    context.set_transform(Affine::translate((f64::from(x), f64::from(y))));
                    context.set_paint(image);
                    context.fill_rect(&Rect::new(0.0, 0.0, width.into(), height.into()));
                }
            }
        }
        context.restore_state(saved);
        Ok(())
    }

    fn tile(
        &mut self,
        commands: &[Command],
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        opacity: f32,
    ) -> Option<Vec<f32>> {
        let origin = Vec2::new(x.into(), y.into());
        let bounds = Rect::from_origin_size(Point::ZERO + origin, (width.into(), height.into()));
        let pixels = usize::from(width) * usize::from(height);
        // Vello's blend kernel takes batches of four RGBA pixels.
        let len = pixels.next_multiple_of(4) * 4;
        let mut layers = vec![self.buffer(len)];
        let mut coverage = Pixmap::new(width, height);
        let simd = Level::try_detect().unwrap_or(Level::baseline());
        let mut skip_depth = 0;
        for command in commands {
            match command {
                Command::Clip(clip) if skip_depth != 0 || clip.intersect(bounds).is_zero_area() => {
                    skip_depth += 1;
                }
                Command::Layer if skip_depth != 0 => skip_depth += 1,
                Command::Layer | Command::Clip(..) => layers.push(self.buffer(len)),
                Command::Fill(Paint::Solid(color)) if skip_depth == 0 && color[3] == 1.0 => {
                    layers.last_mut()?.as_chunks_mut::<4>().0[..pixels].fill(*color);
                }
                Command::Fill(paint) if skip_depth == 0 => {
                    let target = layers.last_mut()?;
                    for (index, pixel) in target.chunks_exact_mut(4).take(pixels).enumerate() {
                        let width = usize::from(width);
                        let point = Point::new(
                            f64::from(x) + (index % width) as f64 + 0.5,
                            f64::from(y) + (index / width) as f64 + 0.5,
                        );
                        let src = paint.sample(point);
                        for (dst, src_channel) in pixel.iter_mut().zip(src) {
                            *dst = src_channel + *dst * (1.0 - src[3]);
                        }
                    }
                }
                Command::Fill(_) => {}
                Command::EndClip(_) | Command::EndLayer(_) if skip_depth != 0 => skip_depth -= 1,
                Command::EndClip(path) => {
                    let mut source = layers.pop()?;
                    let masks = self
                        .masks
                        .get_or_insert_with(|| RenderContext::new(width, height));
                    masks.reset_and_resize(width, height);
                    masks.set_transform(Affine::translate(-origin));
                    masks.set_paint(AlphaColor::<Srgb>::WHITE);
                    masks.fill_path(path);
                    masks.render(&mut coverage, &mut self.resources);
                    for (pixel, mask) in source
                        .chunks_exact_mut(4)
                        .zip(coverage.data_as_u8_slice().chunks_exact(4))
                    {
                        let alpha = f32::from(mask[3]) / 255.0;
                        for channel in pixel {
                            *channel *= alpha;
                        }
                    }
                    blend(simd, layers.last_mut()?, &source, BlendMode::default());
                    self.buffers.push(source);
                }
                Command::EndLayer(mode) => {
                    let source = layers.pop()?;
                    blend(simd, layers.last_mut()?, &source, *mode);
                    self.buffers.push(source);
                }
            }
        }
        let mut source = layers.pop()?;
        source.truncate(pixels * 4);
        let mut visible = false;
        for pixel in source.as_chunks_mut::<4>().0 {
            for component in &mut *pixel {
                *component *= opacity;
            }
            visible |= pixel[3] > 0.0;
        }
        if visible {
            Some(source)
        } else {
            self.buffers.push(source);
            None
        }
    }

    fn buffer(&mut self, len: usize) -> Vec<f32> {
        let mut buffer = self.buffers.pop().unwrap_or_default();
        buffer.resize(len, 0.0);
        buffer.fill(0.0);
        buffer
    }
}

fn blend(level: Level, dest: &mut [f32], source: &[f32], mode: BlendMode) {
    dispatch!(level, simd => blend_pixels(simd, dest, source, mode));
}

fn blend_pixels<S: Simd>(simd: S, dest: &mut [f32], source: &[f32], mode: BlendMode) {
    let source = source
        .as_chunks::<16>()
        .0
        .iter()
        .map(|p| f32x16::from_slice(simd, p));
    F32Kernel::blend(simd, dest, 0, 0, source, mode, None, None);
}

enum Command {
    Clip(Rect),
    EndClip(BezPath),
    Layer,
    EndLayer(BlendMode),
    Fill(Paint),
}

struct Graph<'a> {
    commands: Vec<Command>,
    remaining_work: usize,
    clips: Vec<Rect>,
    bounds: Option<Rect>,
    palette: &'a [ColorRecord],
    foreground: AlphaColor<LinearSrgb>,
}

impl Graph<'_> {
    fn charge(&mut self, work: usize) -> Result<(), Error> {
        self.remaining_work = self
            .remaining_work
            .checked_sub(work)
            .ok_or_else(|| invalid("COLR paint graph exceeds replay limits"))?;
        Ok(())
    }

    fn color(&self, index: u16, alpha: f32) -> Result<Pixel, Error> {
        let color = if index == u16::MAX {
            self.foreground
        } else {
            let entry = self
                .palette
                .get(usize::from(index))
                .ok_or_else(|| invalid("invalid COLR palette index"))?;
            AlphaColor::<Srgb>::from_rgba8(entry.red, entry.green, entry.blue, entry.alpha)
                .convert::<LinearSrgb>()
        };
        Ok(color
            .multiply_alpha(alpha.clamp(0.0, 1.0))
            .premultiply()
            .components)
    }

    fn clip(
        &mut self,
        path: BezPath,
        draw: impl FnOnce(&mut Self) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.charge(path.elements().len())?;
        let path_bounds = path.bounding_box();
        if ![
            path_bounds.x0,
            path_bounds.y0,
            path_bounds.x1,
            path_bounds.y1,
        ]
        .into_iter()
        .all(f64::is_finite)
        {
            return Err(invalid("non-finite COLR clip"));
        }
        let bounds = self
            .clips
            .last()
            .map_or(path_bounds, |parent| parent.intersect(path_bounds));
        self.clips.push(bounds);
        self.commands.push(Command::Clip(bounds));
        draw(self)?;
        self.commands.push(Command::EndClip(path));
        self.clips.pop();
        Ok(())
    }

    fn fill(&mut self, paint: Paint) {
        if let Some(&bounds) = self.clips.last().filter(|bounds| !bounds.is_zero_area()) {
            self.bounds = Some(
                self.bounds
                    .map_or(bounds, |existing| existing.union(bounds)),
            );
            self.commands.push(Command::Fill(paint));
        }
    }
}

enum Paint {
    Solid(Pixel),
    Gradient {
        inverse: Affine,
        geometry: Gradient,
        stops: Vec<(f64, Pixel)>,
        extend: Extend,
    },
}

impl Paint {
    fn sample(&self, point: Point) -> Pixel {
        match self {
            Self::Solid(color) => *color,
            Self::Gradient {
                inverse,
                geometry,
                stops,
                extend,
            } => {
                let Some(t) = geometry.position(*inverse * point) else {
                    return TRANSPARENT;
                };
                sample_color_line(stops, *extend, t)
            }
        }
    }
}

enum Gradient {
    Linear {
        start: Point,
        projection: Vec2,
    },
    Radial {
        start: Point,
        delta: Vec2,
        radius: f64,
        radius_delta: f64,
    },
    Sweep {
        center: Point,
        start: f64,
        end: f64,
    },
}

impl Gradient {
    fn position(&self, point: Point) -> Option<f64> {
        match *self {
            Self::Linear { start, projection } => Some((point - start).dot(projection)),
            Self::Radial {
                start,
                delta,
                radius,
                radius_delta,
            } => {
                // Solve |point - center(t)|² = radius(t)². The latest circle
                // with a positive radius is the one visible at this point.
                let p = point - start;
                let a = delta.hypot2() - radius_delta * radius_delta;
                let b = -2.0 * (p.dot(delta) + radius * radius_delta);
                let c = p.hypot2() - radius * radius;
                let valid = |t: &f64| t.is_finite() && radius + *t * radius_delta > 0.0;
                if a == 0.0 {
                    return Some(-c / b).filter(valid);
                }
                let discriminant = b * b - 4.0 * a * c;
                let q = -0.5 * (b + discriminant.sqrt().copysign(b));
                if q == 0.0 {
                    return Some(-b / (2.0 * a)).filter(valid);
                }
                [q / a, c / q]
                    .into_iter()
                    .filter(valid)
                    .max_by(f64::total_cmp)
            }
            Self::Sweep { center, start, end } => {
                let p = point - center;
                let angle = p.y.atan2(p.x).rem_euclid(std::f64::consts::TAU);
                if start == end {
                    return Some(if angle < start {
                        f64::NEG_INFINITY
                    } else {
                        f64::INFINITY
                    });
                }
                Some((angle - start) / (end - start))
            }
        }
    }
}

fn sample_color_line(stops: &[(f64, Pixel)], extend: Extend, mut t: f64) -> Pixel {
    let Some(&(first, first_color)) = stops.first() else {
        return TRANSPARENT;
    };
    if let [(_, color)] = stops {
        return *color;
    }
    let Some(&(last, last_color)) = stops.last() else {
        return TRANSPARENT;
    };
    let span = last - first;
    if matches!(extend, Extend::Repeat | Extend::Reflect) {
        if span == 0.0 {
            return TRANSPARENT;
        }
        if t < first || t > last {
            t = if extend == Extend::Repeat {
                first + (t - first).rem_euclid(span)
            } else {
                last - ((t - first).rem_euclid(2.0 * span) - span).abs()
            };
        }
    }
    let upper = stops.partition_point(|(offset, _)| *offset <= t);
    match (
        upper.checked_sub(1).and_then(|i| stops.get(i)),
        stops.get(upper),
    ) {
        (Some(&(left, a)), Some(&(right, b))) => {
            let weight = ((t - left) / (right - left)) as f32;
            std::array::from_fn(|i| a[i] + (b[i] - a[i]) * weight)
        }
        (None, _) => first_color,
        (_, None) => last_color,
    }
}

fn invalid(message: impl std::fmt::Display) -> Error {
    Error::Glyph(message.to_string())
}
