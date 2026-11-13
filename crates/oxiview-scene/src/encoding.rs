use crate::{Clip, Error, NodeId, NodeKind, Primitive, Scene};
use oxiview_base::Affine;

/// A clip with its complete local-to-scene transform.
///
/// Keeping the shape and affine together preserves rotated, sheared, and rounded
/// boundaries exactly. All clips on a draw are intersected, not unioned.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedClip {
    /// Original clip geometry.
    pub shape: Clip,
    /// Fully composed transform at the clip's declaration.
    pub transform: Affine,
}

/// An owned operation in paint order, with no retained node references.
#[derive(Clone, Debug, PartialEq)]
pub enum EncodedNode {
    /// Paint geometry with its resolved transform and complete clip intersection.
    Draw {
        /// Local geometry and brush, transformed together.
        primitive: Primitive,
        /// Fully composed local-to-scene transform.
        transform: Affine,
        /// Enclosing clip boundaries with their own resolved transforms.
        clips: Vec<ResolvedClip>,
    },
    /// Begin an isolated compositing group; opacity applies once to its result.
    BeginOpacity(f64),
    /// End the most recent opacity group.
    EndOpacity,
}

/// Plain owned renderer input. References are expanded for each placement.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EncodedScene {
    /// Operations in depth-first paint order; opacity boundaries are balanced.
    pub nodes: Vec<EncodedNode>,
}

#[derive(Clone)]
struct State {
    transform: Affine,
    clips: Vec<ResolvedClip>,
}
enum Work {
    Visit(NodeId, State),
    EndOpacity,
}

impl Scene {
    /// Encodes the selected subtree, including every reference instance.
    ///
    /// Ancestors outside `root` do not contribute state. Later draws paint over
    /// earlier draws. Transform and clip scopes are resolved into each draw;
    /// opacity boundaries remain explicit because per-draw alpha multiplication
    /// would incorrectly composite overlapping children. Traversal is iterative.
    pub fn encode(&self, root: NodeId) -> Result<EncodedScene, Error> {
        self.node(root)?;
        let mut output = EncodedScene::default();
        let mut stack = vec![Work::Visit(
            root,
            State {
                transform: Affine::IDENTITY,
                clips: Vec::new(),
            },
        )];
        while let Some(work) = stack.pop() {
            let Work::Visit(id, mut state) = work else {
                output.nodes.push(EncodedNode::EndOpacity);
                continue;
            };
            let node = self.node(id)?;
            match &node.kind {
                NodeKind::Draw(primitive) => output.nodes.push(EncodedNode::Draw {
                    primitive: primitive.clone(),
                    transform: state.transform,
                    clips: state.clips.clone(),
                }),
                NodeKind::Transform(transform) | NodeKind::Reference(_, transform) => {
                    state.transform = transform.then(state.transform);
                }
                NodeKind::Clip(shape) => state.clips.push(ResolvedClip {
                    shape: shape.clone(),
                    transform: state.transform,
                }),
                NodeKind::Opacity(opacity) => {
                    output.nodes.push(EncodedNode::BeginOpacity(*opacity));
                    stack.push(Work::EndOpacity);
                }
            }
            for child in node.children.iter().rev() {
                stack.push(Work::Visit(*child, state.clone()));
            }
            if let NodeKind::Reference(target, _) = node.kind {
                stack.push(Work::Visit(target, state));
            }
        }
        Ok(output)
    }
}
