use super::{
    EMBOLDEN_EM, Error, GlyphImage, GlyphKey, RasterSource, TextBrush, TextLayout, TextSystem,
    oblique_transform,
};
use oxiview_base::{
    Affine, BezierPath, Color, Glyph, GlyphFont, GlyphKind, PathCommand, Point, Rect,
};
use parley::{GlyphRun, Line, PositionedLayoutItem, fontique::Synthesis};
use skrifa::{
    GlyphId, MetadataProvider,
    raw::{
        FontData, ReadError, TableProvider,
        tables::{bitmap::BitmapSize, sbix::Sbix},
    },
};
use std::{ops::Range, sync::Arc};
use swash::{
    FontRef,
    zeno::{Command, PathData},
};

const MAX_RASTER_SIZE: f64 = 2048.0;

/// Owned drawing for one shaped glyph, in local logical coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum GlyphDraw {
    /// Rasterized bitmap or hinted outline.
    Image(GlyphImage),
    /// Unhinted outline or OpenType COLR paint graph.
    Vector(Glyph),
    /// Unhinted Bézier outline with synthetic style applied at its local baseline.
    /// Paint with the nonzero fill rule.
    Path(BezierPath, Color),
}

/// A horizontal rule attached to a styled glyph run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDecorationKind {
    /// Paint below the glyphs in the draw order.
    Underline,
    /// Paint above the glyphs in the draw order.
    Strikethrough,
}

/// An owned decoration segment in paragraph-local logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextDecoration {
    /// The kind of decoration and its intended draw order.
    pub kind: TextDecorationKind,
    /// The rule's rectangle, relative to the same origin as the glyphs.
    pub rect: Rect,
    /// The resolved foreground of the decorated run.
    pub color: Color,
}

impl TextLayout {
    /// Returns decoration segments in visual run order, using each run's font
    /// metrics and baseline. Apply the same transform used to paint the glyphs.
    pub fn decorations(&self) -> impl Iterator<Item = TextDecoration> + '_ {
        let decorated = self
            .layout
            .styles()
            .iter()
            .any(|style| style.underline.is_some() || style.strikethrough.is_some());
        decorated
            .then(|| self.layout.lines())
            .into_iter()
            .flatten()
            .flat_map(|line| {
                let metrics = line.metrics();
                let start = f64::from(metrics.inline_min_coord) + f64::from(metrics.offset);
                let end = start + f64::from(metrics.advance);
                let hanging = f64::from(metrics.hanging_advance);
                // A line's hanging whitespace is on its trailing visual edge.
                let (start, end) = if self.layout.is_rtl() {
                    (start + hanging, end)
                } else {
                    (start, end - hanging)
                };
                line.items()
                    .filter_map(|item| match item {
                        PositionedLayoutItem::GlyphRun(run) => Some(run),
                        PositionedLayoutItem::InlineBox(_) => None,
                    })
                    .flat_map(move |run| run_decorations(run, start, end))
            })
    }
}

fn run_decorations(
    run: GlyphRun<'_, TextBrush>,
    line_start: f64,
    line_end: f64,
) -> impl Iterator<Item = TextDecoration> {
    let style = run.style();
    let metrics = run.run().font_metrics();
    let x = f64::from(run.offset());
    let left = x.max(line_start);
    let right = (x + f64::from(run.advance())).min(line_end);
    let baseline = f64::from(run.baseline());
    [
        (
            TextDecorationKind::Underline,
            &style.underline,
            metrics.underline_offset,
            metrics.underline_size,
        ),
        (
            TextDecorationKind::Strikethrough,
            &style.strikethrough,
            metrics.strikethrough_offset,
            metrics.strikethrough_size,
        ),
    ]
    .into_iter()
    .filter_map(move |(kind, decoration, offset, thickness)| {
        let decoration = decoration.as_ref()?;
        // OpenType gives the top of each rule in y-up coordinates.
        let top = baseline - f64::from(decoration.offset.unwrap_or(offset));
        let bottom = top + f64::from(decoration.size.unwrap_or(thickness));
        (left < right && top < bottom && [left, top, right, bottom].into_iter().all(f64::is_finite))
            .then_some(TextDecoration {
                kind,
                rect: Rect::new(left, top, right, bottom),
                color: decoration.brush.0,
            })
    })
}

#[derive(Clone, Copy)]
struct Placement {
    scale: f64,
    translation: Point,
    complex: bool,
    hybrid: bool,
}

impl TextSystem {
    /// Configures the largest final device size, in pixels per em, that uses hinted outlines.
    ///
    /// The default is 64. Zero selects vectors even for small outlines. Complex transforms
    /// always select vectors for outlines. Color glyph selection is independent of this setting.
    pub fn with_vector_threshold(mut self, pixels_per_em: f64) -> Result<Self, Error> {
        if !(0.0..=MAX_RASTER_SIZE).contains(&pixels_per_em) {
            return Err(Error::InvalidInput);
        }
        self.vector_threshold = pixels_per_em;
        Ok(self)
    }

    /// Produces owned glyph draws at their local logical positions.
    ///
    /// The caller supplies the complete local-to-device transform, including DPI, and
    /// applies it once when painting the result. Its largest stretch determines the
    /// display size. Positive uniform scale and translation preserve raster pixel phase.
    /// Embedded bitmaps use their strike resolution. Empty glyphs in bitmap-only
    /// faces keep their shaped advance without emitting a draw.
    pub fn glyph_draws(
        &mut self,
        layout: &TextLayout,
        origin: Point,
        local_to_device: Affine,
    ) -> Result<Vec<GlyphDraw>, Error> {
        self.glyph_draws_for(layout, origin, local_to_device, layout.layout.lines())
    }

    pub(super) fn glyph_draws_in_lines(
        &mut self,
        layout: &TextLayout,
        origin: Point,
        local_to_device: Affine,
        lines: Range<usize>,
    ) -> Result<Vec<GlyphDraw>, Error> {
        let selected = layout.layout.lines().skip(lines.start).take(lines.len());
        self.glyph_draws_for(layout, origin, local_to_device, selected)
    }

    fn glyph_draws_for<'a>(
        &mut self,
        layout: &TextLayout,
        origin: Point,
        local_to_device: Affine,
        lines: impl Iterator<Item = Line<'a, TextBrush>>,
    ) -> Result<Vec<GlyphDraw>, Error> {
        let [a, b, c, d, x, y] = local_to_device.coefficients();
        if ![a, b, c, d, x, y].into_iter().all(f64::is_finite) {
            return Err(Error::InvalidInput);
        }
        // The largest singular value is the maximum stretch of a unit vector.
        let scale = ((a + d).hypot(b - c) + (a - d).hypot(b + c)) * 0.5;
        let complex = b != 0.0 || c != 0.0 || a != d || a <= 0.0;
        self.collect_draws(
            layout.glyphs,
            origin,
            Placement {
                scale,
                translation: if complex {
                    Point::default()
                } else {
                    Point::new(x, y)
                },
                complex,
                hybrid: true,
            },
            lines,
        )
    }

    /// Produces owned glyph images using Swash's raster-only sources.
    ///
    /// Glyphs use grayscale antialiasing with quarter-pixel phase caching, and
    /// embedded color bitmap strikes and simple COLRv0 layers are supported.
    /// Use [Self::glyph_draws] for the full COLR paint graph and vector outlines.
    pub fn glyph_images(
        &mut self,
        layout: &TextLayout,
        origin: Point,
        scale: f64,
    ) -> Result<Vec<GlyphImage>, Error> {
        if scale > 16.0 {
            return Err(Error::InvalidInput);
        }
        let draws = self.collect_draws(
            layout.glyphs,
            origin,
            Placement {
                scale,
                translation: Point::default(),
                complex: false,
                hybrid: false,
            },
            layout.layout.lines(),
        )?;
        Ok(draws
            .into_iter()
            .filter_map(|draw| match draw {
                GlyphDraw::Image(image) => Some(image),
                GlyphDraw::Vector(_) | GlyphDraw::Path(..) => None,
            })
            .collect())
    }

    fn collect_draws<'a>(
        &mut self,
        glyphs: usize,
        origin: Point,
        placement: Placement,
        lines: impl Iterator<Item = Line<'a, TextBrush>>,
    ) -> Result<Vec<GlyphDraw>, Error> {
        let Placement {
            scale,
            translation,
            complex,
            hybrid,
        } = placement;
        if ![
            origin.x(),
            origin.y(),
            scale,
            translation.x(),
            translation.y(),
        ]
        .into_iter()
        .all(f64::is_finite)
            || scale <= 0.0
        {
            return Err(Error::InvalidInput);
        }
        let _span = tracing::info_span!("text_draw", glyphs, scale, hybrid).entered();
        let mut output = Vec::new();
        for line in lines {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                let font = run.font();
                let synthesis = run.synthesis();
                let size = run.font_size();
                let display_size = f64::from(size) * scale;
                if !hybrid && display_size > MAX_RASTER_SIZE {
                    return Err(Error::RasterLimit);
                }
                let color = glyph_run.style().brush.0;
                let srgb = color.to_srgb();
                let rgba = srgb.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
                let variations: Arc<[i16]> = run
                    .normalized_coords()
                    .iter()
                    .map(|coord| coord.to_bits())
                    .collect();
                let mut vector_font = None;
                let face = hybrid
                    .then(|| skrifa::FontRef::from_index(font.data.as_ref(), font.index))
                    .transpose()
                    .map_err(|_| Error::InvalidFont)?;
                let color_font = face
                    .as_ref()
                    .and_then(|face| face.cpal().ok().map(|_| face.color_glyphs()));
                let vector_outlines = hybrid && (complex || display_size > self.vector_threshold);
                let bitmaps = face.as_ref().map(BitmapTables::new);
                let outline_free = face.as_ref().filter(|face| !has_outline_tables(face));
                let mut advance = f64::from(glyph_run.offset());
                for glyph in glyph_run.glyphs() {
                    let position = Point::new(
                        origin.x() + advance + f64::from(glyph.x),
                        origin.y() + f64::from(glyph_run.baseline()) + f64::from(glyph.y),
                    );
                    if !position.x().is_finite() || !position.y().is_finite() {
                        return Err(Error::InvalidInput);
                    }
                    advance += f64::from(glyph.advance);
                    let glyph_id = u16::try_from(glyph.id).map_err(|_| Error::InvalidFont)?;
                    let color_glyph = color_font
                        .as_ref()
                        .is_some_and(|color_font| color_font.get(GlyphId::new(glyph.id)).is_some());
                    let bitmap = match &bitmaps {
                        Some(bitmaps) if !color_glyph => bitmaps.strike(
                            GlyphId::new(glyph.id),
                            display_size.ceil() as u16,
                            outline_free.is_some(),
                        )?,
                        _ => None,
                    };
                    if !color_glyph
                        && bitmap.is_none()
                        && let (Some(face), Some(bitmaps)) = (outline_free, &bitmaps)
                    {
                        if !bitmaps.has_strikes()
                            || face.head().map_err(|_| Error::InvalidFont)?.units_per_em() == 0
                            || glyph_id >= face.maxp().map_err(|_| Error::InvalidFont)?.num_glyphs()
                        {
                            return Err(Error::InvalidFont);
                        }
                        // A valid glyph in a bitmap-only face with no strike
                        // has no ink. Its shaped advance was applied above.
                        continue;
                    }
                    if color_glyph || (vector_outlines && bitmap.is_none()) {
                        if srgb[3] > 0.0 {
                            if !color_glyph && (synthesis.embolden() || synthesis.skew().is_some())
                            {
                                if let Some(path) = self.synthesized_outline(
                                    font,
                                    glyph_id,
                                    &variations,
                                    synthesis,
                                    size,
                                    position,
                                )? {
                                    output.push(GlyphDraw::Path(path, color));
                                }
                            } else {
                                let font_bytes = vector_font.get_or_insert_with(|| {
                                    GlyphFont::from_shared(font.data.clone().into_raw_parts().0)
                                });
                                output.push(GlyphDraw::Vector(Glyph {
                                    font: font_bytes.clone(),
                                    face_index: font.index,
                                    glyph_id,
                                    size,
                                    origin: position,
                                    color,
                                    variations: variations.clone(),
                                    kind: if color_glyph {
                                        GlyphKind::Color
                                    } else {
                                        GlyphKind::Outline
                                    },
                                }));
                            }
                        }
                        continue;
                    }
                    // Keep the bitmap at its embedded resolution, even if the glyph will
                    // cover less than one device pixel. The draw's affine scales the image.
                    let (raster_size, source) = if let Some((ppem, source)) = bitmap {
                        (f64::from(ppem), source)
                    } else {
                        (display_size, RasterSource::Auto)
                    };
                    let (sample_scale, x, y) = if bitmap.is_some() {
                        // Bitmap strikes have their own sampling grid. Keep the shaped
                        // baseline exact so a fractional strike pixel cannot trip a
                        // large placement shift when the image is magnified.
                        (raster_size / f64::from(size), 0.0, 0.0)
                    } else {
                        (
                            scale,
                            position.x() * scale + translation.x(),
                            position.y() * scale + translation.y(),
                        )
                    };
                    if !x.is_finite() || !y.is_finite() {
                        return Err(Error::InvalidInput);
                    }
                    let key = GlyphKey {
                        font: font.data.id(),
                        index: font.index,
                        glyph: glyph_id,
                        size: (raster_size as f32).to_bits(),
                        coords: variations.clone(),
                        skew: synthesis.skew().map(f32::to_bits),
                        embolden: synthesis.embolden(),
                        x: ((x - x.floor()) * 4.0).round() as u8,
                        y: ((y - y.floor()) * 4.0).round() as u8,
                        color: rgba,
                        source,
                    };
                    let cached = self.raster_glyph(key, font)?;
                    if let Some(image) = cached.image {
                        let (baseline_x, baseline_y) = if bitmap.is_some() {
                            (position.x(), position.y())
                        } else {
                            (
                                (x.floor() - translation.x()) / sample_scale,
                                (y.floor() - translation.y()) / sample_scale,
                            )
                        };
                        let left = baseline_x + f64::from(cached.left) / sample_scale;
                        let top = baseline_y - f64::from(cached.top) / sample_scale;
                        output.push(GlyphDraw::Image(GlyphImage {
                            rect: Rect::new(
                                left,
                                top,
                                left + f64::from(image.width()) / sample_scale,
                                top + f64::from(image.height()) / sample_scale,
                            ),
                            image,
                        }));
                    }
                }
            }
        }
        Ok(output)
    }

    fn synthesized_outline(
        &mut self,
        font: &parley::FontData,
        glyph: u16,
        variations: &[i16],
        synthesis: Synthesis,
        size: f32,
        baseline: Point,
    ) -> Result<Option<BezierPath>, Error> {
        let font_ref = FontRef::from_index(font.data.as_ref(), font.index as usize)
            .ok_or(Error::InvalidFont)?;
        let units_per_em = font_ref.metrics(variations).units_per_em;
        if units_per_em == 0 {
            return Err(Error::InvalidFont);
        }
        // Work in design units so magnification does not expose rounding at the
        // layout size. Match Swash's raster order: embolden first, then skew.
        let mut outline = self
            .scales
            .builder_with_id(font_ref, [font.data.id(), u64::from(font.index)])
            .normalized_coords(variations.iter().copied())
            .build()
            .scale_outline(glyph)
            .ok_or(Error::InvalidGlyphOutline)?;
        if outline.verbs().is_empty() {
            return Ok(None);
        }
        if synthesis.embolden() {
            let strength = f32::from(units_per_em) * EMBOLDEN_EM;
            outline.embolden(strength, strength);
        }
        if let Some(skew) = synthesis.skew() {
            outline.transform(&oblique_transform(skew));
        }
        let scale = f64::from(size) / f64::from(units_per_em);
        let local = |point: swash::zeno::Point| {
            Point::new(
                baseline.x() + f64::from(point.x) * scale,
                baseline.y() - f64::from(point.y) * scale,
            )
        };
        let commands = outline.path().commands().map(|command| match command {
            Command::MoveTo(point) => PathCommand::MoveTo(local(point)),
            Command::LineTo(end) => PathCommand::LineTo(local(end)),
            Command::QuadTo(control, end) => PathCommand::QuadTo(local(control), local(end)),
            Command::CurveTo(first, second, end) => {
                PathCommand::CubicTo(local(first), local(second), local(end))
            }
            Command::Close => PathCommand::Close,
        });
        BezierPath::new(commands.collect())
            .map(Some)
            .map_err(|_| Error::InvalidGlyphOutline)
    }
}

fn has_outline_tables(face: &skrifa::FontRef<'_>) -> bool {
    // A declared but unreadable outline table must still take the outline error path.
    face.table_directory().table_records().iter().any(|table| {
        matches!(
            &table.tag().to_be_bytes(),
            b"glyf" | b"loca" | b"CFF " | b"CFF2" | b"VARC"
        )
    })
}

struct BitmapTables<'a> {
    sbix: Option<Sbix<'a>>,
    color: Option<(FontData<'a>, &'a [BitmapSize])>,
    alpha: Option<(FontData<'a>, &'a [BitmapSize])>,
}

impl<'a> BitmapTables<'a> {
    fn new(face: &skrifa::FontRef<'a>) -> Self {
        let sbix = face.sbix().ok();
        // Match Swash's source order so a strike index identifies the same table
        // at lookup and rasterization.
        let color = face.cblc().ok().and_then(|table| {
            (sbix.is_none() && face.cbdt().is_ok())
                .then(|| (table.offset_data(), table.bitmap_sizes()))
        });
        let alpha = face.eblc().ok().and_then(|table| {
            face.ebdt()
                .is_ok()
                .then(|| (table.offset_data(), table.bitmap_sizes()))
        });
        Self { sbix, color, alpha }
    }

    fn has_strikes(&self) -> bool {
        self.sbix.as_ref().is_some_and(|table| {
            table
                .strikes()
                .iter()
                .flatten()
                .any(|strike| strike.ppem() > 0)
        }) || [self.color, self.alpha]
            .into_iter()
            .flatten()
            .any(|(_, sizes)| sizes.iter().any(|size| size.ppem_y() > 0))
    }

    fn strike(
        &self,
        glyph: GlyphId,
        target: u16,
        bitmap_only: bool,
    ) -> Result<Option<(u16, RasterSource)>, Error> {
        let color = if let Some(sbix) = &self.sbix {
            best_strike(
                sbix.strikes().iter().map(|strike| {
                    let strike = strike?;
                    let image = strike.glyph_data(glyph)?.filter(|image| {
                        // Faces with outlines can fall back when Swash does not
                        // decode the embedded image format.
                        bitmap_only
                            || matches!(&image.graphic_type().to_be_bytes(), b"png " | b"dupe")
                    });
                    Ok(image.map(|_| strike.ppem()))
                }),
                target,
            )?
        } else {
            bdt_strike(self.color, glyph, target)?
        };
        if let Some((ppem, index)) = color {
            return Ok(Some((ppem, RasterSource::ColorBitmap(index))));
        }
        Ok(bdt_strike(self.alpha, glyph, target)?
            .map(|(ppem, index)| (ppem, RasterSource::AlphaBitmap(index))))
    }
}

fn bdt_strike(
    table: Option<(FontData<'_>, &[BitmapSize])>,
    glyph: GlyphId,
    target: u16,
) -> Result<Option<(u16, u32)>, Error> {
    let Some((data, sizes)) = table else {
        return Ok(None);
    };
    best_strike(
        sizes.iter().map(|size| {
            if !(size.start_glyph_index()..=size.end_glyph_index()).contains(&glyph) {
                return Ok(None);
            }
            // A strike can have holes even within its advertised glyph range.
            let subtables = size.index_subtable_list(data)?;
            if !subtables.index_subtable_records().iter().any(|record| {
                (record.first_glyph_index()..=record.last_glyph_index()).contains(&glyph)
            }) {
                return Ok(None);
            }
            match size.location(data, glyph) {
                Ok(location) => Ok((!location.is_empty()).then(|| u16::from(size.ppem_y()))),
                Err(ReadError::InvalidCollectionIndex(_)) => Ok(None),
                Err(error) => Err(error),
            }
        }),
        target,
    )
}

fn best_strike(
    strikes: impl Iterator<Item = Result<Option<u16>, ReadError>>,
    target: u16,
) -> Result<Option<(u16, u32)>, Error> {
    let rank = |ppem: u16| (ppem < target, ppem.abs_diff(target));
    let mut best = None;
    for (index, strike) in strikes.enumerate() {
        if let Some(ppem) = strike.map_err(|_| Error::InvalidFont)? {
            if ppem == 0 {
                return Err(Error::InvalidGlyphImage);
            }
            if best.is_none_or(|(current, _)| rank(ppem) < rank(current)) {
                best = Some((ppem, index as u32));
            }
        }
    }
    Ok(best)
}
