use crate::{Error, solid};
use oxiview_base::{Glyph, GlyphKind};
use skrifa::{
    FontRef, GlyphId, MetadataProvider,
    instance::{LocationRef, NormalizedCoord, Size},
    outline::{DrawSettings, OutlinePen},
    raw::TableProvider,
};
use vello_cpu::{LinearImages, RenderContext, kurbo};

mod colr;

#[derive(Default)]
pub(super) struct GlyphPainter {
    color: colr::ColorRasterizer,
}

impl GlyphPainter {
    pub(super) fn paint(
        &mut self,
        context: &mut RenderContext,
        images: &mut LinearImages<'_>,
        glyph: &Glyph,
    ) -> Result<(), Error> {
        let transform = *context.transform();
        if transform.determinant() == 0.0 {
            return Ok(());
        }
        let placement = transform * kurbo::Affine::translate((glyph.origin.x(), glyph.origin.y()));
        if !placement.as_coeffs().into_iter().all(f64::is_finite) {
            return Err(Error::Glyph("non-finite glyph placement".into()));
        }

        let face = FontRef::from_index(glyph.font.bytes(), glyph.face_index)
            .map_err(|error| Error::Glyph(error.to_string()))?;
        let upem = face
            .head()
            .map_err(|error| Error::Glyph(error.to_string()))?
            .units_per_em();
        if upem == 0 {
            return Err(Error::Glyph("font has zero units per em".into()));
        }
        let coordinates: Vec<_> = glyph
            .variations
            .iter()
            .copied()
            .map(NormalizedCoord::from_bits)
            .collect();
        let location = LocationRef::new(&coordinates);
        let id = GlyphId::from(glyph.glyph_id);
        let scale = f64::from(glyph.size) / f64::from(upem);
        let placement = placement * kurbo::Affine::scale_non_uniform(scale, -scale);
        if glyph.kind == GlyphKind::Color {
            return self
                .color
                .paint(context, images, &face, glyph, location, placement);
        }

        let outline = outline(&face, id, location)?;
        context.set_transform(placement);
        context.set_paint(solid(glyph.color));
        context.fill_path(&outline);
        context.set_transform(transform);
        Ok(())
    }
}

fn outline(
    face: &FontRef<'_>,
    id: GlyphId,
    location: LocationRef<'_>,
) -> Result<kurbo::BezPath, Error> {
    let outline = face
        .outline_glyphs()
        .get(id)
        .ok_or_else(|| Error::Glyph("outline glyph is missing".into()))?;
    let mut pen = PathPen::default();
    outline
        .draw(DrawSettings::unhinted(Size::unscaled(), location), &mut pen)
        .map_err(|error| Error::Glyph(error.to_string()))?;
    Ok(pen.0)
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
