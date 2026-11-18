//! Owned layout protocol and a private Taffy adapter over the caller's retained tree.

mod adapter;
mod types;
mod validation;
pub use adapter::TaffyLayout;
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
    /// Number of nodes visited while validating the retained tree.
    pub visited_nodes: usize,
    /// Number of uncached layout algorithm evaluations.
    pub computed_nodes: usize,
    /// Number of cached layout results reused.
    pub cache_hits: usize,
    /// Number of calls to custom intrinsic measurement.
    pub measured_nodes: usize,
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
}

#[cfg(test)]
mod tests;
