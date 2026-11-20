//! Retained scene language for the rendering context (role: port).
//!
//! Roots can serve as reusable definitions. Encode a selected root to draw it;
//! unrelated roots are not painted. Every node can own ordered children. Its
//! content is visited before those children; scopes enclose all descendants.
//! References visit their target before their own children, under the reference
//! transform. A target's ownership ancestors are not inherited by an instance.
//! Edits are atomic. Removing a referenced subtree is rejected until outside
//! references are removed. Identity is local to a scene and is never recycled.

use oxiview_base::{Affine, Color, CornerRadii, Point, Rect};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub use oxiview_base::{PixelSize, RgbaImage};

mod encoding;
pub use encoding::{EncodedNode, EncodedScene, ResolvedClip};

/// Stable identity within the scene that allocated it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u64);

/// One linear-gradient stop, validated when inserted into a scene.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorStop {
    /// Position in the inclusive unit interval.
    pub position: f64,
    /// Straight-alpha color.
    pub color: Color,
}

/// Paint in the local coordinate space of its primitive.
#[derive(Clone, Debug, PartialEq)]
pub enum Brush {
    /// Uniform color.
    Solid(Color),
    /// Start, end, and at least two stops in nondecreasing position order.
    /// Equal positions represent a hard transition; values outside the first
    /// and last stop extend the respective endpoint color.
    LinearGradient(Point, Point, Vec<ColorStop>),
}

/// A clipping shape in local coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum Clip {
    /// Axis-aligned rectangle.
    Rect(Rect),
    /// Rectangle with circular corner radii.
    RoundedRect(Rect, CornerRadii),
}

/// A draw operation, with its geometry and brush in the same local space.
#[derive(Clone, Debug, PartialEq)]
pub enum Primitive {
    /// Filled rounded rectangle.
    Fill(Rect, CornerRadii, Brush),
    /// Rounded rectangle with a centered stroke of the given width.
    Stroke(Rect, CornerRadii, f64, Brush),
    /// Straight-sRGB image sampled into a logical destination rectangle.
    /// Image storage is shared by clones; adapters filter premultiplied pixels.
    Image(Rect, RgbaImage),
}

/// Content of a subtree node. Children are supported by every variant.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// Paint a primitive, then its children.
    Draw(Primitive),
    /// Transform the entire subtree.
    Transform(Affine),
    /// Intersect this clip with all enclosing clips.
    Clip(Clip),
    /// Composite the entire group once with this opacity.
    Opacity(f64),
    /// Visit an existing subtree under a placement transform.
    Reference(NodeId, Affine),
}

/// A rejected scene edit or lookup.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Pixel byte length does not match its dimensions.
    #[error("invalid RGBA8 image length")]
    InvalidImage,
    /// The identity does not exist in this scene.
    #[error("scene node does not exist: {0:?}")]
    MissingNode(NodeId),
    /// An insertion position exceeds the child count.
    #[error("child index {index} exceeds child count {len}")]
    ChildIndex {
        /// Requested position.
        index: usize,
        /// Available positions end here, inclusively.
        len: usize,
    },
    /// Ownership and reference edges would form a cycle.
    #[error("scene edit would create a cycle")]
    Cycle,
    /// A surviving reference targets a node being removed.
    #[error("subtree is still referenced: {0:?}")]
    SubtreeReferenced(NodeId),
    /// Gradient stops must be ordered, in range, and number at least two.
    #[error("gradient requires at least two ordered stops in 0..=1")]
    InvalidGradient,
    /// Opacity must be finite and in range.
    #[error("opacity must lie in 0..=1")]
    InvalidOpacity,
    /// Stroke widths must be finite and nonnegative.
    #[error("stroke width must be finite and nonnegative")]
    InvalidStrokeWidth,
    /// No further unique identities can be allocated.
    #[error("scene node identities exhausted")]
    IdentityExhausted,
}

#[derive(Clone, Debug)]
struct Node {
    kind: NodeKind,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
}

#[derive(Clone, Debug, Default)]
struct Forest {
    nodes: BTreeMap<NodeId, Node>,
    roots: Vec<NodeId>,
}

/// An editable retained forest, including reusable subtree definitions.
///
/// Equality compares ordered structure, content, and reference topology, ignoring
/// allocated identity values and removed-node history. It does not compare pixels.
/// Clones share the forest until an edit. Already published scenes keep their contents.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    forest: Arc<Forest>,
    next_id: u64,
}

impl Scene {
    /// Creates an empty scene.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns roots in their ownership order.
    pub fn roots(&self) -> &[NodeId] {
        &self.forest.roots
    }

    /// Returns a node's content.
    pub fn content(&self, id: NodeId) -> Result<&NodeKind, Error> {
        Ok(&self.node(id)?.kind)
    }

    /// Returns ordered children, or roots when `parent` is `None`.
    pub fn children(&self, parent: Option<NodeId>) -> Result<&[NodeId], Error> {
        match parent {
            Some(id) => Ok(&self.node(id)?.children),
            None => Ok(self.roots()),
        }
    }

    /// Inserts content at a child index, or at a root index for `None`.
    /// References must target nodes that exist before this edit.
    pub fn insert(
        &mut self,
        parent: Option<NodeId>,
        index: usize,
        kind: NodeKind,
    ) -> Result<NodeId, Error> {
        self.validate_kind(&kind)?;
        let next = self
            .next_id
            .checked_add(1)
            .ok_or(Error::IdentityExhausted)?;
        let len = self.children(parent)?.len();
        if index > len {
            return Err(Error::ChildIndex { index, len });
        }
        // A new leaf cannot close a cycle unless it references an existing
        // subtree that reaches its parent. Root insertions have no incoming edge.
        if let (Some(parent), NodeKind::Reference(target, _)) = (parent, &kind) {
            self.validate_edge(parent, *target)?;
        }
        let id = NodeId(self.next_id);
        self.attach(parent, index, id)?;
        Arc::make_mut(&mut self.forest).nodes.insert(
            id,
            Node {
                kind,
                parent,
                children: Vec::new(),
            },
        );
        self.next_id = next;
        Ok(id)
    }

    /// Replaces content while preserving identity, parent, and children.
    pub fn replace(&mut self, id: NodeId, kind: NodeKind) -> Result<(), Error> {
        self.node(id)?;
        self.validate_kind(&kind)?;
        if let NodeKind::Reference(target, _) = &kind {
            self.validate_edge(id, *target)?;
        }
        self.node_mut(id)?.kind = kind;
        Ok(())
    }

    /// Moves a subtree. The index addresses the destination after detaching it.
    pub fn reparent(
        &mut self,
        id: NodeId,
        parent: Option<NodeId>,
        index: usize,
    ) -> Result<(), Error> {
        let old_parent = self.node(id)?.parent;
        let len = self.children(parent)?.len() - usize::from(parent == old_parent);
        if index > len {
            return Err(Error::ChildIndex { index, len });
        }
        if let Some(parent) = parent
            && old_parent != Some(parent)
        {
            self.validate_edge(parent, id)?;
        }
        // Both nodes and the post-detach index are validated before mutation.
        // Removing the old edge cannot introduce a cycle or invalidate a node.
        self.detach(id)?;
        self.attach(parent, index, id)?;
        self.node_mut(id)?.parent = parent;
        Ok(())
    }

    /// Removes the ownership subtree, but not referenced definitions.
    /// Fails atomically if any surviving node references a removed node.
    pub fn remove(&mut self, id: NodeId) -> Result<(), Error> {
        self.node(id)?;
        let mut removed = BTreeSet::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            removed.insert(current);
            stack.extend(&self.node(current)?.children);
        }
        for (key, node) in &self.forest.nodes {
            if !removed.contains(key)
                && let NodeKind::Reference(target, _) = node.kind
                && removed.contains(&target)
            {
                return Err(Error::SubtreeReferenced(target));
            }
        }
        self.detach(id)?;
        Arc::make_mut(&mut self.forest)
            .nodes
            .retain(|key, _| !removed.contains(key));
        Ok(())
    }

    fn node(&self, id: NodeId) -> Result<&Node, Error> {
        self.forest.nodes.get(&id).ok_or(Error::MissingNode(id))
    }
    fn node_mut(&mut self, id: NodeId) -> Result<&mut Node, Error> {
        Arc::make_mut(&mut self.forest)
            .nodes
            .get_mut(&id)
            .ok_or(Error::MissingNode(id))
    }
    fn children_mut(&mut self, parent: Option<NodeId>) -> Result<&mut Vec<NodeId>, Error> {
        match parent {
            Some(id) => Ok(&mut self.node_mut(id)?.children),
            None => Ok(&mut Arc::make_mut(&mut self.forest).roots),
        }
    }
    fn attach(&mut self, parent: Option<NodeId>, index: usize, id: NodeId) -> Result<(), Error> {
        let children = self.children_mut(parent)?;
        if index > children.len() {
            return Err(Error::ChildIndex {
                index,
                len: children.len(),
            });
        }
        children.insert(index, id);
        Ok(())
    }
    fn detach(&mut self, id: NodeId) -> Result<(), Error> {
        let parent = self.node(id)?.parent;
        self.children_mut(parent)?.retain(|child| *child != id);
        Ok(())
    }
    fn validate_kind(&self, kind: &NodeKind) -> Result<(), Error> {
        match kind {
            NodeKind::Reference(id, _) => {
                self.node(*id)?;
            }
            NodeKind::Opacity(value) if !(0.0..=1.0).contains(value) => {
                return Err(Error::InvalidOpacity);
            }
            NodeKind::Draw(primitive) => {
                let brush = match primitive {
                    Primitive::Fill(_, _, brush) => brush,
                    Primitive::Stroke(_, _, width, brush) => {
                        if !width.is_finite() || *width < 0.0 {
                            return Err(Error::InvalidStrokeWidth);
                        }
                        brush
                    }
                    Primitive::Image(rect, image) => {
                        if image.width() == 0
                            || image.height() == 0
                            || !rect.edges().iter().all(|value| value.is_finite())
                            || rect.size().width() <= 0.0
                            || rect.size().height() <= 0.0
                        {
                            return Err(Error::InvalidImage);
                        }
                        return Ok(());
                    }
                };
                if let Brush::LinearGradient(_, _, stops) = brush
                    && (stops.len() < 2
                        || stops
                            .iter()
                            .any(|stop| !(0.0..=1.0).contains(&stop.position))
                        || stops
                            .windows(2)
                            .any(|pair| pair[0].position > pair[1].position))
                {
                    return Err(Error::InvalidGradient);
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn validate_edge(&self, source: NodeId, target: NodeId) -> Result<(), Error> {
        // The existing graph is acyclic. An added edge can only create a cycle
        // if its target already reaches its source through children/references.
        let mut visited = BTreeSet::new();
        let mut stack = vec![target];
        while let Some(current) = stack.pop() {
            if current == source {
                return Err(Error::Cycle);
            }
            if !visited.insert(current) {
                continue;
            }
            let node = self.node(current)?;
            stack.extend(&node.children);
            if let NodeKind::Reference(target, _) = node.kind {
                stack.push(target);
            }
        }
        Ok(())
    }
    fn canonical(&self) -> Vec<(NodeKind, usize)> {
        let mut order = Vec::new();
        let mut stack: Vec<_> = self.roots().iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            if let Some(node) = self.forest.nodes.get(&id) {
                order.push(id);
                stack.extend(node.children.iter().rev());
            }
        }
        let ids: BTreeMap<_, _> = order
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, NodeId(i as u64)))
            .collect();
        order
            .iter()
            .filter_map(|id| self.forest.nodes.get(id))
            .map(|node| {
                let mut kind = node.kind.clone();
                if let NodeKind::Reference(target, _) = &mut kind
                    && let Some(canonical) = ids.get(target)
                {
                    *target = *canonical;
                }
                (kind, node.children.len())
            })
            .collect()
    }
}
impl PartialEq for Scene {
    fn eq(&self, other: &Self) -> bool {
        self.canonical() == other.canonical()
    }
}

#[cfg(test)]
mod tests;
