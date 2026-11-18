//! Owned Unicode text layout and DPI-aware glyph rasterization.
//!
//! Parley/fontique/Harfrust perform shaping and fallback. Swash produces cached
//! glyph images consumed by both Scene renderers. Upstream types remain private.

use oxiview_base::{Color, PixelSize, Point, Rect, RgbaImage, Size};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontWeight, Layout, LayoutContext,
    LineHeight, PositionedLayoutItem, StyleProperty,
};
use std::{collections::HashMap, sync::Arc};
use swash::{
    FontRef,
    scale::{Render, ScaleContext, Source, StrikeWith, image::Content},
    zeno::{Format, Vector},
};

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
    layout: Layout<TextBrush>,
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
    /// Logical dimensions including line height.
    pub fn size(&self) -> Size {
        self.size
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
    coords: Vec<i16>,
    x: u8,
    y: u8,
    color: [u8; 4],
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
    font_keys: HashMap<(u64, u32), swash::CacheKey>,
    glyphs: HashMap<GlyphKey, CachedGlyph>,
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
            font_keys: HashMap::new(),
            glyphs: HashMap::new(),
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
        if !style.font_size.is_finite()
            || style.font_size <= 0.0
            || style.font_size > 1024.0
            || !style.line_height.is_finite()
            || style.line_height <= 0.0
            || style.line_height > 16.0
            || !(1..=1000).contains(&style.weight)
            || !style
                .color
                .to_srgb()
                .iter()
                .all(|channel| channel.is_finite())
            || max_width.is_some_and(|width| {
                !width.is_finite() || width < 0.0 || width > f64::from(f32::MAX)
            })
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
        let mut layout: Layout<TextBrush> = builder.build(text);
        layout.set_alignment(
            match style.align {
                TextAlign::Start => Alignment::Start,
                TextAlign::Center => Alignment::Center,
                TextAlign::End => Alignment::End,
            },
            AlignmentOptions::default(),
        );
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
        if glyphs == 0 && text.chars().any(|ch| !ch.is_whitespace()) {
            return Err(Error::MissingFont);
        }
        let size = Size::new(f64::from(layout.width()), f64::from(layout.height()));
        if ![size.width(), size.height()]
            .into_iter()
            .all(f64::is_finite)
        {
            return Err(Error::InvalidInput);
        }
        Ok(TextLayout {
            layout,
            size,
            glyphs,
            missing,
        })
    }

    /// Produces owned glyph-image records at the actual display scale.
    /// Glyphs use grayscale antialiasing with quarter-pixel phase caching.
    pub fn glyph_images(
        &mut self,
        layout: &TextLayout,
        origin: Point,
        scale: f64,
    ) -> Result<Vec<GlyphImage>, Error> {
        if ![origin.x(), origin.y(), scale]
            .into_iter()
            .all(f64::is_finite)
            || scale <= 0.0
            || scale > 16.0
        {
            return Err(Error::InvalidInput);
        }
        let _span = tracing::info_span!("text_raster", glyphs = layout.glyphs, scale).entered();
        let mut output = Vec::new();
        for line in layout.layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                let font = run.font();
                let size = f64::from(run.font_size()) * scale;
                if size > 2048.0 {
                    return Err(Error::RasterLimit);
                }
                let color = glyph_run
                    .style()
                    .brush
                    .0
                    .to_srgb()
                    .map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
                let coords: Vec<i16> = run
                    .normalized_coords()
                    .iter()
                    .map(|coord| coord.to_bits())
                    .collect();
                let mut advance = f64::from(glyph_run.offset());
                for glyph in glyph_run.glyphs() {
                    let x = (origin.x() + advance + f64::from(glyph.x)) * scale;
                    let y =
                        (origin.y() + f64::from(glyph_run.baseline()) + f64::from(glyph.y)) * scale;
                    if !x.is_finite() || !y.is_finite() {
                        return Err(Error::InvalidInput);
                    }
                    advance += f64::from(glyph.advance);
                    let key = GlyphKey {
                        font: font.data.id(),
                        index: font.index,
                        glyph: u16::try_from(glyph.id).map_err(|_| Error::InvalidFont)?,
                        size: (size as f32).to_bits(),
                        coords: coords.clone(),
                        x: ((x - x.floor()) * 4.0).round() as u8,
                        y: ((y - y.floor()) * 4.0).round() as u8,
                        color,
                    };
                    let cached = self.raster_glyph(key, font)?;
                    if let Some(image) = cached.image {
                        let left = (x.floor() + f64::from(cached.left)) / scale;
                        let top = (y.floor() - f64::from(cached.top)) / scale;
                        output.push(GlyphImage {
                            rect: Rect::new(
                                left,
                                top,
                                left + f64::from(image.width()) / scale,
                                top + f64::from(image.height()) / scale,
                            ),
                            image,
                        });
                    }
                }
            }
        }
        Ok(output)
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
        let mut font_ref = FontRef::from_index(font.data.as_ref(), font.index as usize)
            .ok_or(Error::InvalidFont)?;
        font_ref.key = *self.font_keys.entry((key.font, key.index)).or_default();
        let mut scaler = self
            .scales
            .builder(font_ref)
            .size(f32::from_bits(key.size))
            .hint(true)
            .normalized_coords(key.coords.iter().copied())
            .build();
        let image = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ])
        .format(Format::Alpha)
        .offset(Vector::new(
            f32::from(key.x) * 0.25,
            f32::from(key.y) * 0.25,
        ))
        .render(&mut scaler, key.glyph);
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
