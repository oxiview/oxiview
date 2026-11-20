// The fixtures use assertions and unwraps to identify the failing contract.
#![allow(clippy::unwrap_used)]

use super::*;
use oxiview_base::{Color, CornerRadii};
use oxiview_layout::{LayoutError, TaffyLayout};
use oxiview_style::{Length, Overflow, Position};
use std::cell::Cell;

const VIEWPORT: Size = Size::new(320.0, 480.0);

fn block(width: Length, height: Length) -> Style {
    let mut style = Style::default();
    style.layout.display = Display::Block;
    style.layout.width = width;
    style.layout.height = height;
    style
}

fn block_tree() -> UiTree {
    let mut tree = UiTree::new().unwrap();
    tree.set_style(tree.root(), block(Length::Auto, Length::Auto))
        .unwrap();
    tree
}

fn assert_fresh(tree: &UiTree) {
    let mut expected = HashMap::new();
    TaffyLayout::new()
        .compute(
            &mut TreeLayout {
                tree,
                output: &mut expected,
                viewport: tree.viewport,
            },
            tree.root(),
            tree.viewport,
        )
        .unwrap();
    assert_eq!(expected.len(), tree.len());
    for (id, layout) in expected {
        assert_eq!(tree.node(id).unwrap().layout, layout, "{id:?}");
    }
}

#[test]
fn transient_children_do_not_accumulate_removals_between_layout_passes() {
    let mut tree = block_tree();
    let root = tree.root();
    let retained = tree.append(root, Style::default()).unwrap();
    tree.layout(VIEWPORT).unwrap();
    tree.remove(retained).unwrap();
    for _ in 0..100 {
        let temporary = tree.append(root, Style::default()).unwrap();
        tree.remove(temporary).unwrap();
    }
    assert_eq!(tree.layout_changes.len(), 2);
    assert_eq!(
        tree.layout_changes.get(&retained),
        Some(&LayoutChange::Remove)
    );
    tree.layout(VIEWPORT).unwrap();
    assert_eq!(tree.engine.cached_node_count(), 1);
    assert_fresh(&tree);
}

#[test]
fn coalesced_style_edit_skips_the_clean_branches_descendants() {
    let mut tree = block_tree();
    let root = tree.root();
    let edited = tree
        .append(root, block(Length::Auto, Length::Auto))
        .unwrap();
    let leaf = tree
        .append(edited, block(Length::Points(30.0), Length::Points(10.0)))
        .unwrap();
    let clean = tree
        .append(root, block(Length::Auto, Length::Auto))
        .unwrap();
    for _ in 0..200 {
        tree.append(clean, block(Length::Percent(0.5), Length::Points(2.0)))
            .unwrap();
    }
    tree.layout(VIEWPORT).unwrap();
    let before = tree.diagnostics();
    assert_eq!(before.layout_input_visits, 204);
    assert_eq!(before.resolved_styles, 204);

    for height in [12.0, 23.0, 42.0] {
        let mut style = tree.style(leaf).unwrap().clone();
        style.layout.height = Length::Points(height);
        assert!(tree.set_style(leaf, style).unwrap());
    }
    assert_eq!(tree.layout_changes.len(), 1);
    tree.layout(VIEWPORT).unwrap();
    let after = tree.diagnostics();
    let visits = after.layout_node_visits - before.layout_node_visits;
    assert_eq!(after.layout_passes - before.layout_passes, 1);
    assert_eq!(after.layout_input_visits - before.layout_input_visits, 1);
    assert_eq!(after.resolved_styles - before.resolved_styles, 1);
    assert!(
        visits <= 8,
        "layout visited {visits} nodes after a local edit"
    );
    assert_eq!(tree.bounds(edited).unwrap().size().height(), 42.0);
    assert_eq!(tree.bounds(clean).unwrap().edges()[1], 42.0);
    assert_fresh(&tree);
    eprintln!("204-node local edit: 1 input visit, 1 style projection, {visits} layout visits");
}

#[test]
fn an_already_hidden_boundary_keeps_descendant_edits_internal() {
    let mut tree = block_tree();
    let root = tree.root();
    let mut hidden_style = block(Length::Auto, Length::Auto);
    hidden_style.layout.display = Display::None;
    let hidden = tree.append(root, hidden_style).unwrap();
    let leaf = tree
        .append(hidden, block(Length::Auto, Length::Auto))
        .unwrap();
    let clean = tree
        .append(root, block(Length::Auto, Length::Auto))
        .unwrap();
    for _ in 0..32 {
        tree.append(clean, block(Length::Auto, Length::Points(2.0)))
            .unwrap();
    }
    tree.layout(VIEWPORT).unwrap();
    let before = tree.diagnostics();

    tree.set_intrinsic_size(leaf, Size::new(20.0, 31.0))
        .unwrap();
    tree.layout(VIEWPORT).unwrap();
    let after = tree.diagnostics();
    assert_eq!(after.layout_input_visits - before.layout_input_visits, 1);
    assert_eq!(after.resolved_styles - before.resolved_styles, 0);
    assert_eq!(after.layout_node_visits - before.layout_node_visits, 2);
    assert_eq!(after.measured_nodes, before.measured_nodes);
    assert_eq!(tree.node(leaf).unwrap().layout, LayoutBox::default());
    assert_fresh(&tree);

    tree.set_style(hidden, block(Length::Auto, Length::Auto))
        .unwrap();
    tree.layout(VIEWPORT).unwrap();
    assert_eq!(tree.bounds(leaf).unwrap().size().height(), 31.0);
    assert_eq!(tree.bounds(clean).unwrap().edges()[1], 31.0);
    assert_fresh(&tree);
}

#[test]
fn failed_pass_preserves_geometry_and_retries_the_same_journal() {
    let mut tree = block_tree();
    let root = tree.root();
    let from = tree
        .append(root, block(Length::Points(200.0), Length::Auto))
        .unwrap();
    let to = tree
        .append(root, block(Length::Points(90.0), Length::Auto))
        .unwrap();
    let moving = tree
        .append(from, block(Length::Percent(0.5), Length::Points(20.0)))
        .unwrap();
    let removed = tree
        .append(from, block(Length::Auto, Length::Points(5.0)))
        .unwrap();
    let measured = tree.append(to, block(Length::Auto, Length::Auto)).unwrap();
    let failing = Rc::new(Cell::new(false));
    let measure_state = Rc::clone(&failing);
    tree.set_measure(measured, move |_, _| Dimensions {
        width: 30.0,
        height: if measure_state.get() { f64::NAN } else { 19.0 },
    })
    .unwrap();
    tree.layout(VIEWPORT).unwrap();
    let before = tree.diagnostics();
    let retained = tree.engine.cached_node_count();
    let old_geometry: Vec<_> = [root, from, to, moving, measured]
        .into_iter()
        .map(|id| (id, tree.node(id).unwrap().layout))
        .collect();

    tree.reparent(moving, to, 0).unwrap();
    tree.remove(removed).unwrap();
    let replacement = tree
        .append(from, block(Length::Percent(0.75), Length::Points(12.0)))
        .unwrap();
    assert_eq!(replacement.slot, removed.slot);
    assert_ne!(replacement, removed);
    tree.invalidate_measure(measured).unwrap();
    let pending = tree.layout_changes.clone();
    failing.set(true);
    let resized = Size::new(400.0, 520.0);
    assert!(matches!(tree.layout(resized), Err(UiError::Layout(_))));
    assert_eq!(tree.diagnostics(), before);
    assert_eq!(tree.viewport, VIEWPORT);
    assert_eq!(tree.layout_changes, pending);
    assert_eq!(tree.engine.cached_node_count(), retained);
    for (id, layout) in old_geometry {
        assert_eq!(tree.node(id).unwrap().layout, layout);
    }
    assert_eq!(tree.node(replacement).unwrap().layout, LayoutBox::default());

    failing.set(false);
    tree.layout(resized).unwrap();
    assert!(tree.layout_changes.is_empty());
    assert_eq!(tree.bounds(root).unwrap().size(), resized);
    assert_eq!(tree.engine.cached_node_count(), tree.len());
    assert_eq!(tree.bounds(moving).unwrap().size().width(), 45.0);
    assert_eq!(tree.bounds(replacement).unwrap().size().width(), 150.0);
    assert_fresh(&tree);

    tree.clear(from).unwrap();
    tree.layout(resized).unwrap();
    assert_eq!(tree.engine.cached_node_count(), retained - 1);
    assert_fresh(&tree);
}

#[test]
fn absolute_candidates_follow_topology_and_viewport_through_cached_branches() {
    let mut tree = block_tree();
    let root = tree.root();
    tree.set_style(root, block(Length::Points(250.0), Length::Auto))
        .unwrap();
    let from = tree
        .append(root, block(Length::Percent(0.8), Length::Points(60.0)))
        .unwrap();
    let mut positioned = block(Length::Points(120.0), Length::Points(80.0));
    positioned.layout.position = Position::Relative;
    let to = tree.append(root, positioned).unwrap();
    let mut translated = block(Length::Percent(0.5), Length::Points(20.0));
    translated.layout.margin.left = Length::Points(25.0);
    let carrier = tree.append(from, translated).unwrap();
    let mut style = block(Length::Percent(0.5), Length::Points(10.0));
    style.layout.position = Position::Absolute;
    style.layout.inset.left = Length::Percent(0.25);
    style.layout.inset.top = Length::Points(-4.0);
    let child = tree.append(carrier, style).unwrap();
    tree.layout(VIEWPORT).unwrap();
    assert_eq!(
        tree.bounds(child).unwrap(),
        Rect::new(80.0, -4.0, 240.0, 6.0)
    );
    assert_fresh(&tree);

    let before = tree.diagnostics();
    tree.paint(VIEWPORT, 2.0).unwrap();
    tree.paint(VIEWPORT, 1.5).unwrap();
    assert_eq!(tree.diagnostics().layout_passes, before.layout_passes);

    tree.layout(Size::new(400.0, 480.0)).unwrap();
    assert_eq!(
        tree.bounds(child).unwrap(),
        Rect::new(100.0, -4.0, 300.0, 6.0)
    );
    assert_fresh(&tree);
    tree.reparent(carrier, to, 0).unwrap();
    tree.layout(VIEWPORT).unwrap();
    assert_eq!(
        tree.bounds(child).unwrap(),
        Rect::new(30.0, 56.0, 90.0, 66.0)
    );
    assert_fresh(&tree);

    tree.reparent(to, root, 0).unwrap();
    tree.layout(VIEWPORT).unwrap();
    assert_eq!(
        tree.bounds(child).unwrap(),
        Rect::new(30.0, -4.0, 90.0, 6.0)
    );
    assert_fresh(&tree);

    let mut hidden_style = tree.style(to).unwrap().clone();
    hidden_style.layout.display = Display::None;
    tree.set_style(to, hidden_style).unwrap();
    tree.layout(VIEWPORT).unwrap();
    assert_eq!(tree.node(child).unwrap().layout, LayoutBox::default());
    assert_fresh(&tree);
    tree.reparent(carrier, from, 0).unwrap();
    tree.layout(VIEWPORT).unwrap();
    assert_eq!(
        tree.bounds(child).unwrap(),
        Rect::new(80.0, -4.0, 240.0, 6.0)
    );
    assert_fresh(&tree);
}

#[test]
fn only_layout_properties_enter_the_mutation_journal() {
    type StyleCase = (fn(&mut Style), bool, bool);
    let cases: [StyleCase; 12] = [
        (|_| {}, false, false),
        (
            |s| s.background = Color::from_rgba8(255, 0, 0, 255),
            true,
            false,
        ),
        (
            |s| s.border_color = Color::from_rgba8(255, 0, 0, 255),
            true,
            false,
        ),
        (|s| s.border_radii = CornerRadii::uniform(4.0), true, false),
        (|s| s.opacity = 0.5, true, false),
        (|s| s.z_index = Some(1), true, false),
        (|s| s.overflow = Overflow::Clip, true, false),
        (|s| s.hit_test = false, false, false),
        (|s| s.stationary_hover_updates = true, false, false),
        (|s| s.layout.border_width = 3.0, true, true),
        (|s| s.layout.width = Length::Points(40.0), true, true),
        (|s| s.layout.display = Display::None, true, true),
    ];
    for (edit, paints, lays_out) in cases {
        let mut tree = block_tree();
        let id = tree.append(tree.root(), Style::default()).unwrap();
        tree.paint(VIEWPORT, 1.0).unwrap();
        let before = tree.diagnostics();
        let mut style = tree.style(id).unwrap().clone();
        edit(&mut style);
        assert_eq!(tree.set_style(id, style.clone()).unwrap(), paints);
        assert_eq!(tree.layout_changes.contains_key(&id), lays_out);

        let pending = tree.layout_changes.clone();
        let mut invalid = style.clone();
        invalid.layout.border_width = f64::NAN;
        assert!(matches!(
            tree.set_style(id, invalid),
            Err(UiError::Style(_))
        ));
        assert_eq!(tree.style(id).unwrap(), &style);
        assert_eq!(tree.layout_changes, pending);

        tree.layout(VIEWPORT).unwrap();
        let after = tree.diagnostics();
        assert_eq!(
            after.layout_passes - before.layout_passes,
            u64::from(lays_out)
        );
        assert_eq!(
            after.layout_input_visits - before.layout_input_visits,
            u64::from(lays_out)
        );
        assert_eq!(
            after.resolved_styles - before.resolved_styles,
            u64::from(lays_out)
        );
    }
}

struct FixedGeometry;

impl LayoutEngine<NodeId> for FixedGeometry {
    fn compute<T: LayoutTree<NodeId = NodeId>>(
        &mut self,
        tree: &mut T,
        root: NodeId,
        _viewport: Size,
    ) -> Result<LayoutStats, LayoutError> {
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            pending.extend_from_slice(tree.children(id));
            tree.set_layout(
                id,
                LayoutBox {
                    size: Size::new(55.0, 55.0),
                    ..LayoutBox::default()
                },
            );
        }
        Ok(LayoutStats::default())
    }
}

#[test]
fn custom_geometry_is_replaced_on_the_next_dirty_default_pass() {
    let mut tree = block_tree();
    let fixed = block(Length::Points(20.0), Length::Points(10.0));
    let changed = tree.append(tree.root(), fixed.clone()).unwrap();
    let unaffected = tree.append(tree.root(), fixed).unwrap();
    tree.layout(VIEWPORT).unwrap();
    tree.layout_with(&mut FixedGeometry, VIEWPORT).unwrap();
    tree.layout(VIEWPORT).unwrap();
    assert_eq!(
        tree.bounds(unaffected).unwrap().size(),
        Size::new(55.0, 55.0)
    );

    let mut style = tree.style(changed).unwrap().clone();
    style.layout.height = Length::Points(30.0);
    tree.set_style(changed, style).unwrap();
    tree.layout(VIEWPORT).unwrap();
    assert_eq!(
        tree.bounds(unaffected).unwrap().size(),
        Size::new(20.0, 10.0)
    );
    assert_fresh(&tree);
}

#[test]
fn queued_visibility_suppresses_the_root_and_restores_current_geometry() {
    let mut tree = block_tree();
    let root = tree.root();
    let leaf = tree
        .append(root, block(Length::Percent(0.5), Length::Points(12.0)))
        .unwrap();
    let mut style = tree.style(root).unwrap().clone();
    style.layout.margin.top = Length::Points(5.0);
    style.layout.padding.left = Length::Points(8.0);
    style.layout.border_width = 3.0;
    style.layout.position = Position::Relative;
    tree.set_style(root, style).unwrap();
    tree.layout(VIEWPORT).unwrap();

    let sender = tree.command_sender();
    sender
        .mutate(
            root,
            Box::new(move |tree, handler, work| {
                tree.apply(input::Command::Visible(root, false), handler, work)
            }),
        )
        .unwrap();
    tree.flush_updates(VIEWPORT, |_, _, _| {}).unwrap();
    for id in [root, leaf] {
        assert_eq!(tree.node(id).unwrap().layout, LayoutBox::default());
    }
    assert_fresh(&tree);

    let resized = Size::new(400.0, 480.0);
    sender
        .mutate(
            root,
            Box::new(move |tree, handler, work| {
                tree.apply(input::Command::Visible(root, true), handler, work)
            }),
        )
        .unwrap();
    tree.flush_updates(resized, |_, _, _| {}).unwrap();
    assert_eq!(tree.bounds(root).unwrap().edges()[1], 5.0);
    assert_eq!(tree.bounds(leaf).unwrap().size().width(), 193.0);
    assert_fresh(&tree);
}

#[test]
fn generated_incremental_edits_match_fresh_boxes() {
    let mut tree = block_tree();
    let root = tree.root();
    let fixed = block(Length::Auto, Length::Points(100.0));
    let a = tree.append(root, fixed.clone()).unwrap();
    let b = tree.append(root, fixed).unwrap();
    let anchor = tree
        .append(a, block(Length::Auto, Length::Percent(0.5)))
        .unwrap();
    let mut moving = tree.append(b, Style::default()).unwrap();
    let mut seed = 17u64;
    let mut viewport = VIEWPORT;
    tree.layout(viewport).unwrap();

    for step in 0..96 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let change = 10.0 + ((seed >> 32) % 61) as f64;
        match step % 8 {
            0 => tree
                .set_intrinsic_size(moving, Size::new(change, change))
                .unwrap(),
            1 => {
                let mut style = tree.style(a).unwrap().clone();
                style.layout.height = Length::Points(change + 30.0);
                tree.set_style(a, style).unwrap();
            }
            2 => {
                let parent = if tree.parent(moving).unwrap() == Some(a) {
                    b
                } else {
                    a
                };
                tree.reparent(moving, parent, 0).unwrap();
            }
            3 => {
                let mut style = tree.style(moving).unwrap().clone();
                style.layout.position = Position::Absolute;
                style.layout.inset.left = Length::Percent(0.25);
                tree.set_style(moving, style).unwrap();
            }
            4 => {
                let mut style = tree.style(b).unwrap().clone();
                style.layout.display = if style.layout.display == Display::Block {
                    Display::None
                } else {
                    Display::Block
                };
                tree.set_style(b, style).unwrap();
            }
            5 => viewport = Size::new(200.0 + change, 400.0 + change),
            6 => tree.reparent(anchor, a, 0).unwrap(),
            _ => {
                let parent = tree.parent(moving).unwrap().unwrap();
                tree.remove(moving).unwrap();
                moving = tree.insert(parent, 0, Style::default()).unwrap();
            }
        }
        tree.layout(viewport).unwrap();
        assert_fresh(&tree);
    }
}
