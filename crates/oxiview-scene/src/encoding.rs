use crate::{Brush, Clip, Error, NodeId, NodeKind, Primitive, Scene};
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

enum Work {
    Visit(NodeId),
    RestoreTransform(Affine),
    PopClip,
    EndOpacity,
}

fn resolve_primitive(primitive: &Primitive) -> Primitive {
    let mut resolved = primitive.clone();
    let brush = match &mut resolved {
        Primitive::Fill(_, _, brush)
        | Primitive::Stroke(_, _, _, brush)
        | Primitive::FillPath(_, _, brush) => brush,
        Primitive::Glyph(_) | Primitive::Image(..) => return resolved,
    };
    if let Brush::LinearGradient(start, end, stops) = brush
        && start == end
        && let Some(stop) = stops
            .iter()
            .rfind(|stop| stop.position == 0.0)
            .or_else(|| stops.first())
    {
        *brush = Brush::Solid(stop.color);
    }
    resolved
}

impl Scene {
    /// Encodes the selected subtree, including every reference instance.
    ///
    /// Ancestors outside `root` do not contribute state. Later draws paint over
    /// earlier draws. Transform and clip scopes are resolved into each draw;
    /// opacity boundaries remain explicit because per-draw alpha multiplication
    /// would incorrectly composite overlapping children. Traversal is iterative.
    pub fn encode(&self, root: NodeId) -> Result<EncodedScene, Error> {
        let mut output = EncodedScene::default();
        let mut transform = Affine::IDENTITY;
        let mut clips = Vec::new();
        let mut stack = vec![Work::Visit(root)];
        while let Some(work) = stack.pop() {
            let id = match work {
                Work::Visit(id) => id,
                Work::RestoreTransform(previous) => {
                    transform = previous;
                    continue;
                }
                Work::PopClip => {
                    clips.pop();
                    continue;
                }
                Work::EndOpacity => {
                    output.nodes.push(EncodedNode::EndOpacity);
                    continue;
                }
            };
            let node = self.node(id)?;
            match &node.kind {
                NodeKind::Draw(primitive) => output.nodes.push(EncodedNode::Draw {
                    primitive: resolve_primitive(primitive),
                    transform,
                    clips: clips.clone(),
                }),
                NodeKind::Transform(local) | NodeKind::Reference(_, local) => {
                    stack.push(Work::RestoreTransform(transform));
                    transform = local.then(transform);
                }
                NodeKind::Clip(shape) => {
                    clips.push(ResolvedClip {
                        shape: shape.clone(),
                        transform,
                    });
                    stack.push(Work::PopClip);
                }
                NodeKind::Opacity(opacity) => {
                    output.nodes.push(EncodedNode::BeginOpacity(*opacity));
                    stack.push(Work::EndOpacity);
                }
            }
            stack.extend(node.children.iter().rev().copied().map(Work::Visit));
            if let NodeKind::Reference(target, _) = node.kind {
                stack.push(Work::Visit(target));
            }
        }
        Ok(output)
    }
}
