//! Public geometry and invalidation fixtures for M2 layout.
#![allow(clippy::unwrap_used)]

use oxiview_base::{Point, Rect, Size};
use oxiview_core::{NodeId, UiTree};
use oxiview_layout::TaffyLayout;
use oxiview_style::{
    Display,
    Length::{Auto, Percent, Points},
    Overflow, Position, Style,
};

fn block(width: oxiview_style::Length, height: oxiview_style::Length) -> Style {
    let mut style = Style::default();
    style.layout.display = Display::Block;
    style.layout.width = width;
    style.layout.height = height;
    style
}

fn absolute_fixture() -> (UiTree, [NodeId; 4]) {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let mut root_style = block(Points(160.0), Points(100.0));
    root_style.layout.margin.left = Points(13.0);
    root_style.layout.margin.top = Points(7.0);
    root_style.layout.padding.left = Points(6.0);
    root_style.layout.padding.top = Points(8.0);
    root_style.layout.border_width = 4.0;
    tree.set_style(root, root_style).unwrap();

    let mut wrapper = block(Points(100.0), Points(60.0));
    wrapper.layout.display = Display::Grid;
    wrapper.layout.margin.left = Points(20.0);
    wrapper.layout.border_width = 2.0;
    let wrapper = tree.append(root, wrapper).unwrap();

    let mut absolute = block(Points(20.0), Points(10.0));
    absolute.layout.position = Position::Absolute;
    absolute.layout.inset.right = Percent(0.25);
    absolute.layout.inset.top = Points(-10.0);
    let absolute = tree.append(wrapper, absolute).unwrap();

    let mut nested = block(Points(4.0), Points(3.0));
    nested.layout.position = Position::Absolute;
    nested.layout.inset.right = Points(2.0);
    nested.layout.inset.bottom = Points(1.0);
    let nested = tree.append(absolute, nested).unwrap();
    (tree, [root, wrapper, absolute, nested])
}

fn bounds(tree: &UiTree, nodes: [NodeId; 4]) -> [Rect; 4] {
    nodes.map(|node| tree.bounds(node).unwrap())
}

#[test]
fn hoisted_boxes_follow_the_viewport_and_nearest_positioned_ancestor() {
    let viewport = Size::new(320.0, 180.0);
    let (mut tree, nodes) = absolute_fixture();
    let (mut fresh, fresh_nodes) = absolute_fixture();
    let [_, wrapper, absolute, nested] = nodes;

    tree.layout(viewport).unwrap();
    assert_eq!(
        tree.bounds(wrapper).unwrap().origin(),
        Point::new(43.0, 19.0)
    );
    assert_eq!(
        tree.bounds(absolute).unwrap(),
        Rect::new(220.0, -10.0, 240.0, 0.0)
    );
    assert_eq!(
        tree.bounds(nested).unwrap(),
        Rect::new(234.0, -4.0, 238.0, -1.0)
    );

    let before = tree.diagnostics().layout_node_visits;
    for (tree, [_, _, _, nested]) in [(&mut tree, nodes), (&mut fresh, fresh_nodes)] {
        let mut style = tree.style(nested).unwrap().clone();
        style.layout.inset.right = Points(5.0);
        tree.set_style(nested, style).unwrap();
    }
    tree.layout(viewport).unwrap();
    fresh
        .layout_with(&mut TaffyLayout::new(), viewport)
        .unwrap();
    assert_eq!(tree.diagnostics().layout_node_visits - before, 2);
    assert_eq!(
        tree.bounds(nested).unwrap().origin(),
        Point::new(231.0, -4.0)
    );
    assert_eq!(bounds(&tree, nodes), bounds(&fresh, fresh_nodes));

    for (tree, [_, wrapper, _, _]) in [(&mut tree, nodes), (&mut fresh, fresh_nodes)] {
        let mut style = tree.style(wrapper).unwrap().clone();
        style.layout.margin.left = Points(30.0);
        tree.set_style(wrapper, style).unwrap();
    }
    let resized = Size::new(400.0, 220.0);
    tree.layout(resized).unwrap();
    fresh.layout_with(&mut TaffyLayout::new(), resized).unwrap();
    assert_eq!(
        tree.bounds(wrapper).unwrap().origin(),
        Point::new(53.0, 19.0)
    );
    assert_eq!(
        tree.bounds(absolute).unwrap().origin(),
        Point::new(280.0, -10.0)
    );
    assert_eq!(bounds(&tree, nodes), bounds(&fresh, fresh_nodes));

    for (tree, [_, wrapper, _, _]) in [(&mut tree, nodes), (&mut fresh, fresh_nodes)] {
        let mut style = tree.style(wrapper).unwrap().clone();
        style.layout.position = Position::Relative;
        tree.set_style(wrapper, style).unwrap();
    }
    tree.layout(resized).unwrap();
    fresh.layout_with(&mut TaffyLayout::new(), resized).unwrap();
    assert_eq!(
        tree.bounds(absolute).unwrap().origin(),
        Point::new(107.0, 11.0)
    );
    assert_eq!(
        tree.bounds(nested).unwrap().origin(),
        Point::new(118.0, 17.0)
    );
    assert_eq!(bounds(&tree, nodes), bounds(&fresh, fresh_nodes));
}

#[test]
fn overflow_clip_preserves_margin_collapse_through_a_fixed_height_block() {
    let viewport = Size::new(200.0, 180.0);
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    tree.set_style(root, block(Auto, Auto)).unwrap();
    let wrapper = tree.append(root, block(Auto, Points(40.0))).unwrap();
    let marker = tree
        .append(root, block(Points(10.0), Points(10.0)))
        .unwrap();
    let mut child_style = block(Points(50.0), Points(20.0));
    child_style.layout.margin.top = Points(24.0);
    let child = tree.append(wrapper, child_style.clone()).unwrap();
    tree.layout(viewport).unwrap();
    assert_eq!(tree.bounds(wrapper).unwrap().origin().y(), 24.0);
    assert_eq!(tree.bounds(child).unwrap().origin().y(), 24.0);
    assert_eq!(tree.bounds(marker).unwrap().origin().y(), 64.0);

    let before = tree.diagnostics();
    let mut clipped = tree.style(wrapper).unwrap().clone();
    clipped.overflow = Overflow::Clip;
    tree.set_style(wrapper, clipped).unwrap();
    tree.layout(viewport).unwrap();
    assert_eq!(tree.diagnostics(), before);

    child_style.layout.margin.top = Points(18.0);
    tree.set_style(child, child_style).unwrap();
    tree.layout(viewport).unwrap();
    assert_eq!(tree.bounds(wrapper).unwrap().origin().y(), 18.0);
    assert_eq!(tree.bounds(wrapper).unwrap().size().height(), 40.0);
    assert_eq!(tree.bounds(child).unwrap().origin().y(), 18.0);
    assert_eq!(tree.bounds(marker).unwrap().origin().y(), 58.0);
    let nodes = [root, wrapper, child, marker];
    let incremental = bounds(&tree, nodes);
    tree.layout_with(&mut TaffyLayout::new(), viewport).unwrap();
    assert_eq!(incremental, bounds(&tree, nodes));
}
