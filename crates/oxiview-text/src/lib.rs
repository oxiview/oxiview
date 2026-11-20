//! Owned Unicode text layout and DPI-aware glyph drawing.
//!
//! Parley/fontique/Harfrust perform shaping and fallback. Swash produces cached
//! glyph images for hinted small text and embedded bitmap strikes.
//! Large or transformed glyphs carry owned font data or synthesized outlines.

mod drawing;

pub use drawing::{GlyphDraw, TextDecoration, TextDecorationKind};

use oxiview_base::{Color, PixelSize, Rect, RgbaImage, Size};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontStyle, FontWeight, Layout,
    LayoutContext, LineHeight, PositionedLayoutItem, StyleProperty,
};
use std::{collections::HashMap, sync::Arc};
use swash::{
    FontRef,
    scale::{Render, ScaleContext, Source, StrikeWith, image::Content},
    zeno::{Angle, Format, Transform, Vector},
};

mod rich;
pub use rich::{RichText, TextSpan, TextSpanStyle};
use rich::{valid_color, valid_font_size, valid_line_height, valid_weight};

const EMBOLDEN_EM: f32 = 0.03;

fn oblique_transform(degrees: f32) -> Transform {
    Transform::skew(Angle::from_degrees(degrees), Angle::from_degrees(0.0))
}

/// Direction-aware paragraph alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    /// Paragraph starting edge.
    #[default]
    Start,
    /// Center each line.
    Center,
    /// Paragraph ending edge.
    End,
}

/// Owned static paragraph properties, measured in logical units.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    /// Logical font size.
    pub font_size: f64,
    /// Monochrome glyph foreground; color glyphs retain their own RGB.
    pub color: Color,
    /// Preferred family; None selects the system UI family. Missing glyphs fall back.
    pub family: Option<String>,
    /// OpenType weight from 1 through 1000.
    pub weight: u16,
    /// Line-height multiplier relative to font size.
    pub line_height: f64,
    /// Direction-aware alignment.
    pub align: TextAlign,
}
impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_size: 16.0,
            color: Color::from_rgba8(0, 0, 0, 255),
            family: None,
            weight: 400,
            line_height: 1.25,
            align: TextAlign::Start,
        }
    }
}

/// Text validation, font, or rasterization failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Non-finite or out-of-range style, width, origin, or scale.
    #[error("invalid text style, width, position, or scale")]
    InvalidInput,
    /// A range splits a UTF-8 code point, overlaps another span, or is empty or out of bounds.
    #[error("invalid text span {start}..{end}")]
    InvalidSpan {
        /// Inclusive byte offset.
        start: usize,
        /// Exclusive byte offset.
        end: usize,
    },
    /// The number of spans exceeds the supported per-paragraph limit.
    #[error("too many text spans")]
    TooManySpans,
    /// Font data contains no usable face.
    #[error("invalid font data")]
    InvalidFont,
    /// Font discovery supplied no glyph run for nonempty text.
    #[error("no usable font found for text")]
    MissingFont,
    /// A glyph exceeded the supported raster dimensions.
    #[error("glyph raster size exceeds the supported limit")]
    RasterLimit,
    /// Raster data and placement disagree.
    #[error("invalid glyph image data")]
    InvalidGlyphImage,
    /// The font could not supply a valid outline for a synthesized glyph.
    #[error("invalid glyph outline")]
    InvalidGlyphOutline,
    /// Owned foundation image conversion failed.
    #[error("text image conversion failed: {0}")]
    Image(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TextBrush(Color);
impl Default for TextBrush {
    fn default() -> Self {
        Self(Color::from_rgba8(0, 0, 0, 255))
    }
}

/// Shaped paragraph with private font/run storage and logical dimensions.
#[derive(Clone)]
pub struct TextLayout {
    layout: Arc<Layout<TextBrush>>,
    size: Size,
    glyphs: usize,
    missing: usize,
}

/// One rasterized glyph positioned in logical local coordinates.
/// Core/rendering consumers may turn this owned record into their draw language.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphImage {
    /// Destination rectangle, including the glyph's bearings and raster phase.
    pub rect: Rect,
    /// Immutable shared straight-sRGB RGBA8 pixels at the requested display scale.
    pub image: RgbaImage,
}
impl TextLayout {
    /// Reuses shaped glyphs and styles to lay out the paragraph at a new width.
    pub fn rewrap(&self, max_width: Option<f64>) -> Result<Self, Error> {
        if !valid_width(max_width) {
            return Err(Error::InvalidInput);
        }
        Self::wrap((*self.layout).clone(), max_width)
    }

    fn wrap(mut layout: Layout<TextBrush>, max_width: Option<f64>) -> Result<Self, Error> {
        layout.break_all_lines(max_width.map(|width| width as f32));
        let mut glyphs = 0;
        let mut missing = 0;
        for line in layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    for glyph in run.glyphs() {
                        glyphs += 1;
                        missing += usize::from(glyph.id == 0);
                    }
                }
            }
        }
        let size = Size::new(f64::from(layout.width()), f64::from(layout.height()));
        if !size.width().is_finite() || !size.height().is_finite() {
            return Err(Error::InvalidInput);
        }
        Ok(Self {
            layout: Arc::new(layout),
            size,
            glyphs,
            missing,
        })
    }

    /// Logical dimensions including line height.
    pub fn size(&self) -> Size {
        self.size
    }
    /// First-line baseline measured down from the top of the paragraph.
    /// An empty source has no text baseline.
    pub fn first_baseline(&self) -> Option<f64> {
        if self.layout.text_len() == 0 {
            return None;
        }
        self.layout.first_baseline().map(f64::from)
    }
    /// Number of laid-out lines.
    pub fn line_count(&self) -> usize {
        self.layout.len()
    }
    /// Shaped glyph count, which may differ from Unicode scalar count.
    pub fn glyph_count(&self) -> usize {
        self.glyphs
    }
    /// Remaining .notdef glyph count after fallback.
    pub fn missing_glyph_count(&self) -> usize {
        self.missing
    }
}

fn valid_width(width: Option<f64>) -> bool {
    width.is_none_or(|width| (0.0..=f64::from(f32::MAX)).contains(&width))
}

/// Bounded glyph-cache counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextCacheStats {
    /// Reused glyph entries.
    pub hits: u64,
    /// Requested rasterizations.
    pub misses: u64,
    /// Retained pixel bytes.
    pub bytes: usize,
    /// Retained entries, including whitespace without images.
    pub entries: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: u64,
    index: u32,
    glyph: u16,
    size: u32,
    coords: Arc<[i16]>,
    skew: Option<u32>,
    embolden: bool,
    x: u8,
    y: u8,
    color: [u8; 4],
    source: RasterSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RasterSource {
    Auto,
    ColorBitmap(u32),
    AlphaBitmap(u32),
}

#[derive(Clone)]
struct CachedGlyph {
    image: Option<RgbaImage>,
    left: i32,
    top: i32,
    last_used: u64,
}

/// Reusable window-thread font, shaping, rasterization, and bounded LRU resources.
pub struct TextSystem {
    fonts: FontContext,
    layouts: LayoutContext<TextBrush>,
    scales: ScaleContext,
    glyphs: HashMap<GlyphKey, CachedGlyph>,
    vector_threshold: f64,
    budget: usize,
    tick: u64,
    stats: TextCacheStats,
}
impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
    }
}
impl TextSystem {
    /// Discovers system fonts and creates an 8 MiB glyph cache.
    pub fn new() -> Self {
        Self::with_fonts(FontContext::new())
    }
    /// Creates deterministic font resources containing only explicitly registered fonts.
    pub fn without_system_fonts() -> Self {
        Self::with_fonts(FontContext {
            collection: parley::fontique::Collection::new(parley::fontique::CollectionOptions {
                shared: false,
                system_fonts: false,
            }),
            source_cache: parley::fontique::SourceCache::default(),
        })
    }
    fn with_fonts(fonts: FontContext) -> Self {
        Self {
            fonts,
            layouts: LayoutContext::new(),
            scales: ScaleContext::new(),
            glyphs: HashMap::new(),
            vector_threshold: 64.0,
            budget: 8 * 1024 * 1024,
            tick: 0,
            stats: TextCacheStats::default(),
        }
    }
    /// Registers owned font bytes and returns the number of registered faces.
    pub fn register_font(&mut self, bytes: Vec<u8>) -> Result<usize, Error> {
        let count = self
            .fonts
            .collection
            .register_fonts(parley::fontique::Blob::new(Arc::new(bytes)), None)
            .iter()
            .map(|(_, fonts)| fonts.len())
            .sum();
        if count == 0 {
            Err(Error::InvalidFont)
        } else {
            Ok(count)
        }
    }
    /// Limits retained glyph pixels; existing Scene images retain their shared storage.
    pub fn set_cache_budget(&mut self, bytes: usize) {
        self.budget = bytes;
        self.evict();
    }
    /// Reads cache counters without exposing upstream types.
    pub fn cache_stats(&self) -> TextCacheStats {
        self.stats
    }

    /// Shapes Unicode, performs font fallback, and wraps in logical units.
    pub fn layout(
        &mut self,
        text: &str,
        style: &TextStyle,
        max_width: Option<f64>,
    ) -> Result<TextLayout, Error> {
        self.layout_spans(text, &[], style, max_width)
    }

    /// Shapes and wraps a paragraph with validated per-span overrides.
    pub fn layout_rich(
        &mut self,
        text: &RichText,
        style: &TextStyle,
        max_width: Option<f64>,
    ) -> Result<TextLayout, Error> {
        self.layout_spans(text.as_str(), text.spans(), style, max_width)
    }

    fn layout_spans(
        &mut self,
        text: &str,
        spans: &[TextSpan],
        style: &TextStyle,
        max_width: Option<f64>,
    ) -> Result<TextLayout, Error> {
        if !valid_font_size(style.font_size)
            || !valid_line_height(style.line_height)
            || !valid_weight(style.weight)
            || !valid_color(style.color)
            || !valid_width(max_width)
        {
            return Err(Error::InvalidInput);
        }
        let _span = tracing::info_span!("text_layout", bytes = text.len()).entered();
        let mut builder = self
            .layouts
            .ranged_builder(&mut self.fonts, text, 1.0, false);
        builder.push_default(StyleProperty::FontSize(style.font_size as f32));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(f32::from(
            style.weight,
        ))));
        builder.push_default(StyleProperty::FontFamily(FontFamily::from(
            style.family.as_deref().unwrap_or("system-ui"),
        )));
        builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(
            style.line_height as f32,
        )));
        builder.push_default(StyleProperty::Brush(TextBrush(style.color)));
        for span in spans {
            let range = &span.range;
            let style = &span.style;
            if let Some(family) = &style.family {
                builder.push(
                    StyleProperty::FontFamily(FontFamily::from(family.as_str())),
                    range.clone(),
                );
            }
            if let Some(size) = style.font_size {
                builder.push(StyleProperty::FontSize(size as f32), range.clone());
            }
            if let Some(weight) = style.weight {
                builder.push(
                    StyleProperty::FontWeight(FontWeight::new(f32::from(weight))),
                    range.clone(),
                );
            }
            if let Some(color) = style.color {
                builder.push(StyleProperty::Brush(TextBrush(color)), range.clone());
            }
            if style.italic {
                builder.push(StyleProperty::FontStyle(FontStyle::Italic), range.clone());
            }
            if style.underline {
                builder.push(StyleProperty::Underline(true), range.clone());
            }
            if style.strikethrough {
                builder.push(StyleProperty::Strikethrough(true), range.clone());
            }
            if let Some(spacing) = style.letter_spacing {
                builder.push(StyleProperty::LetterSpacing(spacing as f32), range.clone());
            }
            if let Some(spacing) = style.word_spacing {
                builder.push(StyleProperty::WordSpacing(spacing as f32), range.clone());
            }
            if let Some(height) = style.line_height {
                builder.push(
                    StyleProperty::LineHeight(LineHeight::FontSizeRelative(height as f32)),
                    range.clone(),
                );
            }
        }
        let mut layout: Layout<TextBrush> = builder.build(text);
        layout.set_alignment(
            match style.align {
                TextAlign::Start => Alignment::Start,
                TextAlign::Center => Alignment::Center,
                TextAlign::End => Alignment::End,
            },
            AlignmentOptions::default(),
        );
        let layout = TextLayout::wrap(layout, max_width)?;
        if layout.glyph_count() == 0 && text.chars().any(|ch| !ch.is_whitespace()) {
            return Err(Error::MissingFont);
        }
        Ok(layout)
    }

    fn raster_glyph(
        &mut self,
        key: GlyphKey,
        font: &parley::FontData,
    ) -> Result<CachedGlyph, Error> {
        self.tick = self.tick.saturating_add(1);
        if let Some(value) = self.glyphs.get_mut(&key) {
            value.last_used = self.tick;
            self.stats.hits = self.stats.hits.saturating_add(1);
            return Ok(value.clone());
        }
        self.stats.misses = self.stats.misses.saturating_add(1);
        let font_ref = FontRef::from_index(font.data.as_ref(), font.index as usize)
            .ok_or(Error::InvalidFont)?;
        let mut scaler = self
            .scales
            // Fontique already gives each blob a unique ID. Swash's bounded
            // scaler and hinting caches can use that ID and the collection index.
            .builder_with_id(font_ref, [key.font, u64::from(key.index)])
            .size(f32::from_bits(key.size))
            .hint(key.source == RasterSource::Auto)
            .normalized_coords(key.coords.iter().copied())
            .build();
        let image = match key.source {
            RasterSource::Auto => Render::new(&[
                Source::ColorOutline(0),
                Source::ColorBitmap(StrikeWith::BestFit),
                Source::Outline,
            ])
            .format(Format::Alpha)
            .transform(key.skew.map(f32::from_bits).map(oblique_transform))
            .embolden(if key.embolden {
                f32::from_bits(key.size) * EMBOLDEN_EM
            } else {
                0.0
            })
            .offset(Vector::new(
                f32::from(key.x) * 0.25,
                f32::from(key.y) * 0.25,
            ))
            .render(&mut scaler, key.glyph),
            RasterSource::ColorBitmap(index) => Some(
                scaler
                    .scale_color_bitmap(key.glyph, StrikeWith::Index(index))
                    .ok_or(Error::InvalidGlyphImage)?,
            ),
            RasterSource::AlphaBitmap(index) => Some(
                scaler
                    .scale_bitmap(key.glyph, StrikeWith::Index(index))
                    .ok_or(Error::InvalidGlyphImage)?,
            ),
        };
        let value = if let Some(image) = image {
            let pixels = u64::from(image.placement.width) * u64::from(image.placement.height);
            if pixels > 4 * 1024 * 1024 {
                return Err(Error::RasterLimit);
            }
            let mut rgba = Vec::new();
            match image.content {
                Content::Mask => {
                    if image.data.len() as u64 != pixels {
                        return Err(Error::InvalidGlyphImage);
                    }
                    for alpha in image.data {
                        rgba.extend([
                            key.color[0],
                            key.color[1],
                            key.color[2],
                            alpha_product(alpha, key.color[3]),
                        ]);
                    }
                }
                Content::Color | Content::SubpixelMask => {
                    if image.data.len() as u64 != pixels * 4 {
                        return Err(Error::InvalidGlyphImage);
                    }
                    for pixel in image.data.chunks_exact(4) {
                        let [r, g, b, a] = *pixel else {
                            return Err(Error::InvalidGlyphImage);
                        };
                        if image.content == Content::Color {
                            // Swash composites COLR outline layers into premultiplied
                            // RGBA; bitmap PNG strikes are straight RGBA. The Scene
                            // boundary always owns straight-sRGB pixels.
                            let [r, g, b] = if matches!(image.source, Source::ColorOutline(_)) {
                                [
                                    unpremultiply(r, a),
                                    unpremultiply(g, a),
                                    unpremultiply(b, a),
                                ]
                            } else {
                                [r, g, b]
                            };
                            rgba.extend([r, g, b, alpha_product(a, key.color[3])]);
                        } else {
                            rgba.extend([
                                key.color[0],
                                key.color[1],
                                key.color[2],
                                alpha_product(r.max(g).max(b), key.color[3]),
                            ]);
                        }
                    }
                }
            }
            // Empty outlines such as spaces can acquire a nonzero bounding box
            // from a fractional raster phase. Keep their logical advance, but
            // do not emit a transparent draw or vary draw count with DPI.
            let has_ink = rgba
                .chunks_exact(4)
                .any(|pixel| matches!(pixel, [_, _, _, alpha] if *alpha != 0));
            let glyph = if pixels == 0 || !has_ink {
                None
            } else {
                Some(
                    RgbaImage::new(
                        PixelSize {
                            width: image.placement.width,
                            height: image.placement.height,
                        },
                        rgba,
                    )
                    .map_err(|error| Error::Image(error.to_string()))?,
                )
            };
            CachedGlyph {
                image: glyph,
                left: image.placement.left,
                top: image.placement.top,
                last_used: self.tick,
            }
        } else {
            CachedGlyph {
                image: None,
                left: 0,
                top: 0,
                last_used: self.tick,
            }
        };
        self.stats.bytes = self
            .stats
            .bytes
            .saturating_add(value.image.as_ref().map_or(0, |image| image.pixels().len()));
        self.glyphs.insert(key, value.clone());
        self.evict();
        Ok(value)
    }
    fn evict(&mut self) {
        while self.stats.bytes > self.budget || self.glyphs.len() > 4096 {
            let Some(key) = self
                .glyphs
                .iter()
                .min_by_key(|(_, value)| value.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(value) = self.glyphs.remove(&key) {
                self.stats.bytes = self
                    .stats
                    .bytes
                    .saturating_sub(value.image.as_ref().map_or(0, |image| image.pixels().len()));
            }
        }
        self.stats.entries = self.glyphs.len();
    }
}
fn alpha_product(a: u8, b: u8) -> u8 {
    ((u16::from(a) * u16::from(b) + 127) / 255) as u8
}
fn unpremultiply(channel: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        0
    } else {
        ((u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha)).min(255) as u8
    }
}
