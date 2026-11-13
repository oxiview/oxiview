//! CPU rendering adapter for the rendering context.
//!
//! The rasterizer API is unstable. Output can drift between versions, gradient
//! quality is lower than the GPU path, and pixels do not match the GPU renderer
//! byte for byte. Snapshot review guards silent drift; version upgrades require
//! re-reviewing and re-accepting baselines. All rasterizer types stay private.

use kurbo::Shape;
use oxiview_base::{Color, CornerRadii, Rect};
use oxiview_scene::{Brush, Clip, EncodedNode, EncodedScene, NodeId, Primitive, Scene};
use vello_cpu::{Pixmap, RenderContext, Resources, color, kurbo, peniko};

/// Integer output dimensions in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelSize {
    /// Width in pixels; must be in 1..=65535.
    pub width: u32,
    /// Height in pixels; must be in 1..=65535.
    pub height: u32,
}

/// Owned row-major, straight (non-premultiplied) sRGB RGBA8 pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    size: PixelSize,
    pixels: Vec<u8>,
}
impl RgbaImage {
    /// Output width in pixels.
    pub fn width(&self) -> u32 {
        self.size.width
    }
    /// Output height in pixels.
    pub fn height(&self) -> u32 {
        self.size.height
    }
    /// Straight RGBA8 bytes, with transparent pixels normalized to zero RGB.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// Scene encoding or rasterizer size rejection.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The requested scene root could not be encoded.
    #[error("scene encoding failed: {0}")]
    SceneEncoding(String),
    /// Dimensions must be nonzero and fit the rasterizer's u16 dimensions.
    #[error("unsupported CPU render size: {0:?}")]
    InvalidSize(PixelSize),
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
        let encoded = scene
            .encode(root)
            .map_err(|error| Error::SceneEncoding(error.to_string()))?;
        let mut context = RenderContext::new(width, height);
        paint_scene(&mut context, &encoded);
        let mut pixmap = Pixmap::new(width, height);
        context.render(&mut pixmap, &mut Resources::new());
        let pixels = pixmap
            .data_as_u8_slice()
            .chunks_exact(4)
            .flat_map(|pixel| {
                let [r, g, b, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
                [straight(r, a), straight(g, a), straight(b, a), a]
            })
            .collect();
        Ok(RgbaImage { size, pixels })
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

fn straight(channel: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        return 0;
    }
    // Round to nearest; clamp for robustness to premultiplied quantization.
    ((u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha)).min(255) as u8
}

fn path(rect: Rect, radii: CornerRadii) -> kurbo::BezPath {
    let [x0, y0, x1, y1] = rect.edges();
    let [tl, tr, br, bl] = radii.values();
    kurbo::RoundedRect::new(x0, y0, x1, y1, kurbo::RoundedRectRadii::new(tl, tr, br, bl))
        .to_path(0.1)
}

fn solid(value: Color) -> color::AlphaColor<color::Srgb> {
    color::AlphaColor::new(value.to_srgb())
}

fn paint(context: &mut RenderContext, brush: &Brush) {
    match brush {
        Brush::Solid(value) => context.set_paint(solid(*value)),
        Brush::LinearGradient(start, end, stops) => {
            let mut gradient =
                peniko::Gradient::new_linear((start.x(), start.y()), (end.x(), end.y()))
                    .with_interpolation_cs(color::ColorSpaceTag::LinearSrgb);
            gradient.stops.extend(
                stops
                    .iter()
                    .map(|stop| peniko::ColorStop::from((stop.position as f32, solid(stop.color)))),
            );
            context.set_paint(gradient);
        }
    }
}

fn paint_scene(context: &mut RenderContext, scene: &EncodedScene) {
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
                    Primitive::Fill(rect, radii, brush) => {
                        paint(context, brush);
                        context.fill_path(&path(*rect, *radii));
                    }
                    Primitive::Stroke(rect, radii, width, brush) => {
                        paint(context, brush);
                        context.set_stroke(kurbo::Stroke::new(*width));
                        context.stroke_path(&path(*rect, *radii));
                    }
                }
                for _ in clips {
                    context.pop_clip();
                }
                context.set_transform(previous);
            }
        }
    }
}

#[cfg(test)]
mod tests;
