//! Generated edit equivalence against independent final construction.

use oxiview_base::{Affine, Color, CornerRadii, Point, Rect};
use oxiview_scene::*;
use proptest::prelude::*;

fn draw(value: u8) -> NodeKind {
    NodeKind::Draw(Primitive::Fill(
        Rect::new(0.0, 0.0, 10.0, 20.0),
        CornerRadii::uniform(2.0),
        Brush::Solid(Color::from_rgba8(value, 0, 0, 255)),
    ))
}
fn group() -> NodeKind {
    NodeKind::Transform(Affine::IDENTITY)
}
fn gradient(colors: [u8; 2]) -> NodeKind {
    NodeKind::Draw(Primitive::Fill(
        Rect::new(0.0, 0.0, 10.0, 20.0),
        CornerRadii::uniform(2.0),
        Brush::LinearGradient(
            Point::new(0.0, 0.0),
            Point::new(10.0, 20.0),
            vec![
                ColorStop {
                    position: 0.0,
                    color: Color::from_rgba8(colors[0], 0, 0, 255),
                },
                ColorStop {
                    position: 1.0,
                    color: Color::from_rgba8(0, colors[1], 0, 255),
                },
            ],
        ),
    ))
}

/// Final-state values only: no scene identities or edit history.
struct FinalState {
    transform: Affine,
    clip: Clip,
    opacity: f64,
    colors: [u8; 2],
    placements: [Affine; 2],
    left: Vec<u8>,
    right: Vec<u8>,
}

fn build_one_pass(state: &FinalState) -> Result<(Scene, NodeId), Error> {
    let mut scene = Scene::new();
    let definition = scene.insert(None, 0, NodeKind::Transform(state.transform))?;
    let clip = scene.insert(Some(definition), 0, NodeKind::Clip(state.clip.clone()))?;
    let opacity = scene.insert(Some(clip), 0, NodeKind::Opacity(state.opacity))?;
    scene.insert(Some(opacity), 0, gradient(state.colors))?;
    scene.insert(Some(opacity), 1, draw(state.colors[1]))?;
    let root = scene.insert(None, 1, group())?;
    for (index, values) in [&state.left, &state.right].into_iter().enumerate() {
        let parent = scene.insert(Some(root), index, group())?;
        scene.insert(
            Some(parent),
            0,
            NodeKind::Reference(definition, state.placements[index]),
        )?;
        for (i, value) in values.iter().enumerate() {
            scene.insert(Some(parent), i + 1, draw(*value))?;
        }
    }
    Ok((scene, root))
}

proptest! {
    // Generated operations must succeed; unwraps identify violated edit contracts.
    #[test]
    #[allow(clippy::unwrap_used)]
    fn edits_equal_one_pass(
        values in prop::collection::vec((any::<u8>(), any::<u8>(), any::<bool>(), any::<bool>()), 0..60),
        scale in 2u8..8,
        offset in 1i16..200,
        opacity in any::<u8>(),
        radius in 1u8..10,
        rounded in any::<bool>(),
        colors in any::<[u8; 2]>(),
    ) {
        let rect = Rect::new(0.0, 0.0, 10.0, 20.0);
        let mut final_state = FinalState {
            transform: Affine::new([f64::from(scale), 0.0, 0.5, 2.0, 0.0, 0.0]),
            clip: if rounded { Clip::RoundedRect(rect, CornerRadii::uniform(f64::from(radius))) } else { Clip::Rect(rect) },
            opacity: f64::from(opacity) / 255.0,
            colors,
            placements: [
                Affine::new([1.0, 0.0, 0.0, 1.0, f64::from(offset), 0.0]),
                Affine::new([1.0, 0.0, 0.0, 1.0, -f64::from(offset), 5.0]),
            ],
            left: Vec::new(),
            right: Vec::new(),
        };
        let mut edited = Scene::new();
        let definition = edited.insert(None, 0, NodeKind::Transform(final_state.transform)).unwrap();
        let clip = edited.insert(Some(definition), 0, NodeKind::Clip(Clip::Rect(rect))).unwrap();
        let scope = edited.insert(Some(clip), 0, NodeKind::Opacity(1.0)).unwrap();
        let paint = edited.insert(Some(scope), 0, draw(0)).unwrap();
        // Temporary descendants exercise removal of a whole ownership subtree.
        let temporary = edited.insert(Some(scope), 1, group()).unwrap();
        edited.insert(Some(temporary), 0, draw(0)).unwrap();
        let root = edited.insert(None, 1, group()).unwrap();
        let left = edited.insert(Some(root), 0, group()).unwrap();
        let right = edited.insert(Some(root), 1, group()).unwrap();
        edited.insert(Some(left), 0, NodeKind::Reference(definition, final_state.placements[0])).unwrap();
        let instance = edited.insert(Some(left), 1, NodeKind::Reference(definition, Affine::IDENTITY)).unwrap();
        edited.replace(instance, NodeKind::Reference(definition, final_state.placements[1])).unwrap();
        edited.reparent(instance, Some(right), 0).unwrap();

        // Mutate the shared definition after both instances already exist.
        edited.replace(clip, NodeKind::Clip(final_state.clip.clone())).unwrap();
        edited.replace(scope, NodeKind::Opacity(final_state.opacity)).unwrap();
        edited.replace(paint, gradient(colors)).unwrap();
        edited.remove(temporary).unwrap();
        let sibling = edited.insert(Some(root), 2, draw(colors[1])).unwrap();
        edited.reparent(sibling, Some(scope), 1).unwrap();
        for (initial, final_value, keep, move_right) in values {
            let id = edited.insert(Some(left), final_state.left.len() + 1, draw(initial)).unwrap();
            edited.replace(id, draw(final_value)).unwrap();
            if !keep { edited.remove(id).unwrap(); }
            else if move_right {
                edited.reparent(id, Some(right), 1).unwrap();
                final_state.right.insert(0, final_value);
            } else { final_state.left.push(final_value); }
        }
        // This builder receives only final values, never reads the edited scene,
        // and inserts each final node once, without replaying replacements/moves.
        let (direct, direct_root) = build_one_pass(&final_state).unwrap();
        prop_assert_eq!(&edited, &direct);
        prop_assert_eq!(edited.encode(root).unwrap(), direct.encode(direct_root).unwrap());
    }
}
