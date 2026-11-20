//! CPU rendering adapter for the rendering context.
//!
//! Scene paints composite in premultiplied linear light and frames read back in
//! straight sRGB. All Vello types stay private. Pixel snapshots guard rasterizer
//! changes, including differences in coverage and gradient sampling.

use kurbo::Shape;
use oxiview_base::{Color, CornerRadii, Rect};
use oxiview_scene::{Brush, Clip, EncodedNode, EncodedScene, NodeId, Primitive, Scene};
use std::{
    collections::{HashMap, hash_map::Entry},
    f64::consts::{FRAC_PI_2, PI},
};
use vello_cpu::{LinearImages, RenderContext, Resources, color, kurbo, peniko};

mod glyph;
mod path_fill;

pub use oxiview_scene::{PixelSize, RgbaImage};

/// Scene encoding or rasterizer size rejection.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The requested scene root could not be encoded.
    #[error("scene encoding failed: {0}")]
    SceneEncoding(String),
    /// Dimensions must be nonzero and fit the rasterizer's u16 dimensions.
    #[error("unsupported CPU render size: {0:?}")]
    InvalidSize(PixelSize),
    /// A glyph's font face, outline, or color paint graph could not be rendered.
    #[error("glyph rendering failed: {0}")]
    Glyph(String),
}

/// Renders a caller-selected retained subtree to straight RGBA8 pixels.
pub trait Renderer {
    /// Encodes `root` and paints its nodes in order on a transparent image.
    fn render(&mut self, scene: &Scene, root: NodeId, size: PixelSize) -> Result<RgbaImage, Error>;
}

/// Single-threaded CPU implementation; no GPU or platform resources are needed.
#[derive(Default)]
pub struct CpuRenderer;

impl Renderer for CpuRenderer {
    fn render(&mut self, scene: &Scene, root: NodeId, size: PixelSize) -> Result<RgbaImage, Error> {
        let (width, height) = dimensions(size)?;
        let length = usize::from(width)
            .checked_mul(usize::from(height))
            .and_then(|count| count.checked_mul(4))
            .ok_or(Error::InvalidSize(size))?;
        let encoded = scene
            .encode(root)
            .map_err(|error| Error::SceneEncoding(error.to_string()))?;
        let mut context = RenderContext::new(width, height);
        let mut resources = Resources::new();
        let mut images = LinearImages::default();
        paint_scene(&mut context, &encoded, &mut images)?;
        let mut pixels = vec![0; length];
        if !context.render_linear(&mut pixels, &mut resources, &images) {
            return Err(Error::InvalidSize(size));
        }
        RgbaImage::new(size, pixels).map_err(|e| Error::SceneEncoding(e.to_string()))
    }
}

fn dimensions(size: PixelSize) -> Result<(u16, u16), Error> {
    let width = u16::try_from(size.width).map_err(|_| Error::InvalidSize(size))?;
    let height = u16::try_from(size.height).map_err(|_| Error::InvalidSize(size))?;
    if width == 0 || height == 0 {
        return Err(Error::InvalidSize(size));
    }
    Ok((width, height))
}

fn path(rect: Rect, radii: CornerRadii) -> kurbo::BezPath {
    let [x0, y0, x1, y1] = rect.edges();
    let [tl, tr, br, bl] = radii.values().map(f64::abs);
    let width = (x1 - x0).abs();
    let height = (y1 - y0).abs();
    let fits = tl + tr <= width * (1.0 + f64::EPSILON)
        && bl + br <= width * (1.0 + f64::EPSILON)
        && tl + bl <= height * (1.0 + f64::EPSILON)
        && tr + br <= height * (1.0 + f64::EPSILON);
    if !fits || tl.max(tr).max(br).max(bl) <= width.min(height) / 2.0 {
        return kurbo::RoundedRect::new(
            x0,
            y0,
            x1,
            y1,
            kurbo::RoundedRectRadii::new(tl, tr, br, bl),
        )
        .to_path(0.1);
    }

    // Kurbo's RoundedRect clips each radius to half the shortest side.
    // Fitted asymmetric radii can extend farther along an edge.
    let (left, top, right, bottom) = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
    let mut path = kurbo::BezPath::new();
    path.move_to((left + tl, top));
    for (edge, center, radius, angle) in [
        ((right - tr, top), (right - tr, top + tr), tr, -FRAC_PI_2),
        ((right, bottom - br), (right - br, bottom - br), br, 0.0),
        ((left + bl, bottom), (left + bl, bottom - bl), bl, FRAC_PI_2),
        ((left, top + tl), (left + tl, top + tl), tl, PI),
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
    path
}

fn solid(value: Color) -> color::AlphaColor<color::Srgb> {
    // Vello's paint types use Srgb as a numeric carrier. The float compositor
    // and straight sRGB readback interpret these components as linear.
    color::AlphaColor::new(value.to_linear().map(|channel| channel.clamp(0.0, 1.0)))
}

fn paint(context: &mut RenderContext, brush: &Brush) {
    match brush {
        Brush::Solid(value) => context.set_paint(solid(*value)),
        Brush::LinearGradient(start, end, stops) => {
            // Interpolate the already-linear stop values in the working space.
            let mut gradient =
                peniko::Gradient::new_linear((start.x(), start.y()), (end.x(), end.y()));
            gradient.stops.extend(
                stops
                    .iter()
                    .map(|stop| peniko::ColorStop::from((stop.position as f32, solid(stop.color)))),
            );
            context.set_paint(gradient);
        }
    }
}

fn paint_scene<'a>(
    context: &mut RenderContext,
    scene: &'a EncodedScene,
    images: &mut LinearImages<'a>,
) -> Result<(), Error> {
    let mut image_paints = HashMap::new();
    let mut glyphs = glyph::GlyphPainter::default();
    for node in &scene.nodes {
        match node {
            EncodedNode::BeginOpacity(opacity) => context.push_opacity_layer(*opacity as f32),
            EncodedNode::EndOpacity => context.pop_layer(),
            EncodedNode::Draw {
                primitive,
                transform,
                clips,
            } => {
                let previous = *context.transform();
                for clip in clips {
                    context.set_transform(kurbo::Affine::new(clip.transform.coefficients()));
                    let clip_path = match clip.shape {
                        Clip::Rect(rect) => path(rect, CornerRadii::default()),
                        Clip::RoundedRect(rect, radii) => path(rect, radii),
                    };
                    context.push_clip_path(&clip_path);
                }
                context.set_transform(kurbo::Affine::new(transform.coefficients()));
                match primitive {
                    Primitive::Glyph(glyph) => glyphs.paint(context, images, glyph)?,
                    Primitive::FillPath(path, rule, brush) => {
                        path_fill::paint_path(context, path, *rule, brush)?;
                    }
                    Primitive::Fill(rect, radii, brush) => {
                        paint(context, brush);
                        context.fill_path(&path(*rect, *radii));
                    }
                    Primitive::Stroke(rect, radii, width, brush) => {
                        paint(context, brush);
                        context.set_stroke(kurbo::Stroke::new(*width));
                        context.stroke_path(&path(*rect, *radii));
                    }
                    Primitive::Image(rect, image) => {
                        let key = (
                            image.pixels().as_ptr().addr(),
                            image.width(),
                            image.height(),
                        );
                        let paint = match image_paints.entry(key) {
                            Entry::Occupied(entry) => entry.into_mut(),
                            Entry::Vacant(entry) => {
                                let size = PixelSize {
                                    width: image.width(),
                                    height: image.height(),
                                };
                                let (width, height) = dimensions(size)?;
                                let paint = images
                                    .insert_srgb(width, height, image.pixels())
                                    .ok_or(Error::InvalidSize(size))?;
                                entry.insert(paint)
                            }
                        };
                        context.set_paint(paint.clone());
                        let [left, top, right, bottom] = rect.edges();
                        context.set_paint_transform(kurbo::Affine::new([
                            (right - left) / f64::from(image.width()),
                            0.0,
                            0.0,
                            (bottom - top) / f64::from(image.height()),
                            left,
                            top,
                        ]));
                        context.fill_path(&path(*rect, CornerRadii::default()));
                        context.reset_paint_transform();
                    }
                }
                for _ in clips {
                    context.pop_clip();
                }
                context.set_transform(previous);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
