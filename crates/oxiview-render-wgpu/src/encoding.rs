use crate::{Error, PixelSize};
use oxiview_base::{Affine, Color, CornerRadii, Point, Rect};
use oxiview_scene::{Brush, Clip, EncodedNode, EncodedScene, Primitive};

mod tiles;
use tiles::Bounds;

// Word-addressed private protocol, mirrored in raster.wgsl. Header: width,
// height, stack stride, tile-table offset. Each operation starts with opcode and next.
// Draw: outer shape, inner shape, clip count, brush, then clip offsets.
// SDF shape: MAX tag, inverse affine, bounds, TL/TR/BR/BL radii, scale.
// Polygon: vertex count then xy pairs. Brush: inverse affine, gradient endpoints,
// stop count, then (position, premultiplied linear RGBA) stops.
pub(crate) struct Commands {
    words: Vec<u32>,
    pub(crate) depth: u32,
}
impl Commands {
    pub(crate) fn new(scene: &EncodedScene, size: PixelSize) -> Result<Self, Error> {
        let mut result = Self {
            words: vec![size.width, size.height, 1, 0],
            depth: 1,
        };
        let mut opacities = Vec::new();
        let mut operations: Vec<(u32, Bounds)> = Vec::new();
        for node in &scene.nodes {
            let start = result.words.len();
            let mut bounds = Bounds::EMPTY;
            match node {
                EncodedNode::BeginOpacity(opacity) => {
                    opacities.push((*opacity, operations.len(), Bounds::EMPTY));
                    result.depth = result.depth.max(count(opacities.len())?);
                    result.words.extend([1, 0]);
                }
                EncodedNode::EndOpacity => {
                    let (opacity, begin, group_bounds) = opacities
                        .pop()
                        .ok_or_else(|| invalid("unbalanced opacity"))?;
                    bounds = group_bounds;
                    operations
                        .get_mut(begin)
                        .ok_or_else(|| invalid("invalid opacity group"))?
                        .1 = bounds;
                    result.words.extend([2, 0]);
                    result.float(opacity)?;
                }
                EncodedNode::Draw {
                    primitive,
                    transform,
                    clips,
                } => {
                    // Singular transforms cover no area. State is resolved per draw.
                    let Ok(inverse) = transform.inverse() else {
                        continue;
                    };
                    let (rect, radii, width, brush) = match primitive {
                        Primitive::Fill(rect, radii, brush) => (*rect, *radii, None, brush),
                        Primitive::Stroke(rect, radii, width, brush) => {
                            (*rect, *radii, Some(*width), brush)
                        }
                    };
                    if width == Some(0.0) {
                        continue;
                    }
                    result.words.extend([0, 0, 0, 0, count(clips.len())?, 0]);
                    let clip_start = result.words.len();
                    result.words.resize(clip_start + clips.len(), 0);
                    let outer = result.shape(
                        rect,
                        radii,
                        width.map_or(0.0, |width| width / 2.0),
                        *transform,
                    )?;
                    bounds =
                        Bounds::shape(rect, width.map_or(0.0, |width| width / 2.0), *transform);
                    result.set(start + 2, outer)?;
                    let inner = if let Some(width) = width {
                        result.shape(rect, radii, -width / 2.0, *transform)?
                    } else {
                        result.empty_polygon()?
                    };
                    result.set(start + 3, inner)?;
                    for (index, clip) in clips.iter().enumerate() {
                        let (rect, radii) = match clip.shape {
                            Clip::Rect(rect) => (rect, CornerRadii::default()),
                            Clip::RoundedRect(rect, radii) => (rect, radii),
                        };
                        let offset = result.shape(rect, radii, 0.0, clip.transform)?;
                        bounds = bounds.intersect(Bounds::shape(rect, 0.0, clip.transform));
                        result.set(clip_start + index, offset)?;
                    }
                    result.set(start + 5, count(result.words.len())?)?;
                    for value in inverse.coefficients() {
                        result.float(value)?;
                    }
                    match brush {
                        Brush::Solid(color) => {
                            for value in [0.0, 0.0, 1.0, 0.0] {
                                result.float(value)?;
                            }
                            result.words.push(2);
                            result.stop(0.0, *color)?;
                            result.stop(1.0, *color)?;
                        }
                        Brush::LinearGradient(a, b, stops) => {
                            for value in [a.x(), a.y(), b.x(), b.y()] {
                                result.float(value)?;
                            }
                            result.words.push(count(stops.len())?);
                            for stop in stops {
                                result.stop(stop.position, stop.color)?;
                            }
                        }
                    }
                }
            }
            result.set(start + 1, count(result.words.len())?)?;
            if let Some((_, _, group_bounds)) = opacities.last_mut() {
                *group_bounds = group_bounds.union(bounds);
            }
            operations.push((count(start)?, bounds));
        }
        if !opacities.is_empty() {
            return Err(invalid("unbalanced opacity"));
        }
        result.set(2, result.depth)?;
        result.set(3, count(result.words.len())?)?;
        tiles::append(&mut result.words, &operations, size)?;
        Ok(result)
    }
    pub(crate) fn bytes(&self) -> Vec<u8> {
        self.words
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect()
    }
    // Differential-test oracle: the same GPU interpreter visits the original
    // complete ordered stream in every tile, without any spatial culling.
    #[cfg(test)]
    pub(crate) fn unbinned(scene: &EncodedScene, size: PixelSize) -> Result<Self, Error> {
        let mut commands = Self::new(scene, size)?;
        let end = *commands
            .words
            .get(3)
            .ok_or_else(|| invalid("missing header"))?;
        commands.words.truncate(end as usize);
        let bounds = Bounds::shape(
            Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
            0.0,
            Affine::IDENTITY,
        );
        let mut operations = Vec::new();
        let mut cursor = 4;
        while cursor < end {
            operations.push((cursor, bounds));
            cursor = *commands
                .words
                .get(cursor as usize + 1)
                .ok_or_else(|| invalid("invalid command"))?;
        }
        tiles::append(&mut commands.words, &operations, size)?;
        Ok(commands)
    }
    fn float(&mut self, value: f64) -> Result<(), Error> {
        let value = value as f32;
        if !value.is_finite() {
            return Err(invalid("non-finite or out-of-range GPU coordinate/color"));
        }
        self.words.push(value.to_bits());
        Ok(())
    }
    fn stop(&mut self, position: f64, color: Color) -> Result<(), Error> {
        self.float(position)?;
        let [r, g, b, a] = color.to_linear();
        for value in [r * a, g * a, b * a, a] {
            self.float(f64::from(value))?;
        }
        Ok(())
    }
    fn empty_polygon(&mut self) -> Result<u32, Error> {
        let offset = count(self.words.len())?;
        self.words.push(0);
        Ok(offset)
    }
    fn set(&mut self, index: usize, value: u32) -> Result<(), Error> {
        *self
            .words
            .get_mut(index)
            .ok_or_else(|| invalid("invalid command offset"))? = value;
        Ok(())
    }

    // A tagged shape record lets draws and clips independently choose the SDF
    // fast path. Polygon records retain their vertex count as the first word.
    fn shape(
        &mut self,
        rect: Rect,
        radii: CornerRadii,
        outset: f64,
        transform: Affine,
    ) -> Result<u32, Error> {
        let Some(scale) = sdf_scale(transform) else {
            return self.polygon(rect, radii, outset, transform);
        };
        let [a, b, c, d] = rect.edges();
        if !rect
            .edges()
            .iter()
            .chain(radii.values().iter())
            .all(|x| x.is_finite())
        {
            return Err(invalid("non-finite geometry"));
        }
        let (l, t, r, b) = (a.min(c), b.min(d), a.max(c), b.max(d));
        let limit = (r - l).min(b - t) * 0.5;
        let radii = radii.values().map(|r| {
            let r = r.abs().min(limit);
            if r == 0.0 { 0.0 } else { (r + outset).max(0.0) }
        });
        let (l, t, r, b) = (l - outset, t - outset, r + outset, b + outset);
        if l >= r || t >= b {
            return self.empty_polygon();
        }
        let inverse = transform
            .inverse()
            .map_err(|_| invalid("singular SDF transform"))?;
        let offset = count(self.words.len())?;
        self.words.push(u32::MAX);
        for value in inverse
            .coefficients()
            .into_iter()
            .chain([l, t, r, b])
            .chain(radii)
            .chain([scale])
        {
            self.float(value)?;
        }
        Ok(offset)
    }
    fn polygon(
        &mut self,
        rect: Rect,
        radii: CornerRadii,
        outset: f64,
        transform: Affine,
    ) -> Result<u32, Error> {
        let edges = rect.edges();
        if !edges
            .iter()
            .chain(radii.values().iter())
            .chain(transform.coefficients().iter())
            .all(|v| v.is_finite())
        {
            return Err(invalid("non-finite geometry"));
        }
        let [a, b, c, d] = edges;
        let (left, top, right, bottom) = (a.min(c), b.min(d), a.max(c), b.max(d));
        let limit = (right - left).min(bottom - top) / 2.0;
        let [tl, tr, br, bl] = radii.values().map(|r| {
            let r = r.abs().min(limit);
            // Sharp corners retain the CPU stroke's default miter join.
            if r == 0.0 { 0.0 } else { (r + outset).max(0.0) }
        });
        let (left, top, right, bottom) =
            (left - outset, top - outset, right + outset, bottom + outset);
        if left >= right || top >= bottom {
            return self.empty_polygon();
        }
        let offset = self.empty_polygon()?;
        let [a, b, c, d, _, _] = transform.coefficients();
        let scale = a.hypot(b) + c.hypot(d);
        let corners = [
            (left + tl, top + tl, tl, std::f64::consts::PI),
            (right - tr, top + tr, tr, 1.5 * std::f64::consts::PI),
            (right - br, bottom - br, br, 0.0),
            (left + bl, bottom - bl, bl, std::f64::consts::FRAC_PI_2),
        ];
        let mut vertices = 0;
        for (x, y, radius, angle) in corners {
            // Chord error <= 0.1 physical pixel under the affine norm bound.
            let segments = ((radius * scale / 0.2).sqrt() * std::f64::consts::FRAC_PI_2)
                .ceil()
                .max(1.0);
            if !segments.is_finite() || segments > 65536.0 {
                return Err(invalid("curve exceeds flattening capacity"));
            }
            let segments = segments as u32;
            for step in 0..=segments {
                let angle =
                    angle + f64::from(step) / f64::from(segments) * std::f64::consts::FRAC_PI_2;
                let point = transform.transform_point(Point::new(
                    x + radius * angle.cos(),
                    y + radius * angle.sin(),
                ));
                self.float(point.x())?;
                self.float(point.y())?;
                vertices += 1;
            }
        }
        self.set(offset as usize, vertices)?;
        Ok(offset)
    }
}
fn count(value: usize) -> Result<u32, Error> {
    u32::try_from(value).map_err(|_| invalid("encoded scene exceeds GPU address space"))
}
fn invalid(message: &str) -> Error {
    Error::SceneEncoding(message.into())
}

// Similarities only: finite nonzero columns, equal squared lengths and zero dot
// product within 1e-12 relative error. Translation, rotation, uniform scale and
// reflection preserve circular corners and distance. Shear/nonuniform scale fall
// back to polygons. The tolerance admits floating-point rotation composition.
pub(crate) fn sdf_scale(transform: Affine) -> Option<f64> {
    let [a, b, c, d, e, f] = transform.coefficients();
    if ![a, b, c, d, e, f].iter().all(|x| x.is_finite()) {
        return None;
    }
    let x = a * a + b * b;
    let y = c * c + d * d;
    let tolerance = x.max(y) * 1e-12;
    if x > 0.0
        && x.is_finite()
        && y.is_finite()
        && (x - y).abs() <= tolerance
        && (a * c + b * d).abs() <= tolerance
    {
        Some(x.sqrt())
    } else {
        None
    }
}
