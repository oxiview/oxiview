use std::ops::Range;

const MAX_EXTENT: f64 = f32::MAX as f64;

/// An invalid size, index, or aggregate extent for a virtual layout axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExtentError {
    /// An item size must be finite, positive, and within the layout coordinate domain.
    #[error("invalid virtual item size")]
    InvalidSize,
    /// The aggregate extent exceeds the layout coordinate domain.
    #[error("virtual extent is too large")]
    TooLarge,
    /// The requested position does not identify an item in the axis.
    #[error("virtual extent index is out of bounds")]
    InvalidIndex,
}

fn check_size(size: f64) -> Result<(), ExtentError> {
    if size <= 0.0 || !size.is_finite() || size > MAX_EXTENT {
        return Err(ExtentError::InvalidSize);
    }
    Ok(())
}

fn check_total(extent: f64) -> Result<(), ExtentError> {
    if !extent.is_finite() || extent > MAX_EXTENT {
        return Err(ExtentError::TooLarge);
    }
    Ok(())
}

/// Geometry and viewport queries shared by fixed and variable virtual axes.
pub trait VirtualExtent {
    /// The number of items along the axis.
    fn len(&self) -> usize;
    /// Whether the axis contains zero items.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The combined size of all items in logical points.
    fn total(&self) -> f64;
    /// The position of an item boundary. Index `len()` denotes the end of the axis.
    fn offset(&self, index: usize) -> Option<f64>;
    /// The size of an item in logical points.
    fn size(&self, index: usize) -> Option<f64>;
    /// The item containing the offset. Items own their leading edge, and the final edge is outside.
    fn index_at(&self, offset: f64) -> Option<usize>;

    /// Items intersecting the half-open viewport, with up to `overscan` extra items at each end.
    /// An empty intersection remains empty. A non-finite offset or viewport yields an empty range.
    fn visible_range(&self, offset: f64, viewport: f64, overscan: usize) -> Range<usize> {
        if !offset.is_finite() || !viewport.is_finite() || viewport <= 0.0 || self.is_empty() {
            return 0..0;
        }
        let leading = offset.max(0.0);
        let trailing = (offset + viewport).min(self.total());
        if leading >= trailing {
            let empty = if leading >= self.total() {
                self.len()
            } else {
                0
            };
            return empty..empty;
        }
        let start = self.index_at(leading).unwrap_or(self.len());
        let end = self.index_at(trailing).map_or(self.len(), |index| {
            index + usize::from(self.offset(index).is_some_and(|edge| edge < trailing))
        });
        start.saturating_sub(overscan)..end.saturating_add(overscan).min(self.len())
    }

    /// The offset that reveals the item with the smallest movement and clamps it to the axis.
    /// An item larger than the viewport aligns to the top when scrolling forward into it.
    fn reveal(&self, index: usize, offset: f64, viewport: f64) -> Option<f64> {
        if index >= self.len() || !offset.is_finite() || !viewport.is_finite() || viewport < 0.0 {
            return None;
        }
        let start = self.offset(index)?;
        let end = self.offset(index + 1)?;
        let target = if start < offset {
            start
        } else if end > offset + viewport {
            (end - viewport).min(start)
        } else {
            offset
        };
        Some(target.clamp(0.0, (self.total() - viewport).max(0.0)))
    }
}

/// A virtual axis with equal item sizes and constant-time position and range queries.
#[derive(Clone, Copy, Debug)]
pub struct FixedExtent {
    len: usize,
    size: f64,
}

impl FixedExtent {
    /// Creates a fixed axis. Empty axes still require a valid representative item size.
    pub fn new(len: usize, size: f64) -> Result<Self, ExtentError> {
        check_size(size)?;
        check_total(len as f64 * size)?;
        Ok(Self { len, size })
    }

    /// Changes the item count while preserving the uniform size.
    pub fn set_len(&mut self, len: usize) -> Result<(), ExtentError> {
        check_total(len as f64 * self.size)?;
        self.len = len;
        Ok(())
    }
}

impl VirtualExtent for FixedExtent {
    fn len(&self) -> usize {
        self.len
    }

    fn total(&self) -> f64 {
        self.len as f64 * self.size
    }

    fn offset(&self, index: usize) -> Option<f64> {
        (index <= self.len).then_some(index as f64 * self.size)
    }

    fn size(&self, index: usize) -> Option<f64> {
        (index < self.len).then_some(self.size)
    }

    fn index_at(&self, offset: f64) -> Option<usize> {
        if !offset.is_finite() || offset < 0.0 || offset >= self.total() {
            return None;
        }
        let index = ((offset / self.size) as usize).min(self.len - 1);
        // Division and multiplication can round on opposite sides of an item boundary.
        if index as f64 * self.size > offset {
            Some(index.saturating_sub(1))
        } else if index + 1 < self.len && (index + 1) as f64 * self.size <= offset {
            Some(index + 1)
        } else {
            Some(index)
        }
    }
}

/// A virtual axis backed by a balanced sum index. Queries and point updates take logarithmic
/// time. Appends grow the backing index geometrically, so a resize can rebuild the index.
#[derive(Clone, Debug)]
pub struct VariableExtent {
    len: usize,
    base: usize,
    sums: Vec<f64>,
}

impl VariableExtent {
    /// Builds the index from positive sizes in display order.
    pub fn new(sizes: impl IntoIterator<Item = f64>) -> Result<Self, ExtentError> {
        let sizes: Vec<_> = sizes.into_iter().collect();
        Self::from_sizes(&sizes)
    }

    fn from_sizes(sizes: &[f64]) -> Result<Self, ExtentError> {
        for &size in sizes {
            check_size(size)?;
        }
        let (base, mut sums) = Self::storage(sizes.len())?;
        sums[base..base + sizes.len()].copy_from_slice(sizes);
        Self::from_leaves(sizes.len(), base, sums)
    }

    fn storage(len: usize) -> Result<(usize, Vec<f64>), ExtentError> {
        let base = len
            .checked_next_power_of_two()
            .ok_or(ExtentError::TooLarge)?;
        let capacity = base.checked_mul(2).ok_or(ExtentError::TooLarge)?;
        let mut sums = Vec::new();
        sums.try_reserve_exact(capacity)
            .map_err(|_| ExtentError::TooLarge)?;
        sums.resize(capacity, 0.0);
        Ok((base, sums))
    }

    fn from_leaves(len: usize, base: usize, mut sums: Vec<f64>) -> Result<Self, ExtentError> {
        for node in (1..base).rev() {
            sums[node] = sums[2 * node] + sums[2 * node + 1];
        }
        check_total(sums[1])?;
        Ok(Self { len, base, sums })
    }

    /// Replaces one size, without changing the index if validation fails.
    pub fn set_size(&mut self, index: usize, size: f64) -> Result<bool, ExtentError> {
        let previous = self.size(index).ok_or(ExtentError::InvalidIndex)?;
        check_size(size)?;
        if previous == size {
            return Ok(false);
        }
        // Validate the actual sum along the affected path, avoiding cancellation when a
        // large item shrinks and retaining the previous extent on failure.
        let mut sum = size;
        let mut node = self.base + index;
        while node > 1 {
            sum += self.sums[node ^ 1];
            node /= 2;
        }
        check_total(sum)?;
        self.set_leaf(index, size);
        Ok(true)
    }

    /// Appends `count` equally sized items in one update.
    pub fn append(&mut self, count: usize, size: f64) -> Result<(), ExtentError> {
        check_size(size)?;
        let len = self.len.checked_add(count).ok_or(ExtentError::TooLarge)?;
        check_total(self.total() + count as f64 * size)?;
        if len > self.base {
            let (base, mut sums) = Self::storage(len)?;
            sums[base..base + self.len]
                .copy_from_slice(&self.sums[self.base..self.base + self.len]);
            sums[base + self.len..base + len].fill(size);
            *self = Self::from_leaves(len, base, sums)?;
        } else {
            for index in self.len..len {
                self.set_leaf(index, size);
            }
            self.len = len;
        }
        Ok(())
    }

    fn set_leaf(&mut self, index: usize, size: f64) {
        let mut node = self.base + index;
        self.sums[node] = size;
        while node > 1 {
            node /= 2;
            self.sums[node] = self.sums[node * 2] + self.sums[node * 2 + 1];
        }
    }
}

impl VirtualExtent for VariableExtent {
    fn len(&self) -> usize {
        self.len
    }

    fn total(&self) -> f64 {
        self.sums[1]
    }

    fn offset(&self, index: usize) -> Option<f64> {
        if index > self.len {
            return None;
        }
        if index == self.len {
            return Some(self.total());
        }
        // Add left-subtree sums in traversal order, matching index_at's boundaries.
        let mut node = 1;
        let mut offset = 0.0;
        let mut bit = self.base >> 1;
        while bit > 0 {
            node *= 2;
            if index & bit != 0 {
                offset += self.sums[node];
                node += 1;
            }
            bit >>= 1;
        }
        Some(offset)
    }

    fn size(&self, index: usize) -> Option<f64> {
        (index < self.len).then(|| self.sums[self.base + index])
    }

    fn index_at(&self, offset: f64) -> Option<usize> {
        if !offset.is_finite() || offset < 0.0 || offset >= self.total() {
            return None;
        }
        let mut node = 1;
        let mut start = 0.0;
        while node < self.base {
            node *= 2;
            let middle = start + self.sums[node];
            if offset >= middle {
                node += 1;
                start = middle;
            }
        }
        // The two accumulation orders can round the last edge differently.
        Some((node - self.base).min(self.len - 1))
    }
}

#[cfg(test)]
mod tests;
