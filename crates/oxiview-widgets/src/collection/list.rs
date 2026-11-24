use super::{
    CollectionError, MountStats,
    mount::{Mounts, Placement},
    order::KeyOrder,
};
use oxiview_base::{Point, Rect, Size};
use oxiview_core::{NodeId, UiTree, View};
use oxiview_layout::{FixedExtent, VariableExtent, VirtualExtent};
use std::ops::Range;

/// The stable top row and the logical offset into that row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListAnchor {
    /// Stable data key of the anchored row.
    pub key: u64,
    /// Distance from the leading edge of the row.
    pub intra_row: f64,
}

/// Data and content-space geometry passed to a newly mounted list row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListRow {
    /// Stable row identity.
    pub key: u64,
    /// Current zero-based position in the ordered data source.
    pub index: usize,
    /// Frame relative to the virtual content origin.
    pub frame: Rect,
}

/// Keyed virtual list with estimated heights and a retained window of row Views.
///
/// Keep the viewport parent mounted across reconciliations and let it own focus, clipping and
/// scroll offset. Use a positioned parent whose direct children belong to this reconciler.
pub struct VirtualList {
    order: KeyOrder,
    extent: VariableExtent,
    estimate: f64,
    overscan: usize,
    mounts: Mounts<u64>,
}

impl VirtualList {
    /// Creates an empty list with a positive estimate and a row count margin on each edge.
    pub fn new(estimated_height: f64, overscan: usize) -> Result<Self, CollectionError> {
        FixedExtent::new(0, estimated_height)?;
        Ok(Self {
            order: KeyOrder::default(),
            extent: VariableExtent::new([])?,
            estimate: estimated_height,
            overscan,
            mounts: Mounts::default(),
        })
    }

    /// Replaces the display order, preserving the heights of keys shared with the old order.
    pub fn set_rows(&mut self, keys: impl IntoIterator<Item = u64>) -> Result<(), CollectionError> {
        let order = KeyOrder::new(keys)?;
        let extent = VariableExtent::new(order.keys.iter().map(|&key| {
            self.index_of(key)
                .and_then(|index| self.height(index))
                .unwrap_or(self.estimate)
        }))?;
        self.order = order;
        self.extent = extent;
        Ok(())
    }

    /// Appends fresh keys at the estimated height without rebuilding the displayed key order.
    pub fn append_rows(
        &mut self,
        keys: impl IntoIterator<Item = u64>,
    ) -> Result<(), CollectionError> {
        let keys = self.order.checked_tail(keys)?;
        self.extent.append(keys.len(), self.estimate)?;
        self.order.append(keys);
        Ok(())
    }

    /// The number of rows in the ordered model.
    pub fn len(&self) -> usize {
        self.extent.len()
    }

    /// Whether the ordered model is empty.
    pub fn is_empty(&self) -> bool {
        self.extent.is_empty()
    }

    /// Returns the current index of a displayed key, whether or not it is mounted.
    pub fn index_of(&self, key: u64) -> Option<usize> {
        self.order.index(key)
    }

    /// Returns the key at a displayed index.
    pub fn key_at(&self, index: usize) -> Option<u64> {
        self.order.key(index)
    }

    /// Returns the index of the row containing the given content-space offset.
    pub fn index_at(&self, offset: f64) -> Option<usize> {
        self.extent.index_at(offset)
    }

    /// Returns the estimated or measured height at the displayed index.
    pub fn height(&self, index: usize) -> Option<f64> {
        self.extent.size(index)
    }

    /// Returns the content-space leading edge, including the final edge at `len()`.
    pub fn row_offset(&self, index: usize) -> Option<f64> {
        self.extent.offset(index)
    }

    /// Returns the height of the entire virtual content.
    pub fn total_height(&self) -> f64 {
        self.extent.total()
    }

    /// Returns the viewport's intersecting rows before adding overscan.
    pub fn visible_rows(&self, offset: f64, viewport_height: f64) -> Range<usize> {
        self.extent.visible_range(offset, viewport_height, 0)
    }

    /// Captures the key and position within the row at the viewport's leading edge.
    pub fn anchor(&self, offset: f64) -> Option<ListAnchor> {
        let index = self.index_at(offset)?;
        Some(ListAnchor {
            key: self.key_at(index)?,
            intra_row: offset - self.row_offset(index)?,
        })
    }

    /// Restores a captured leading edge after the list order or measured heights change.
    /// The caller can clamp the restored offset to the current viewport's scroll limits.
    pub fn offset_for_anchor(&self, anchor: ListAnchor) -> Option<f64> {
        if !anchor.intra_row.is_finite() {
            return None;
        }
        let index = self.index_of(anchor.key)?;
        let start = self.row_offset(index)?;
        let last = self.row_offset(index + 1)?.next_down().max(start);
        Some((start + anchor.intra_row.max(0.0)).min(last))
    }

    /// Updates one height in logarithmic time, including for an unmounted row.
    pub fn set_height(&mut self, key: u64, height: f64) -> Result<bool, CollectionError> {
        let index = self.index_of(key).ok_or(CollectionError::UnknownKey(key))?;
        Ok(self.extent.set_size(index, height)?)
    }

    /// Invalidates all old measurements, for example after the available row width changes.
    pub fn reset_heights(&mut self, estimate: f64) -> Result<(), CollectionError> {
        FixedExtent::new(self.len(), estimate)?;
        self.extent = VariableExtent::new(std::iter::repeat_n(estimate, self.len()))?;
        self.estimate = estimate;
        Ok(())
    }

    /// Returns the clamped offset needed to reveal a displayed key.
    pub fn reveal(&self, key: u64, offset: f64, viewport_height: f64) -> Option<f64> {
        self.extent
            .reveal(self.index_of(key)?, offset, viewport_height)
    }

    /// Mounts entering rows, releases departing ones, and updates existing placements in place.
    /// Builder roots are stretched and clipped inside absolute row wrappers. The builder runs
    /// once per mount. Use the retained root or its bindings to update existing row content.
    pub fn reconcile(
        &mut self,
        tree: &mut UiTree,
        parent: NodeId,
        offset: f64,
        viewport: Size,
        mut build: impl FnMut(ListRow) -> View,
    ) -> Result<MountStats, CollectionError> {
        tree.children(parent)?;
        let range = self
            .extent
            .visible_range(offset, viewport.height(), self.overscan);
        let disposed = self.mounts.prune(tree, |key| {
            self.order
                .index(key)
                .filter(|index| range.contains(index))
                .map(|_| parent)
        })?;
        let placements = range.enumerate().filter_map(|(index, row)| {
            let key = self.order.key(row)?;
            let frame = Rect::from_origin_size(
                Point::new(0.0, self.extent.offset(row)?),
                Size::new(viewport.width(), self.extent.size(row)?),
            );
            Some(Placement {
                key,
                parent,
                index,
                frame,
                item: ListRow {
                    key,
                    index: row,
                    frame,
                },
            })
        });
        let mut stats = self
            .mounts
            .place(tree, placements, |row| Some(build(row)))?;
        stats.disposed = disposed;
        Ok(stats)
    }

    /// Returns the builder root for a key in the last reconciled window.
    pub fn row_node(&self, key: u64) -> Option<NodeId> {
        self.mounts.view(key)
    }

    /// Iterates the last reconciled window's row keys and builder roots in unspecified order.
    pub fn mounted_rows(&self) -> impl Iterator<Item = (u64, NodeId)> + '_ {
        self.mounts.views()
    }

    /// Releases one row View so its builder runs again if the next window contains that key.
    pub fn invalidate_row(&mut self, tree: &mut UiTree, key: u64) -> Result<bool, CollectionError> {
        self.mounts.remove(tree, key)
    }

    /// Unmounts all list rows while keeping the ordered data and extent index.
    pub fn clear(&mut self, tree: &mut UiTree) -> Result<(), CollectionError> {
        self.mounts.clear(tree)
    }
}
