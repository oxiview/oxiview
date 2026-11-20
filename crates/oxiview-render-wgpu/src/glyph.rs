use crate::{
    Error,
    vector::{checked, invalid},
};
use glifo::{
    AtlasCacher, AtlasPaint, AtlasSlot, DrawSink, GlyphPrepCache, GlyphRenderError, GlyphRenderer,
    GlyphRun, GlyphRunBackend, GlyphRunBuilder,
};
use oxiview_base::{Glyph, GlyphFont};
use peniko::color::{AlphaColor, LinearSrgb, Srgb};
use skrifa::{
    FontRef, GlyphId, MetadataProvider,
    color::{Brush, ColorPainter, CompositeMode, Transform},
    instance::{LocationRef, NormalizedCoord, Size},
    outline::{DrawSettings, OutlinePen},
    raw::{TableProvider, tables::cpal::ColorRecord, types::BoundingBox},
};
use std::{collections::HashMap, ops::RangeInclusive, sync::Arc};
use vello_gpu::{Image, ImageSource, PaintType, RenderState, Tint, kurbo, peniko};

mod colr;
mod hard_stop;
mod normalize;
#[cfg(test)]
mod tests;

const MAX_NORMALIZED_FONTS: usize = 128;
const MAX_NORMALIZED_FONT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Eq, Hash, PartialEq)]
struct FontKey {
    allocation: usize,
    byte_len: usize,
    face_index: u32,
    // Used only when the private normalized font depends on the location.
    variations: Option<Arc<[i16]>>,
}

#[derive(Default)]
pub(crate) struct GlyphPainter {
    fonts: HashMap<FontKey, peniko::Blob<u8>>,
    font_bytes: usize,
    cache: GlyphPrepCache,
}

impl GlyphPainter {
    pub(crate) fn maintain(&mut self) {
        self.cache.maintain();
    }

    pub(crate) fn outline(
        &self,
        glyph: &Glyph,
        transform: kurbo::Affine,
    ) -> Result<(kurbo::BezPath, kurbo::Affine), Error> {
        let placement = placement(glyph, transform)?;
        let face = face(glyph)?;
        let coords = coordinates(glyph);
        let outline = face
            .outline_glyphs()
            .get(GlyphId::from(glyph.glyph_id))
            .ok_or_else(|| invalid("outline glyph is missing"))?;
        let mut pen = PathPen::default();
        outline
            .draw(
                DrawSettings::unhinted(Size::new(glyph.size), LocationRef::new(&coords)),
                &mut pen,
            )
            .map_err(glyph_error)?;
        Ok((
            pen.0,
            placement * kurbo::Affine::scale_non_uniform(1.0, -1.0),
        ))
    }

    pub(crate) fn color(
        &mut self,
        scene: &mut vello_gpu::Scene,
        glyph: &Glyph,
        transform: kurbo::Affine,
    ) -> Result<(), Error> {
        let placement = placement(glyph, transform)?;
        let foreground =
            AlphaColor::<Srgb>::new(glyph.color.to_srgb().map(|channel| channel.clamp(0.0, 1.0)));
        let opacity = foreground.components[3];
        if opacity == 0.0 {
            return Ok(());
        }

        let coords = coordinates(glyph);
        let (blob, cached) = self.color_font(glyph, &coords)?;
        let face = FontRef::from_index(blob.as_ref(), glyph.face_index).map_err(glyph_error)?;
        let color_glyphs = face.color_glyphs();
        let color_glyph = color_glyphs
            .get(GlyphId::from(glyph.glyph_id))
            .ok_or_else(|| invalid("COLR glyph is missing"))?;
        // Validate the same graph Glifo will replay, including any normalized
        // color lines, before it can index into the palette or gradient stops.
        let palettes = face.color_palettes();
        let palette = palettes
            .get(0)
            .ok_or_else(|| invalid("color glyph has no usable palette"))?;
        let mut check = ColorPaintCheck {
            palette_len: palette.colors().len(),
            valid: true,
            needs_replay: false,
        };
        color_glyph
            .paint(LocationRef::new(&coords), &mut check)
            .map_err(glyph_error)?;
        if !check.valid {
            return Err(invalid("invalid COLR palette index or color line"));
        }
        let upem = face.head().map_err(glyph_error)?.units_per_em();

        // Isolate the graph from unrelated Scene pixels; multiply glyph opacity
        // once after all internal paint nodes and composite modes have executed.
        let saved = scene.take_current_state();
        scene.push_opacity_layer(opacity);
        let foreground = foreground.with_alpha(1.0);
        let result = if check.needs_replay {
            let scale = Size::new(glyph.size).linear_scale(upem);
            let placement =
                placement * kurbo::Affine::scale_non_uniform(f64::from(scale), -f64::from(scale));
            colr::paint(
                scene,
                &face,
                glyph,
                LocationRef::new(&coords),
                placement,
                foreground,
                palette.colors(),
            )
        } else {
            let mut transient = GlyphPrepCache::default();
            let cache = if cached {
                &mut self.cache
            } else {
                &mut transient
            };
            Self::paint_glifo(scene, glyph, placement, foreground, blob, cache)
        };
        scene.pop_layer();
        scene.restore_state(saved);
        result
    }

    fn paint_glifo(
        scene: &mut vello_gpu::Scene,
        glyph: &Glyph,
        placement: kurbo::Affine,
        foreground: AlphaColor<Srgb>,
        blob: peniko::Blob<u8>,
        cache: &mut GlyphPrepCache,
    ) -> Result<(), Error> {
        let font = peniko::FontData::new(blob, glyph.face_index);
        let result = GlyphRunBuilder::new(
            font,
            placement,
            kurbo::Affine::IDENTITY,
            ColorRun {
                sink: LinearSink { scene, foreground },
                cache,
            },
        )
        .font_size(glyph.size)
        .hint(false)
        .normalized_coords(&glyph.variations)
        .fill_glyphs(std::iter::once(glifo::Glyph {
            id: u32::from(glyph.glyph_id),
            x: 0.0,
            y: 0.0,
        }));
        result.map_err(glyph_error)
    }

    fn color_font(
        &mut self,
        glyph: &Glyph,
        coords: &[NormalizedCoord],
    ) -> Result<(peniko::Blob<u8>, bool), Error> {
        let mut key = FontKey {
            allocation: glyph.font.bytes().as_ptr().addr(),
            byte_len: glyph.font.bytes().len(),
            face_index: glyph.face_index,
            variations: None,
        };
        if let Some(blob) = self.fonts.get(&key) {
            return Ok((blob.clone(), true));
        }
        key.variations = Some(glyph.variations.clone());
        if let Some(blob) = self.fonts.get(&key) {
            return Ok((blob.clone(), true));
        }
        let face = face(glyph)?;
        let palettes = face.color_palettes();
        let palette = palettes
            .get(0)
            .ok_or_else(|| invalid("color glyph has no usable palette"))?;
        let (source, varies) = FontSource::new(&glyph.font, &face, palette.colors(), coords)?;
        if !varies {
            key.variations = None;
        }
        // The blob owns the source allocation even when it reads adjusted
        // bytes. Keeping that ownership protects the allocation-based key.
        // Charge the original for every entry, including shared originals.
        let bytes = source.retained_bytes().saturating_add(
            key.variations
                .as_ref()
                .map_or(0, |coords| std::mem::size_of_val(coords.as_ref())),
        );
        let blob = peniko::Blob::new(Arc::new(source));
        if bytes > MAX_NORMALIZED_FONT_BYTES {
            return Ok((blob, false));
        }
        if self.fonts.len() >= MAX_NORMALIZED_FONTS
            || bytes > MAX_NORMALIZED_FONT_BYTES - self.font_bytes
        {
            self.fonts.clear();
            self.font_bytes = 0;
            self.cache.clear();
        }
        self.font_bytes += bytes;
        self.fonts.insert(key, blob.clone());
        Ok((blob, true))
    }
}

struct FontSource {
    font: GlyphFont,
    adjusted: Option<Vec<u8>>,
}

impl FontSource {
    fn retained_bytes(&self) -> usize {
        self.font
            .bytes()
            .len()
            .saturating_add(self.adjusted.as_ref().map_or(0, Vec::capacity))
    }

    fn new(
        font: &GlyphFont,
        face: &FontRef<'_>,
        palette: &[ColorRecord],
        coords: &[NormalizedCoord],
    ) -> Result<(Self, bool), Error> {
        let cpal = face.cpal().map_err(glyph_error)?;
        let first = cpal
            .color_record_indices()
            .first()
            .ok_or_else(|| invalid("CPAL has no default palette"))?;
        let mut source = Self {
            font: font.clone(),
            adjusted: None,
        };
        if first.get() != 0 && !palette.is_empty() {
            // Glifo 0.5 looks up palette indices from the start of the CPAL
            // record array. Present palette zero there in its private FontData.
            let records = cpal
                .color_records_array()
                .ok_or_else(|| invalid("CPAL has no color records"))?
                .map_err(glyph_error)?;
            let base = font.bytes().as_ptr().addr();
            let dest = records
                .as_ptr()
                .addr()
                .checked_sub(base)
                .ok_or_else(|| invalid("invalid CPAL color records"))?;
            let origin = palette
                .as_ptr()
                .addr()
                .checked_sub(base)
                .ok_or_else(|| invalid("invalid CPAL default palette"))?;
            let index = std::ptr::from_ref(first)
                .addr()
                .checked_sub(base)
                .ok_or_else(|| invalid("invalid CPAL palette offset"))?;
            let len = std::mem::size_of_val(palette);
            let mut bytes = font.bytes().to_vec();
            if len > bytes.len().saturating_sub(origin)
                || len > bytes.len().saturating_sub(dest)
                || 2 > bytes.len().saturating_sub(index)
            {
                return Err(invalid("CPAL palette exceeds font data"));
            }
            bytes.copy_within(origin..origin + len, dest);
            // Skrifa also reads the private font during radial replay. The
            // old palette range may now overlap the relocated color records.
            bytes[index..index + 2].fill(0);
            source.adjusted = Some(bytes);
        }
        let varies = normalize::gradients(face, coords, font.bytes(), &mut source.adjusted)?;
        Ok((source, varies))
    }
}

impl AsRef<[u8]> for FontSource {
    fn as_ref(&self) -> &[u8] {
        self.adjusted.as_deref().unwrap_or(self.font.bytes())
    }
}

fn placement(glyph: &Glyph, transform: kurbo::Affine) -> Result<kurbo::Affine, Error> {
    let placement = transform * kurbo::Affine::translate((glyph.origin.x(), glyph.origin.y()));
    checked(placement.as_coeffs())?;
    Ok(placement)
}

fn face(glyph: &Glyph) -> Result<FontRef<'_>, Error> {
    let face = FontRef::from_index(glyph.font.bytes(), glyph.face_index).map_err(glyph_error)?;
    if face.head().map_err(glyph_error)?.units_per_em() == 0 {
        return Err(invalid("font has zero units per em"));
    }
    Ok(face)
}

fn coordinates(glyph: &Glyph) -> Vec<NormalizedCoord> {
    glyph
        .variations
        .iter()
        .copied()
        .map(NormalizedCoord::from_bits)
        .collect()
}

fn glyph_error(error: impl std::fmt::Display) -> Error {
    Error::SceneEncoding(format!("glyph paint failed: {error}"))
}

#[derive(Default)]
struct PathPen(kurbo::BezPath);

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), f64::from(y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), f64::from(y)));
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0
            .quad_to((f64::from(cx), f64::from(cy)), (f64::from(x), f64::from(y)));
    }

    fn curve_to(&mut self, c0x: f32, c0y: f32, c1x: f32, c1y: f32, x: f32, y: f32) {
        self.0.curve_to(
            (f64::from(c0x), f64::from(c0y)),
            (f64::from(c1x), f64::from(c1y)),
            (f64::from(x), f64::from(y)),
        );
    }

    fn close(&mut self) {
        self.0.close_path();
    }
}

struct ColorPaintCheck {
    palette_len: usize,
    valid: bool,
    needs_replay: bool,
}

impl ColorPainter for ColorPaintCheck {
    fn push_transform(&mut self, _: Transform) {}
    fn pop_transform(&mut self) {}
    fn push_clip_glyph(&mut self, _: GlyphId) {}
    fn push_clip_box(&mut self, _: BoundingBox<f32>) {}
    fn pop_clip(&mut self) {}
    fn push_layer(&mut self, _: CompositeMode) {}
    fn pop_layer(&mut self) {}

    fn fill(&mut self, brush: Brush<'_>) {
        if let Brush::RadialGradient { c0, r0, c1, r1, .. } = brush {
            // Glifo flattens these circles, and Vello's default degenerate
            // gradient paint is a solid. The fallback keeps the COLR domain.
            self.needs_replay |= r1 <= 0.0 || r0 < 0.0 || (r0 == r1 && c0 == c1);
        }
        let valid_index = |index| index == u16::MAX || usize::from(index) < self.palette_len;
        self.valid &= match brush {
            Brush::Solid { palette_index, .. } => valid_index(palette_index),
            Brush::LinearGradient { color_stops, .. }
            | Brush::RadialGradient { color_stops, .. }
            | Brush::SweepGradient { color_stops, .. } => {
                !color_stops.is_empty()
                    && color_stops
                        .iter()
                        .all(|stop| valid_index(stop.palette_index))
            }
        };
    }

    fn fill_glyph(&mut self, id: GlyphId, _: Option<Transform>, brush: Brush<'_>) {
        if id == hard_stop::MARKER_GLYPH {
            self.needs_replay = true;
        } else {
            self.fill(brush);
        }
    }
}

struct ColorRun<'a> {
    sink: LinearSink<'a>,
    cache: &'a mut GlyphPrepCache,
}

impl<'a> GlyphRunBackend<'a> for ColorRun<'a> {
    fn atlas_cache(self, _enabled: bool) -> Self {
        self
    }

    fn fill_glyphs<G: Iterator<Item = glifo::Glyph> + Clone>(
        mut self,
        run: GlyphRun<'a>,
        glyphs: G,
    ) -> Result<(), GlyphRenderError> {
        run.build(glyphs, self.cache.as_mut(), AtlasCacher::Disabled)
            .fill_glyphs(&mut self.sink)
    }

    fn stroke_glyphs<G: Iterator<Item = glifo::Glyph> + Clone>(
        mut self,
        run: GlyphRun<'a>,
        glyphs: G,
    ) -> Result<(), GlyphRenderError> {
        run.build(glyphs, self.cache.as_mut(), AtlasCacher::Disabled)
            .stroke_glyphs(&mut self.sink)
    }

    fn render_decoration<G: Iterator<Item = glifo::Glyph> + Clone>(
        mut self,
        run: GlyphRun<'a>,
        glyphs: G,
        x: RangeInclusive<f32>,
        baseline: f32,
        offset: f32,
        size: f32,
        buffer: f32,
    ) {
        run.build(glyphs, self.cache.as_mut(), AtlasCacher::Disabled)
            .render_decoration(x, baseline, offset, size, buffer, &mut self.sink);
    }
}

/// Glifo supplies the COLR graph geometry, transforms and blend operators.
/// Feed Vello linear channel values for both palette paints and their stops.
/// Vello uses the Srgb type for its numeric blending space without gamma decoding.
struct LinearSink<'a> {
    scene: &'a mut vello_gpu::Scene,
    foreground: AlphaColor<Srgb>,
}

fn linear_channels(color: AlphaColor<Srgb>) -> AlphaColor<Srgb> {
    AlphaColor::new(color.convert::<LinearSrgb>().components)
}

impl DrawSink for LinearSink<'_> {
    fn set_transform(&mut self, transform: kurbo::Affine) {
        self.scene.set_transform(transform);
    }
    fn set_paint(&mut self, paint: AtlasPaint) {
        match paint {
            AtlasPaint::Solid(color) => self.scene.set_paint(linear_channels(color)),
            AtlasPaint::Gradient(mut gradient) => {
                gradient.interpolation_cs = peniko::color::ColorSpaceTag::Srgb;
                gradient.interpolation_alpha_space = peniko::InterpolationAlphaSpace::Premultiplied;
                for stop in gradient.stops.iter_mut() {
                    stop.color = peniko::color::DynamicColor::from_alpha_color(linear_channels(
                        stop.color.to_alpha_color(),
                    ));
                }
                self.scene.set_paint(gradient);
            }
        }
    }
    fn set_paint_transform(&mut self, transform: kurbo::Affine) {
        self.scene.set_paint_transform(transform);
    }
    fn fill_path(&mut self, path: &kurbo::BezPath) {
        self.scene.fill_path(path);
    }
    fn fill_rect(&mut self, rect: &kurbo::Rect) {
        self.scene.fill_rect(rect);
    }
    fn push_clip_layer(&mut self, clip: &kurbo::BezPath) {
        self.scene.push_clip_layer(clip);
    }
    fn push_clip_path(&mut self, clip: &kurbo::BezPath) {
        self.scene.push_clip_path(clip);
    }
    fn push_clip_rect(&mut self, clip: &kurbo::Rect) {
        self.scene.push_clip_rect(clip);
    }
    fn push_blend_layer(&mut self, blend: peniko::BlendMode) {
        self.scene.push_blend_layer(blend);
    }
    fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }
    fn pop_clip_path(&mut self) {
        self.scene.pop_clip();
    }
    fn width(&self) -> u16 {
        self.scene.width()
    }
    fn height(&self) -> u16 {
        self.scene.height()
    }
}

impl GlyphRenderer for LinearSink<'_> {
    type SavedState = RenderState;

    fn save_state(&mut self) -> Self::SavedState {
        self.scene.save_current_state()
    }
    fn restore_state(&mut self, state: Self::SavedState) {
        self.scene.restore_state(state);
    }
    fn stroke_path(&mut self, path: &kurbo::BezPath) {
        self.scene.stroke_path(path);
    }
    fn set_paint_image(&mut self, image: Image) {
        self.scene.set_paint(image);
    }
    fn set_tint(&mut self, tint: Option<Tint>) {
        self.scene.set_tint(tint);
    }
    fn get_context_color(&self) -> AlphaColor<Srgb> {
        self.foreground
    }
    fn current_paint(&self) -> &PaintType {
        self.scene.paint()
    }
    fn atlas_image_source(&self, slot: &AtlasSlot) -> ImageSource {
        GlyphRenderer::atlas_image_source(self.scene, slot)
    }
    fn atlas_paint_transform(&self, slot: &AtlasSlot) -> kurbo::Affine {
        GlyphRenderer::atlas_paint_transform(self.scene, slot)
    }
}
