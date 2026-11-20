use crate::{Error, PixelSize};
use oxiview_base::{Affine, Color, CornerRadii, Point, Rect};
use oxiview_scene::{Brush, Clip, EncodedNode, EncodedScene, Primitive};

mod tiles;
use tiles::Bounds;

// Word-addressed private protocol, mirrored in raster.wgsl. Header: width,
// height, stack stride, tile-table offset. Each operation starts with opcode and next.
// Draw: outer shape, inner shape, clip count, brush, then clip offsets.
// Bordered fill (opcode 3): outer, inner, clip count, background brush, border
// brush, then clip offsets. Shared coverage keeps the fill/stroke edge correlated.
// Analytic shape: MAX (SDF) or MAX-1 (sampled) tag, inverse affine, bounds,
// TL/TR/BR/BL radii, SDF scale (unused for sampled shapes).
// Polygon: vertex count then xy pairs. A solid brush starts with MAX (which
// cannot encode a finite affine coefficient), then four premultiplied linear
// channels. Gradients start with inverse affine, endpoints, and stop count,
// followed by (position, premultiplied linear RGBA) stops.
pub(crate) struct Commands {
    words: Vec<u32>,
    pub(crate) depth: u32,
}
impl Commands {
    pub(crate) fn new(scene: &EncodedScene, size: PixelSize) -> Result<Self, Error> {
        let mut result = Self {
            words: vec![size.width, size.height, 0, 0],
            depth: 0,
        };
        let mut opacities = Vec::new();
        let mut operations: Vec<(u32, Bounds)> = Vec::new();
        let mut nodes = scene.nodes.iter().peekable();
        while let Some(node) = nodes.next() {
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
                    let boxed = bordered_fill(node, nodes.peek().copied());
                    if boxed.is_some() {
                        nodes.next();
                    }
                    let (mut rect, mut radii, mut width, brush) = match primitive {
                        Primitive::Fill(rect, radii, brush) => (*rect, *radii, None, brush),
                        Primitive::Stroke(rect, radii, width, brush) => {
                            (*rect, *radii, Some(*width), brush)
                        }
                        Primitive::Image(..) | Primitive::Glyph(_) | Primitive::FillPath(..) => {
                            return Err(invalid("image, glyph, or path requires Vello"));
                        }
                    };
                    if let Some(boxed) = &boxed {
                        rect = boxed.rect;
                        radii = boxed.radii;
                        width = Some(boxed.width);
                    }
                    if width == Some(0.0) {
                        continue;
                    }
                    if boxed.is_some() {
                        result.words.extend([3, 0, 0, 0, count(clips.len())?, 0, 0]);
                    } else {
                        result.words.extend([0, 0, 0, 0, count(clips.len())?, 0]);
                    }
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
                    match brush {
                        Brush::Solid(color) => {
                            if inverse
                                .coefficients()
                                .into_iter()
                                .any(|v| !(v as f32).is_finite())
                            {
                                return Err(invalid("non-finite or out-of-range GPU transform"));
                            }
                            result.solid_brush(*color)?;
                        }
                        Brush::LinearGradient(a, b, stops) => {
                            result.brush_transform(inverse)?;
                            for value in [a.x(), a.y(), b.x(), b.y()] {
                                result.float(value)?;
                            }
                            result.words.push(count(stops.len())?);
                            for stop in stops {
                                result.stop(stop.position, stop.color)?;
                            }
                        }
                    }
                    if let Some(boxed) = boxed {
                        let offset = result.solid_brush(boxed.color)?;
                        result.set(start + 6, offset)?;
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
    pub(crate) fn opacity_stack_bytes(&self, size: PixelSize) -> Result<u64, Error> {
        // WGSL requires a binding large enough for one vec4 even when no
        // BeginOpacity instruction can address the stack.
        let bytes = u64::from(size.width)
            .checked_mul(u64::from(size.height))
            .and_then(|n| n.checked_mul(u64::from(self.depth)))
            .and_then(|n| n.checked_mul(16))
            .ok_or(Error::InvalidSize(size))?;
        Ok(bytes.max(16))
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
        if [r, g, b, a].into_iter().any(|channel| !channel.is_finite()) {
            return Err(invalid("non-finite or out-of-range GPU color"));
        }
        let [r, g, b, a] = [r, g, b, a].map(|channel| channel.clamp(0.0, 1.0));
        for value in [r * a, g * a, b * a, a] {
            self.float(f64::from(value))?;
        }
        Ok(())
    }
    fn brush_transform(&mut self, inverse: Affine) -> Result<(), Error> {
        for value in inverse.coefficients() {
            self.float(value)?;
        }
        Ok(())
    }
    fn solid_brush(&mut self, color: Color) -> Result<u32, Error> {
        let offset = count(self.words.len())?;
        let rgba = color.to_srgb();
        let [r, g, b, a] = color.to_linear();
        if rgba
            .into_iter()
            .chain([r * a, g * a, b * a, a])
            .any(|x| !x.is_finite())
        {
            return Err(invalid("non-finite or out-of-range GPU color"));
        }
        let [r, g, b, a] = [r, g, b, a].map(|v| v.clamp(0.0, 1.0));
        self.words.push(u32::MAX);
        self.words
            .extend([r * a, g * a, b * a, a].map(f32::to_bits));
        Ok(offset)
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

    fn shape(
        &mut self,
        rect: Rect,
        radii: CornerRadii,
        outset: f64,
        transform: Affine,
    ) -> Result<u32, Error> {
        let [a, b, c, d] = rect.edges();
        let radii = radii.values().map(f64::abs);
        if ![a, b, c, d]
            .into_iter()
            .chain(radii)
            .chain(transform.coefficients())
            .all(f64::is_finite)
        {
            return Err(invalid("non-finite geometry"));
        }
        let (l, t, r, b) = (a.min(c), b.min(d), a.max(c), b.max(d));
        let (width, height) = (r - l, b - t);
        // The UI passes proportionally fitted radii. For overlapping radii
        // supplied directly to Scene, mirror the CPU's legacy Kurbo clamp.
        let radii = if radii_fit(width, height, radii) {
            radii
        } else {
            radii.map(|radius| radius.min(width.min(height) * 0.5))
        };
        let radii = radii.map(|radius| {
            // Sharp corners retain the CPU stroke's default miter join.
            if radius == 0.0 {
                0.0
            } else {
                (radius + outset).max(0.0)
            }
        });
        let (l, t, r, b) = (l - outset, t - outset, r + outset, b + outset);
        if l >= r || t >= b {
            return self.empty_polygon();
        }
        let scale = sdf_scale(transform);
        // A stroke inset can trim a large arc against another edge. Sample
        // the intersection directly when the adjusted arcs cannot be joined
        // into a polygon in corner order.
        if scale.is_none() && radii_fit(r - l, b - t, radii) {
            return self.polygon([l, t, r, b], radii, transform);
        }
        let inverse = transform
            .inverse()
            .map_err(|_| invalid("singular analytic shape transform"))?;
        let offset = count(self.words.len())?;
        self.words.push(if scale.is_some() {
            u32::MAX
        } else {
            u32::MAX - 1
        });
        for value in inverse
            .coefficients()
            .into_iter()
            .chain([l, t, r, b])
            .chain(radii)
            .chain([scale.unwrap_or_default()])
        {
            self.float(value)?;
        }
        Ok(offset)
    }
    fn polygon(
        &mut self,
        [left, top, right, bottom]: [f64; 4],
        [tl, tr, br, bl]: [f64; 4],
        transform: Affine,
    ) -> Result<u32, Error> {
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

struct BorderedFill {
    rect: Rect,
    radii: CornerRadii,
    width: f64,
    color: Color,
}

// A filled box followed immediately by its inside border represents correlated
// coverage at the same outer edge. Averaging each draw independently and then
// applying source-over twice makes that edge too opaque. Fuse only an exact
// geometric match in the same opacity/clip scope; all other draws stay separate.
fn bordered_fill(first: &EncodedNode, next: Option<&EncodedNode>) -> Option<BorderedFill> {
    let EncodedNode::Draw {
        primitive: Primitive::Fill(outer, outer_radii, Brush::Solid(_)),
        transform,
        clips,
    } = first
    else {
        return None;
    };
    let EncodedNode::Draw {
        primitive: Primitive::Stroke(rect, radii, width, Brush::Solid(color)),
        transform: border_transform,
        clips: border_clips,
    } = next?
    else {
        return None;
    };
    if transform != border_transform
        || !width.is_finite()
        || *width <= 0.0
        || rect.size().width() <= 0.0
        || rect.size().height() <= 0.0
    {
        return None;
    }
    let inherited = border_clips == clips;
    let own_clip = border_clips.len() == clips.len() + 1
        && border_clips.get(..clips.len()) == Some(clips.as_slice())
        && border_clips.last().is_some_and(|clip| {
            clip.transform == *transform && clip.shape == Clip::RoundedRect(*outer, *outer_radii)
        });
    if !inherited && !own_clip {
        return None;
    }
    let half = *width / 2.0;
    let [left, top, right, bottom] = rect.edges();
    let close = |a: f64, b: f64| (a - b).abs() <= 1.0e-9;
    if !outer
        .edges()
        .into_iter()
        .zip([left - half, top - half, right + half, bottom + half])
        .all(|(a, b)| close(a, b))
    {
        return None;
    }
    let size = rect.size();
    if !radii_fit(size.width(), size.height(), radii.values())
        || !radii
            .values()
            .into_iter()
            .zip(outer_radii.values())
            .all(|(inner, outer)| {
                inner >= 0.0 && close(if inner == 0.0 { 0.0 } else { inner + half }, outer)
            })
    {
        return None;
    }
    Some(BorderedFill {
        rect: *rect,
        radii: *radii,
        width: *width,
        color: *color,
    })
}

pub(crate) fn radii_fit(width: f64, height: f64, [tl, tr, br, bl]: [f64; 4]) -> bool {
    // Match the CPU adapter's tolerance for proportional-fit rounding.
    let width = width * (1.0 + f64::EPSILON);
    let height = height * (1.0 + f64::EPSILON);
    tl + tr <= width && bl + br <= width && tl + bl <= height && tr + br <= height
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
