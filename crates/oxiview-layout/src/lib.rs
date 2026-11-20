//! Owned layout protocol and a private Taffy adapter over the caller's retained tree.

mod adapter;
mod types;
mod validation;
pub use adapter::{IncrementalTaffyLayout, TaffyLayout};
use oxiview_base::{Point, Size};
use std::hash::Hash;
pub use types::*;
pub use validation::InputError;

/// The maximum supported number of node levels, including the root.
pub const MAX_LAYOUT_DEPTH: usize = 64;
/// Owned layout input properties, independent of a particular layout implementation.
pub type LayoutInput = LayoutStyle;

/// Available space on one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AvailableSpace {
    /// A finite logical size.
    Definite(f64),
    /// The smallest intrinsic contribution.
    MinContent,
    /// The largest intrinsic contribution.
    MaxContent,
}
/// Committed geometry relative to the structural parent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutBox {
    /// Border-box origin relative to the structural parent.
    pub location: Point,
    /// Border-box dimensions.
    pub size: Size,
    /// Content dimensions before painting clips.
    pub content_size: Size,
    /// Resolved padding widths.
    pub padding: Edges<f64>,
    /// Resolved border widths.
    pub border: Edges<f64>,
}
/// Work performed during one successful layout computation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutStats {
    /// Number of nodes inspected while synchronizing layout inputs.
    pub visited_nodes: usize,
    /// Number of style projections requested from the caller.
    pub resolved_styles: usize,
    /// Number of layout algorithm entry calls, including cache hits and hidden nodes.
    pub layout_visits: usize,
    /// Number of uncached layout algorithm evaluations.
    pub computed_nodes: usize,
    /// Number of cached layout results reused.
    pub cache_hits: usize,
    /// Number of calls to [`LayoutTree::measure`].
    pub measured_nodes: usize,
}

/// A coalesced mutation since the adapter's last successful layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutChange<I> {
    /// Updates a live node. New nodes must report both style and children.
    Update {
        /// The node's current structural parent, or none for the root.
        parent: Option<I>,
        /// Its projected style may have changed.
        style: bool,
        /// Its ordered children may have changed.
        children: bool,
    },
    /// A node was removed. Report every node removed from a subtree.
    Remove,
}
/// Errors leave previously committed geometry intact.
#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    /// The viewport is non-finite, negative, or outside supported numeric range.
    #[error("invalid logical viewport")]
    InvalidViewport,
    /// A node's input style is invalid.
    #[error(transparent)]
    InvalidInput(#[from] InputError),
    /// The retained structure contains a duplicate, cycle, or missing node.
    #[error("layout requires a valid rooted tree")]
    InvalidTree,
    /// The retained tree is deeper than the supported traversal limit.
    #[error("layout tree exceeds the supported depth")]
    ExcessiveDepth,
    /// Custom measurement or computed geometry is invalid.
    #[error("layout or intrinsic measurement produced invalid geometry")]
    InvalidGeometry,
    /// Private cache identities were exhausted without wrapping or reuse.
    #[error("layout cache identities exhausted")]
    CacheExhausted,
}
/// Read-only topology with staged geometry publication and intrinsic measurement.
pub trait LayoutTree {
    /// Caller-owned identity; no Taffy keys cross this boundary.
    /// Distinguish logical nodes across removals and tree instances, or clear
    /// the layout engine's cache before reusing an identity for a different tree.
    type NodeId: Copy + Eq + Hash;
    /// Returns children in structural order.
    fn children(&self, node: Self::NodeId) -> &[Self::NodeId];
    /// Projects typed style into owned layout inputs.
    fn layout_input(&self, node: Self::NodeId) -> LayoutInput;
    /// Publishes geometry after the complete computation succeeds.
    /// With an incremental engine, unchanged geometry can remain at the caller.
    fn set_layout(&mut self, node: Self::NodeId, layout: LayoutBox);
    /// Checks that an identity belongs to a live node.
    fn contains_node(&self, _node: Self::NodeId) -> bool {
        true
    }
    /// Revision of intrinsic measurement data, independent of layout properties.
    /// Change this whenever measurement results can change without a style edit,
    /// including replacement of measurement callbacks or their captured data.
    /// The default is suitable only for immutable measurement content.
    fn content_revision(&self, _node: Self::NodeId) -> u64 {
        0
    }
    /// Measures a leaf's intrinsic content under the supplied constraints.
    fn measure(
        &mut self,
        _node: Self::NodeId,
        known: Dimensions<Option<f64>>,
        _available: Dimensions<AvailableSpace>,
    ) -> Dimensions<f64> {
        Dimensions {
            width: known.width.unwrap_or(0.0),
            height: known.height.unwrap_or(0.0),
        }
    }
}

/// Replaceable layout computation over caller-owned topology and identities.
pub trait LayoutEngine<I: Copy + Eq + Hash> {
    /// Computes a complete layout, publishing geometry only on success.
    fn compute<T: LayoutTree<NodeId = I>>(
        &mut self,
        tree: &mut T,
        root: I,
        viewport: Size,
    ) -> Result<LayoutStats, LayoutError>;

    /// Reports whether a node is a layout boundary in the last successful pass.
    ///
    /// `Some(true)` certifies that changes confined to the node's descendants
    /// leave its size and all other contributions to ancestor layout unchanged
    /// under the committed constraints, with the node's own inputs and position
    /// in the tree held fixed. Layout inside the subtree can still change.
    /// `Some(false)` means the engine did not certify independence for this node.
    /// `None` means no report is available, such as for an unknown node or an
    /// engine that uses this default implementation.
    ///
    /// A failed computation leaves the previous report intact. Pending edits,
    /// including removals, and new constraints are reflected only after success.
    fn layout_boundary(&self, _node: I) -> Option<bool> {
        None
    }
}

impl<I: Copy + Eq + Hash> LayoutEngine<I> for TaffyLayout<I> {
    fn compute<T: LayoutTree<NodeId = I>>(
        &mut self,
        tree: &mut T,
        root: I,
        viewport: Size,
    ) -> Result<LayoutStats, LayoutError> {
        TaffyLayout::compute(self, tree, root, viewport)
    }

    fn layout_boundary(&self, node: I) -> Option<bool> {
        self.committed_layout_boundary(node)
    }
}

#[cfg(test)]
mod tests;
