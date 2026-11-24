use crate::{
    Error, PixelSize,
    atlas::{Atlas, Frame, Source},
    encoding::{radii_fit, sdf_scale},
    glyph::GlyphPainter,
};
use oxiview_base::{Color, CornerRadii, GlyphKind, Point, Rect};
use oxiview_scene::{
    BezierPath, Brush, Clip, EncodedNode, EncodedScene, FillRule, PathCommand, Primitive,
    ResolvedClip,
};
use std::{
    collections::HashSet,
    f64::consts::{FRAC_PI_2, PI},
};
use vello_gpu::{Image, RenderOptions, RenderTarget, RenderTargetConfig, kurbo, peniko};

pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub(crate) fn needs_vello(scene: &EncodedScene) -> bool {
    scene.nodes.iter().any(|node| match node {
        EncodedNode::Draw {
            primitive,
            transform,
            clips,
        } => {
            matches!(
                primitive,
                Primitive::Image(..) | Primitive::Glyph(_) | Primitive::FillPath(..)
            ) || sdf_scale(*transform).is_none()
                || clips.iter().any(|clip| sdf_scale(clip.transform).is_none())
        }
        _ => false,
    })
}

pub(crate) struct Backend {
    renderer: vello_gpu::Renderer,
    resources: vello_gpu::Resources,
    scene: vello_gpu::Scene,
    glyphs: GlyphPainter,
    pub(crate) atlas: Atlas,
}

impl Backend {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let (renderer, resources) = vello_gpu::Renderer::new(
            device,
            queue,
            &RenderTargetConfig {
                format: FORMAT,
                width: 1,
                height: 1,
            },
        );
        Self {
            renderer,
            resources,
            scene: vello_gpu::Scene::new(1, 1),
            glyphs: GlyphPainter::default(),
            atlas: Atlas::new(device),
        }
    }

    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input: &EncodedScene,
        target: &wgpu::TextureView,
        size: PixelSize,
    ) -> Result<(), Error> {
        let width = u16::try_from(size.width).map_err(|_| Error::InvalidSize(size))?;
        let height = u16::try_from(size.height).map_err(|_| Error::InvalidSize(size))?;
        self.scene.reset_and_resize(width, height);
        self.glyphs.maintain();

        let mut nodes = input.nodes.as_slice();
        let mut options = RenderOptions::default();
        while let Some((first, remaining)) = nodes.split_first()
            && let Some(color) = viewport_background(first, size)?
        {
            options.target_init =
                vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::Viewport {
                    color: peniko::color::AlphaColor::new(color),
                });
            nodes = remaining;
        }
        // Pin every atlas item referenced by the frame, including draws which have
        // not been translated yet. Texture writes precede submission of all draws.
        let required = nodes.iter().filter_map(source_key).collect::<HashSet<_>>();
        let mut frame = self.atlas.frame(required);
        let mut paint = Paint {
            scene: &mut self.scene,
            atlas: &mut self.atlas,
            glyphs: &mut self.glyphs,
            frame: &mut frame,
            device,
            queue,
        };
        let mut active_clips: &[ResolvedClip] = &[];
        for node in nodes {
            match node {
                EncodedNode::BeginOpacity(opacity) => {
                    paint.set_clips(active_clips, &[])?;
                    active_clips = &[];
                    paint.scene.push_opacity_layer(*opacity as f32);
                }
                EncodedNode::EndOpacity => {
                    paint.set_clips(active_clips, &[])?;
                    active_clips = &[];
                    paint.scene.pop_layer();
                }
                EncodedNode::Draw {
                    primitive,
                    transform,
                    clips,
                } => {
                    let transform = affine(transform.coefficients())?;
                    if transform.determinant() == 0.0 {
                        continue;
                    }
                    paint.set_clips(active_clips, clips)?;
                    active_clips = clips;
                    paint.scene.set_transform(transform);
                    paint.draw(primitive, transform, clips.is_empty())?;
                }
            }
        }
        paint.set_clips(active_clips, &[])?;

        self.renderer
            .render(
                &self.scene,
                &mut self.resources,
                encoder,
                RenderTarget {
                    view: target,
                    depth_view: None,
                },
                &frame.bindings,
                options,
            )
            .map_err(|error| Error::SceneEncoding(error.to_string()))?;
        let counters = self.atlas.counters;
        tracing::trace!(target: "oxiview_render_wgpu",
            atlas_entries = self.atlas.resident_count(), atlas_hits = counters.hits,
            atlas_uploads = counters.uploads, atlas_evictions = counters.evictions,
            overflow_uploads = counters.overflow_uploads, "Vello frame encoded");
        Ok(())
    }
}

// A leading opaque fill that exactly covers the target can use the attachment
// clear. Successive full backgrounds replace one another before other draws.
fn viewport_background(node: &EncodedNode, size: PixelSize) -> Result<Option<[f32; 4]>, Error> {
    let EncodedNode::Draw {
        primitive: Primitive::Fill(rect, radii, Brush::Solid(color)),
        transform,
        clips,
    } = node
    else {
        return Ok(None);
    };
    let [a, b, c, d, x, y] = transform.coefficients();
    let [x0, y0, x1, y1] = rect.edges();
    if !clips.is_empty()
        || *radii != CornerRadii::default()
        || b != 0.0
        || c != 0.0
        || [a * x0 + x, d * y0 + y, a * x1 + x, d * y1 + y]
            != [0.0, 0.0, f64::from(size.width), f64::from(size.height)]
    {
        return Ok(None);
    }
    checked([a, d, x, y, x0, y0, x1, y1])?;
    let color = valid_color(*color)?;
    Ok((color[3] == 1.0).then_some(color))
}

fn source_key(node: &EncodedNode) -> Option<crate::atlas::Key> {
    let EncodedNode::Draw { primitive, .. } = node else {
        return None;
    };
    let source = match primitive {
        Primitive::Image(_, image) => Source::Image(image),
        Primitive::Fill(_, _, Brush::Solid(color))
        | Primitive::FillPath(_, _, Brush::Solid(color))
        | Primitive::Stroke(_, _, _, Brush::Solid(color)) => Source::Solid(linear(*color)),
        Primitive::Glyph(glyph) if glyph.kind == GlyphKind::Outline => {
            Source::Solid(linear(glyph.color))
        }
        _ => return None,
    };
    Some(source.key())
}

struct Paint<'a> {
    scene: &'a mut vello_gpu::Scene,
    atlas: &'a mut Atlas,
    glyphs: &'a mut GlyphPainter,
    frame: &'a mut Frame,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
}

impl Paint<'_> {
    fn set_clips(&mut self, previous: &[ResolvedClip], next: &[ResolvedClip]) -> Result<(), Error> {
        // Clip paths mask individual draws. Retain the common prefix across
        // adjacent draws to reuse their rasterized intersections.
        let shared = previous
            .iter()
            .zip(next)
            .take_while(|(a, b)| a == b)
            .count();
        for _ in shared..previous.len() {
            self.scene.pop_clip();
        }
        for clip in next.iter().skip(shared) {
            self.scene
                .set_transform(affine(clip.transform.coefficients())?);
            let (rect, radii) = match clip.shape {
                Clip::Rect(rect) => (rect, CornerRadii::default()),
                Clip::RoundedRect(rect, radii) => (rect, radii),
            };
            if radii == CornerRadii::default() {
                let [x0, y0, x1, y1] = rect.edges();
                checked([x0, y0, x1, y1])?;
                self.scene
                    .push_clip_rect(&kurbo::Rect::new(x0, y0, x1, y1).abs());
            } else {
                self.scene.push_clip_path(&rounded(rect, radii)?);
            }
        }
        Ok(())
    }

    fn draw(
        &mut self,
        primitive: &Primitive,
        transform: kurbo::Affine,
        unclipped: bool,
    ) -> Result<(), Error> {
        self.scene.reset_paint_transform();
        match primitive {
            Primitive::Fill(rect, radii, brush) => {
                if unclipped && *radii == CornerRadii::default() && pixel_aligned(*rect, transform)
                {
                    let [x0, y0, x1, y1] = rect.edges();
                    checked([x0, y0, x1, y1])?;
                    if self.brush(brush)? {
                        self.scene
                            .fill_rect(&kurbo::Rect::new(x0, y0, x1, y1).abs());
                    }
                } else {
                    let path = rounded(*rect, *radii)?;
                    if self.brush(brush)? {
                        self.scene.fill_path(&path);
                    }
                }
            }
            Primitive::FillPath(path, rule, brush) => {
                let path = bezier(path, transform)?;
                if self.brush(brush)? {
                    self.scene.set_fill_rule(match rule {
                        FillRule::NonZero => peniko::Fill::NonZero,
                        FillRule::EvenOdd => peniko::Fill::EvenOdd,
                    });
                    self.scene.fill_path(&path);
                    self.scene.set_fill_rule(peniko::Fill::NonZero);
                }
            }
            Primitive::Stroke(rect, radii, width, brush) if *width > 0.0 => {
                checked([*width])?;
                let path = rounded(*rect, *radii)?;
                if self.brush(brush)? {
                    self.scene.set_stroke(kurbo::Stroke::new(*width));
                    self.scene.stroke_path(&path);
                }
            }
            Primitive::Stroke(..) => {}
            Primitive::Image(rect, image) => {
                let [x0, y0, x1, y1] = rect.edges();
                checked([x0, y0, x1, y1])?;
                if x0 != x1 && y0 != y1 {
                    self.texture(Source::Image(image))?;
                    self.scene.set_paint_transform(kurbo::Affine::new([
                        (x1 - x0) / f64::from(image.width()),
                        0.0,
                        0.0,
                        (y1 - y0) / f64::from(image.height()),
                        x0,
                        y0,
                    ]));
                    self.scene
                        .fill_rect(&kurbo::Rect::new(x0, y0, x1, y1).abs());
                }
            }
            Primitive::Glyph(glyph) => match glyph.kind {
                GlyphKind::Outline => {
                    self.texture(Source::Solid(valid_color(glyph.color)?))?;
                    let (path, placement) = self.glyphs.outline(glyph, transform)?;
                    self.scene.set_transform(placement);
                    self.scene.fill_path(&path);
                }
                GlyphKind::Color => self.glyphs.color(self.scene, glyph, transform)?,
            },
        }
        Ok(())
    }

    fn brush(&mut self, brush: &Brush) -> Result<bool, Error> {
        match brush {
            Brush::Solid(color) => {
                let rgba = valid_color(*color)?;
                if rgba[3] == 0.0 {
                    return Ok(false);
                }
                if rgba[3] == 1.0
                    && rgba[..3]
                        .iter()
                        .all(|&channel| channel == 0.0 || channel == 1.0)
                {
                    // Both Vello's packed solid and our half-float atlas represent
                    // these colors exactly. The solid skips a texture sample.
                    self.scene
                        .set_paint(peniko::color::AlphaColor::<peniko::color::Srgb>::new(rgba));
                } else {
                    self.texture(Source::Solid(rgba))?;
                }
            }
            Brush::LinearGradient(start, end, stops) => {
                let positions = [start.x(), start.y(), end.x(), end.y()];
                checked(positions)?;
                let mut gradient =
                    peniko::Gradient::new_linear((start.x(), start.y()), (end.x(), end.y()));
                for stop in stops {
                    gradient.stops.push(peniko::ColorStop::from((
                        stop.position as f32,
                        peniko::color::AlphaColor::<peniko::color::Srgb>::new(valid_color(
                            stop.color,
                        )?),
                    )));
                }
                self.scene.set_paint(gradient);
            }
        }
        Ok(true)
    }

    fn texture(&mut self, source: Source<'_>) -> Result<(), Error> {
        let quality = match source {
            Source::Solid(_) => peniko::ImageQuality::Low,
            Source::Image(_) => peniko::ImageQuality::Medium,
        };
        let source = self
            .atlas
            .image(self.device, self.queue, self.frame, source)?;
        self.scene.set_paint(Image {
            image: source,
            sampler: peniko::ImageSampler::default().with_quality(quality),
        });
        Ok(())
    }
}

fn pixel_aligned(rect: Rect, transform: kurbo::Affine) -> bool {
    let [a, b, c, d, x, y] = transform.as_coeffs();
    let [x0, y0, x1, y1] = rect.edges();
    // Let Vello record a rectangle directly at whole-pixel edges. Preserve the
    // path coverage of clipped, skewed and fractional fill geometry.
    b == 0.0
        && c == 0.0
        && [a * x0 + x, d * y0 + y, a * x1 + x, d * y1 + y]
            .into_iter()
            .all(|edge| edge.fract() == 0.0 && f64::from(edge as f32) == edge)
}

fn affine(coefficients: [f64; 6]) -> Result<kurbo::Affine, Error> {
    checked(coefficients)?;
    Ok(kurbo::Affine::new(coefficients))
}

fn bezier(path: &BezierPath, transform: kurbo::Affine) -> Result<kurbo::BezPath, Error> {
    let point = |local: Point| {
        let local = kurbo::Point::new(local.x(), local.y());
        let device = transform * local;
        checked([device.x, device.y])?;
        Ok(local)
    };
    let mut geometry = kurbo::BezPath::new();
    for &command in path.commands() {
        match command {
            PathCommand::MoveTo(start) => geometry.move_to(point(start)?),
            PathCommand::LineTo(end) => geometry.line_to(point(end)?),
            PathCommand::QuadTo(control, end) => geometry.quad_to(point(control)?, point(end)?),
            PathCommand::CubicTo(first, second, end) => {
                geometry.curve_to(point(first)?, point(second)?, point(end)?);
            }
            PathCommand::Close => geometry.close_path(),
        }
    }
    Ok(geometry)
}

pub(crate) fn checked(values: impl IntoIterator<Item = f64>) -> Result<(), Error> {
    if values.into_iter().any(|value| !(value as f32).is_finite()) {
        return Err(invalid("non-finite or out-of-range GPU geometry"));
    }
    Ok(())
}

pub(crate) fn invalid(message: &str) -> Error {
    Error::SceneEncoding(message.into())
}

fn linear(color: Color) -> [f32; 4] {
    color.to_linear().map(|channel| channel.clamp(0.0, 1.0))
}

fn valid_color(color: Color) -> Result<[f32; 4], Error> {
    if !color.to_linear().into_iter().all(f32::is_finite) {
        return Err(invalid("non-finite GPU color"));
    }
    Ok(linear(color))
}

fn rounded(rect: Rect, radii: CornerRadii) -> Result<kurbo::BezPath, Error> {
    use kurbo::Shape;
    let [x0, y0, x1, y1] = rect.edges();
    let mut radii = radii.values().map(f64::abs);
    checked([x0, y0, x1, y1].into_iter().chain(radii))?;
    let rect = kurbo::Rect::new(x0, y0, x1, y1).abs();
    let max_radius = rect.width().min(rect.height()) * 0.5;
    if !radii_fit(rect.width(), rect.height(), radii) {
        radii = radii.map(|radius| radius.min(max_radius));
    }
    let [tl, tr, br, bl] = radii;
    if radii.into_iter().all(|radius| radius <= max_radius) {
        return Ok(rect
            .to_rounded_rect(kurbo::RoundedRectRadii::new(tl, tr, br, bl))
            .to_path(0.1));
    }
    // Fitted asymmetric radii can be larger than Kurbo's half-side clamp.
    let mut path = kurbo::BezPath::new();
    path.move_to((rect.x0 + tl, rect.y0));
    for (edge, center, radius, angle) in [
        (
            (rect.x1 - tr, rect.y0),
            (rect.x1 - tr, rect.y0 + tr),
            tr,
            -FRAC_PI_2,
        ),
        (
            (rect.x1, rect.y1 - br),
            (rect.x1 - br, rect.y1 - br),
            br,
            0.0,
        ),
        (
            (rect.x0 + bl, rect.y1),
            (rect.x0 + bl, rect.y1 - bl),
            bl,
            FRAC_PI_2,
        ),
        (
            (rect.x0, rect.y0 + tl),
            (rect.x0 + tl, rect.y0 + tl),
            tl,
            PI,
        ),
    ] {
        path.line_to(edge);
        if radius > 0.0 {
            path.extend(
                kurbo::Arc {
                    center: center.into(),
                    radii: kurbo::Vec2::new(radius, radius),
                    start_angle: angle,
                    sweep_angle: FRAC_PI_2,
                    x_rotation: 0.0,
                }
                .append_iter(0.1),
            );
        }
    }
    path.close_path();
    Ok(path)
}
