//! Ordered command lists for 16x16 physical-pixel tiles. Opacity scopes carry
//! the union of their draws, so each tile keeps balanced, ordered group commands.
use super::{Error, PixelSize, count, invalid};
use oxiview_base::{Affine, Point, Rect};

const EDGE: u32 = 16; // Mirrored in raster.wgsl.

#[derive(Clone, Copy)]
pub(super) struct Bounds([f64; 4]);
impl Bounds {
    pub const EMPTY: Self = Self([
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ]);

    pub fn shape(rect: Rect, outset: f64, transform: Affine) -> Self {
        let [a, b, c, d] = rect.edges();
        let (l, t, r, b) = (
            a.min(c) - outset,
            b.min(d) - outset,
            a.max(c) + outset,
            b.max(d) + outset,
        );
        if l >= r || t >= b {
            return Self::EMPTY;
        }
        let local_magnitude = l.abs().max(t.abs()).max(r.abs()).max(b.abs());
        let mut bounds = Self::EMPTY;
        for (x, y) in [(l, t), (r, t), (l, b), (r, b)] {
            let p = transform.transform_point(Point::new(x, y));
            bounds = bounds.union(Self([p.x(), p.y(), p.x(), p.y()]));
        }
        let Self([l, t, r, b]) = bounds;
        // Include the antialiasing fringe and f32 conversion error. Overflow or
        // poorly conditioned transforms disable culling, preserving coverage.
        let [a, b0, c, d, tx, ty] = transform.coefficients();
        let norm = a.abs().max(b0.abs()).max(c.abs()).max(d.abs());
        let det = (a * d - b0 * c).abs();
        // Include the pre-cancellation terms: large local coordinates translated
        // back into the viewport can still lose precision in the GPU inverse.
        let magnitude = l.abs().max(t.abs()).max(r.abs()).max(b.abs())
            + norm * local_magnitude
            + tx.abs().max(ty.abs());
        let pad = 1.0 + magnitude * 1e-6 * (norm * norm / det).max(1.0);
        if !pad.is_finite() {
            return Self([
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::INFINITY,
            ]);
        }
        Self([l - pad, t - pad, r + pad, b + pad])
    }

    pub fn union(self, other: Self) -> Self {
        let Self([l, t, r, b]) = self;
        let Self([x, y, z, w]) = other;
        Self([l.min(x), t.min(y), r.max(z), b.max(w)])
    }

    pub fn intersect(self, other: Self) -> Self {
        let Self([l, t, r, b]) = self;
        let Self([x, y, z, w]) = other;
        Self([l.max(x), t.max(y), r.min(z), b.min(w)])
    }
}

pub(super) fn append(
    words: &mut Vec<u32>,
    operations: &[(u32, Bounds)],
    size: PixelSize,
) -> Result<(), Error> {
    let columns = size.width.div_ceil(EDGE);
    let rows = size.height.div_ceil(EDGE);
    let tile_count = columns.checked_mul(rows).ok_or(Error::InvalidSize(size))? as usize;
    let table_words = tile_count.checked_mul(2).ok_or(Error::InvalidSize(size))?;
    count(
        words
            .len()
            .checked_add(table_words)
            .ok_or(Error::InvalidSize(size))?,
    )?;
    let mut tiles = Vec::new();
    tiles
        .try_reserve_exact(tile_count)
        .map_err(|_| invalid("tile allocation failed"))?;
    tiles.resize_with(tile_count, Vec::new);
    for &(command, Bounds([l, t, r, b])) in operations {
        if l >= r || t >= b {
            continue;
        }
        let x0 = (l.max(0.0) / f64::from(EDGE))
            .floor()
            .min(f64::from(columns)) as u32;
        let y0 = (t.max(0.0) / f64::from(EDGE)).floor().min(f64::from(rows)) as u32;
        let x1 = (r.max(0.0) / f64::from(EDGE))
            .ceil()
            .min(f64::from(columns)) as u32;
        let y1 = (b.max(0.0) / f64::from(EDGE)).ceil().min(f64::from(rows)) as u32;
        for y in y0..y1 {
            for x in x0..x1 {
                tiles
                    .get_mut((y * columns + x) as usize)
                    .ok_or_else(|| invalid("invalid tile"))?
                    .push(command);
            }
        }
    }
    let total = tiles
        .iter()
        .try_fold(words.len() + table_words, |n, tile| {
            n.checked_add(tile.len())
        })
        .ok_or_else(|| invalid("tile lists exceed address space"))?;
    count(total)?;
    words
        .try_reserve_exact(total - words.len())
        .map_err(|_| invalid("tile allocation failed"))?;
    let mut cursor = words.len() + table_words;
    for tile in &tiles {
        words.extend([count(cursor)?, count(tile.len())?]);
        cursor += tile.len();
    }
    for tile in tiles {
        words.extend(tile);
    }
    Ok(())
}
