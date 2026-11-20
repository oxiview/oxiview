use super::*;
use oxiview_base::{Point, Size};
use std::collections::HashMap;

#[derive(Clone)]
struct Node {
    style: LayoutInput,
    children: Vec<usize>,
    layout: LayoutBox,
    intrinsic: Dimensions<f64>,
    revision: u64,
    measure_calls: usize,
    wrap: bool,
}
#[derive(Clone)]
struct Tree {
    nodes: Vec<Node>,
    measures: usize,
    published: Vec<usize>,
}
impl Tree {
    fn new(styles: Vec<LayoutInput>, edges: Vec<Vec<usize>>) -> Self {
        let nodes = styles
            .into_iter()
            .zip(edges)
            .map(|(style, children)| Node {
                style,
                children,
                layout: LayoutBox::default(),
                intrinsic: Dimensions::default(),
                revision: 1,
                measure_calls: 0,
                wrap: false,
            })
            .collect();
        Self {
            nodes,
            measures: 0,
            published: Vec::new(),
        }
    }
    fn geometry(&self, id: usize) -> LayoutBox {
        self.nodes
            .get(id)
            .map_or(LayoutBox::default(), |node| node.layout)
    }
}
impl LayoutTree for Tree {
    type NodeId = usize;
    fn children(&self, id: usize) -> &[usize] {
        self.nodes
            .get(id)
            .map_or(&[], |node| node.children.as_slice())
    }
    fn layout_input(&self, id: usize) -> LayoutInput {
        self.nodes
            .get(id)
            .map_or_else(LayoutInput::default, |node| node.style.clone())
    }
    fn set_layout(&mut self, id: usize, layout: LayoutBox) {
        self.published.push(id);
        if let Some(node) = self.nodes.get_mut(id) {
            node.layout = layout;
        }
    }
    fn contains_node(&self, id: usize) -> bool {
        id < self.nodes.len()
    }
    fn content_revision(&self, id: usize) -> u64 {
        self.nodes.get(id).map_or(0, |node| node.revision)
    }
    fn measure(
        &mut self,
        id: usize,
        known: Dimensions<Option<f64>>,
        available: Dimensions<AvailableSpace>,
    ) -> Dimensions<f64> {
        self.measures += 1;
        let Some(node) = self.nodes.get_mut(id) else {
            return Dimensions::default();
        };
        node.measure_calls += 1;
        let intrinsic = node.intrinsic;
        if node.wrap {
            let width = known.width.unwrap_or_else(|| match available.width {
                AvailableSpace::Definite(width) => intrinsic.width.min(width),
                AvailableSpace::MinContent => 10.0,
                AvailableSpace::MaxContent => intrinsic.width,
            });
            return Dimensions {
                width,
                height: known
                    .height
                    .unwrap_or(intrinsic.height * (intrinsic.width / width.max(1.0)).ceil()),
            };
        }
        Dimensions {
            width: known.width.unwrap_or(intrinsic.width),
            height: known.height.unwrap_or(intrinsic.height),
        }
    }
}
fn fixed(w: f64, h: f64) -> LayoutInput {
    LayoutInput {
        width: Length::Points(w),
        height: Length::Points(h),
        ..LayoutInput::default()
    }
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
}

fn branch_fixture() -> Tree {
    let block = |width, height| LayoutInput {
        display: Display::Block,
        ..fixed(width, height)
    };
    let mut tree = Tree::new(
        vec![
            block(300.0, 300.0),
            block(120.0, 100.0),
            LayoutInput::default(),
            block(120.0, 100.0),
            LayoutInput::default(),
        ],
        vec![vec![1, 3], vec![2], vec![], vec![4], vec![]],
    );
    tree.nodes[2].intrinsic = Dimensions {
        width: 40.0,
        height: 18.0,
    };
    tree.nodes[4].intrinsic = Dimensions {
        width: 60.0,
        height: 22.0,
    };
    tree
}
fn assert_fresh_equivalent(tree: &Tree, viewport: Size) -> Result<(), LayoutError> {
    let mut fresh = tree.clone();
    TaffyLayout::new().compute(&mut fresh, 0, viewport)?;
    let mut stack = vec![0];
    while let Some(id) = stack.pop() {
        let a = tree.geometry(id);
        let b = fresh.geometry(id);
        let values = |layout: LayoutBox| {
            [
                layout.location.x(),
                layout.location.y(),
                layout.size.width(),
                layout.size.height(),
                layout.content_size.width(),
                layout.content_size.height(),
                layout.padding.left,
                layout.padding.right,
                layout.padding.top,
                layout.padding.bottom,
                layout.border.left,
                layout.border.right,
                layout.border.top,
                layout.border.bottom,
            ]
        };
        for (actual, expected) in values(a).into_iter().zip(values(b)) {
            near(actual, expected);
        }
        stack.extend(tree.nodes[id].children.iter().copied());
    }
    Ok(())
}

#[test]
fn persistent_cache_reuses_unchanged_branches_and_measurement_revisions() -> Result<(), LayoutError>
{
    for display in [Display::Flex, Display::Grid, Display::Block] {
        let mut tree = branch_fixture();
        tree.nodes[0].style.display = display;
        tree.nodes[0].style.grid_template_columns =
            vec![TrackSize::Points(150.0), TrackSize::Points(150.0)];
        let mut engine = TaffyLayout::new();
        let viewport = Size::new(300.0, 300.0);
        engine.compute(&mut tree, 0, viewport)?;
        assert!(tree.nodes[2].measure_calls > 0 && tree.nodes[4].measure_calls > 0);
        let first = tree.nodes[2].measure_calls;
        let sibling = tree.nodes[4].measure_calls;
        let clean = engine.compute(&mut tree, 0, viewport)?;
        assert_eq!(clean.measured_nodes, 0);
        assert_eq!(clean.computed_nodes, 0);
        tree.nodes[2].intrinsic.height = 39.0;
        tree.nodes[2].revision += 1;
        let changed = engine.compute(&mut tree, 0, viewport)?;
        assert!(tree.nodes[2].measure_calls > first);
        assert_eq!(tree.nodes[4].measure_calls, sibling);
        assert!(changed.cache_hits > 0);
        near(tree.geometry(2).size.height(), 39.0);
        assert_fresh_equivalent(&tree, viewport)?;
    }
    Ok(())
}

#[test]
fn cached_constraints_percentages_and_wrapping_match_fresh_layout() -> Result<(), LayoutError> {
    let mut tree = branch_fixture();
    tree.nodes[1].style.width = Length::Percent(0.5);
    tree.nodes[2].wrap = true;
    tree.nodes[2].intrinsic = Dimensions {
        width: 260.0,
        height: 14.0,
    };
    let mut engine = TaffyLayout::new();
    for width in [300.0, 480.0, 220.0, 300.0] {
        tree.nodes[0].style.width = Length::Points(width);
        let viewport = Size::new(width, 350.0);
        engine.compute(&mut tree, 0, viewport)?;
        near(tree.geometry(1).size.width(), width * 0.5);
        assert_fresh_equivalent(&tree, viewport)?;
    }
    tree.nodes[0].style.padding = Edges::all(Length::Percent(0.04));
    tree.nodes[1].style.border_width = 3.0;
    let viewport = Size::new(300.0, 350.0);
    engine.compute(&mut tree, 0, viewport)?;
    assert_fresh_equivalent(&tree, viewport)?;
    tree.nodes[0].style.display = Display::Grid;
    tree.nodes[0].style.grid_template_columns = vec![TrackSize::Fr(1.0), TrackSize::Fr(2.0)];
    engine.compute(&mut tree, 0, viewport)?;
    assert_fresh_equivalent(&tree, viewport)
}

#[test]
fn cached_absolute_candidates_survive_reorder_reparent_and_removal() -> Result<(), LayoutError> {
    let mut tree = branch_fixture();
    tree.nodes[0].style.position = Position::Relative;
    tree.nodes[2].style.position = Position::Absolute;
    tree.nodes[2].style.width = Length::Percent(0.3);
    tree.nodes[2].style.inset.left = Length::Percent(0.1);
    tree.nodes[2].style.inset.top = Length::Points(7.0);
    let viewport = Size::new(300.0, 300.0);
    let mut engine = TaffyLayout::new();
    engine.compute(&mut tree, 0, viewport)?;
    engine.compute(&mut tree, 0, viewport)?;
    assert_fresh_equivalent(&tree, viewport)?;
    tree.nodes[0].children.reverse();
    engine.compute(&mut tree, 0, viewport)?;
    assert_fresh_equivalent(&tree, viewport)?;
    tree.nodes[1].children.clear();
    tree.nodes[3].children.push(2);
    engine.compute(&mut tree, 0, viewport)?;
    assert_fresh_equivalent(&tree, viewport)?;
    tree.nodes[3].style.position = Position::Relative;
    engine.compute(&mut tree, 0, viewport)?;
    assert_fresh_equivalent(&tree, viewport)?;
    tree.nodes[0].children.retain(|id| *id != 1);
    engine.compute(&mut tree, 0, viewport)?;
    assert_eq!(engine.cached_node_count(), 4);
    assert_fresh_equivalent(&tree, viewport)?;
    tree.nodes[0].children.push(1);
    tree.nodes[1].children.clear();
    tree.nodes[1].style = LayoutInput::default();
    tree.nodes[1].intrinsic = Dimensions {
        width: 75.0,
        height: 49.0,
    };
    tree.nodes[1].revision += 1;
    engine.compute(&mut tree, 0, viewport)?;
    assert_eq!(engine.cached_node_count(), 5);
    near(tree.geometry(1).size.height(), 49.0);
    assert_fresh_equivalent(&tree, viewport)
}

#[test]
fn hidden_branch_edits_and_restoration_do_not_reuse_stale_geometry() -> Result<(), LayoutError> {
    let mut tree = branch_fixture();
    let mut engine = TaffyLayout::new();
    let viewport = Size::new(300.0, 300.0);
    engine.compute(&mut tree, 0, viewport)?;
    let sibling = tree.nodes[4].measure_calls;
    tree.nodes[1].style.display = Display::None;
    engine.compute(&mut tree, 0, viewport)?;
    let hidden = tree.nodes[2].measure_calls;
    tree.nodes[2].intrinsic.height = 67.0;
    tree.nodes[2].revision += 1;
    engine.compute(&mut tree, 0, viewport)?;
    assert_eq!(tree.nodes[2].measure_calls, hidden);
    near(tree.geometry(2).size.height(), 0.0);
    assert_fresh_equivalent(&tree, viewport)?;
    tree.nodes[1].style.display = Display::Block;
    engine.compute(&mut tree, 0, viewport)?;
    near(tree.geometry(2).size.height(), 67.0);
    assert_eq!(tree.nodes[4].measure_calls, sibling);
    assert_fresh_equivalent(&tree, viewport)
}

#[test]
fn failed_cached_pass_preserves_geometry_and_cache_for_recovery() -> Result<(), LayoutError> {
    let mut tree = branch_fixture();
    let mut engine = TaffyLayout::new();
    let viewport = Size::new(300.0, 300.0);
    engine.compute(&mut tree, 0, viewport)?;
    let before: Vec<_> = tree.nodes.iter().map(|node| node.layout).collect();
    tree.nodes[2].intrinsic.height = f64::NAN;
    tree.nodes[2].revision += 1;
    assert!(matches!(
        engine.compute(&mut tree, 0, viewport),
        Err(LayoutError::InvalidGeometry)
    ));
    assert_eq!(
        tree.nodes
            .iter()
            .map(|node| node.layout)
            .collect::<Vec<_>>(),
        before
    );
    tree.nodes[2].intrinsic.height = 31.0;
    tree.nodes[2].revision += 1;
    engine.compute(&mut tree, 0, viewport)?;
    assert_fresh_equivalent(&tree, viewport)?;
    engine.clear_cache();
    assert_eq!(engine.cached_node_count(), 0);
    let fresh = engine.compute(&mut tree, 0, viewport)?;
    assert!(fresh.measured_nodes > 0);
    assert_fresh_equivalent(&tree, viewport)
}

#[test]
fn generated_dirty_edits_remain_equivalent_to_fresh_computation() -> Result<(), LayoutError> {
    let mut tree = branch_fixture();
    let mut engine = TaffyLayout::new();
    let mut seed = 17u64;
    for step in 0..120 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        match step % 6 {
            0 => tree.nodes[0].style.width = Length::Points(200.0 + (seed % 200) as f64),
            1 => {
                tree.nodes[2].intrinsic.height = 12.0 + (seed % 60) as f64;
                tree.nodes[2].revision += 1;
            }
            2 => tree.nodes[0].children.reverse(),
            3 => tree.nodes[1].style.padding = Edges::all(Length::Points((seed % 9) as f64)),
            4 => {
                tree.nodes[3].style.position = if seed.is_multiple_of(2) {
                    Position::Relative
                } else {
                    Position::Static
                }
            }
            _ => tree.nodes[4].style.width = Length::Percent(0.2 + (seed % 6) as f64 * 0.1),
        }
        let viewport = Size::new(400.0, 400.0);
        engine.compute(&mut tree, 0, viewport)?;
        assert_fresh_equivalent(&tree, viewport)?;
    }
    Ok(())
}

#[test]
fn flex_padding_and_gap_are_parent_relative() -> Result<(), LayoutError> {
    let root = LayoutInput {
        padding: Edges::all(Length::Points(10.0)),
        column_gap: Length::Points(10.0),
        ..fixed(200.0, 60.0)
    };
    let mut tree = Tree::new(
        vec![root, fixed(50.0, 20.0), fixed(50.0, 20.0)],
        vec![vec![1, 2], vec![], vec![]],
    );
    let stats = TaffyLayout::new().compute(&mut tree, 0, Size::new(200.0, 60.0))?;
    near(tree.geometry(1).location.x(), 10.0);
    near(tree.geometry(2).location.x(), 70.0);
    near(tree.geometry(1).size.width(), 50.0);
    assert_eq!(stats.visited_nodes, 3);
    Ok(())
}
#[test]
fn grid_tracks_and_auto_placement_match_hand_calculation() -> Result<(), LayoutError> {
    let root = LayoutInput {
        display: Display::Grid,
        grid_template_columns: vec![TrackSize::Fr(1.0), TrackSize::Fr(2.0)],
        grid_template_rows: vec![TrackSize::Points(40.0), TrackSize::Points(40.0)],
        ..fixed(300.0, 80.0)
    };
    let mut tree = Tree::new(
        vec![
            root,
            LayoutInput::default(),
            LayoutInput::default(),
            LayoutInput::default(),
        ],
        vec![vec![1, 2, 3], vec![], vec![], vec![]],
    );
    TaffyLayout::new().compute(&mut tree, 0, Size::new(300.0, 80.0))?;
    near(tree.geometry(1).size.width(), 100.0);
    near(tree.geometry(2).size.width(), 200.0);
    near(tree.geometry(2).location.x(), 100.0);
    near(tree.geometry(3).location.y(), 40.0);
    Ok(())
}
#[test]
fn block_sibling_margins_collapse() -> Result<(), LayoutError> {
    let root = LayoutInput {
        display: Display::Block,
        width: Length::Points(200.0),
        ..LayoutInput::default()
    };
    let first = LayoutInput {
        margin: Edges {
            bottom: Length::Points(10.0),
            ..Edges::all(Length::Points(0.0))
        },
        ..fixed(100.0, 20.0)
    };
    let second = LayoutInput {
        margin: Edges {
            top: Length::Points(15.0),
            ..Edges::all(Length::Points(0.0))
        },
        ..fixed(100.0, 20.0)
    };
    let mut tree = Tree::new(vec![root, first, second], vec![vec![1, 2], vec![], vec![]]);
    TaffyLayout::new().compute(&mut tree, 0, Size::new(200.0, 400.0))?;
    near(tree.geometry(2).location.y(), 35.0);
    near(tree.geometry(0).size.height(), 55.0);
    Ok(())
}
#[test]
fn absolute_uses_positioned_ancestor_and_returns_structural_offsets() -> Result<(), LayoutError> {
    let root = LayoutInput {
        display: Display::Block,
        position: Position::Relative,
        ..fixed(300.0, 200.0)
    };
    let wrapper = LayoutInput {
        inset: Edges::all(Length::Points(100.0)),
        margin: Edges {
            left: Length::Points(40.0),
            ..Edges::all(Length::Points(0.0))
        },
        ..fixed(200.0, 80.0)
    };
    let child = LayoutInput {
        position: Position::Absolute,
        inset: Edges {
            left: Length::Points(12.0),
            top: Length::Points(18.0),
            ..Edges::all(Length::Auto)
        },
        ..fixed(30.0, 20.0)
    };
    let mut tree = Tree::new(vec![root, wrapper, child], vec![vec![1], vec![2], vec![]]);
    TaffyLayout::new().compute(&mut tree, 0, Size::new(300.0, 200.0))?;
    near(tree.geometry(1).location.x(), 40.0);
    near(
        tree.geometry(1).location.x() + tree.geometry(2).location.x(),
        12.0,
    );
    near(tree.geometry(2).location.y(), 18.0);
    Ok(())
}
#[test]
fn relative_offset_does_not_move_following_sibling() -> Result<(), LayoutError> {
    let root = LayoutInput {
        display: Display::Block,
        ..fixed(200.0, 100.0)
    };
    let first = LayoutInput {
        position: Position::Relative,
        inset: Edges {
            top: Length::Points(10.0),
            ..Edges::all(Length::Auto)
        },
        ..fixed(50.0, 20.0)
    };
    let mut tree = Tree::new(
        vec![root, first, fixed(50.0, 20.0)],
        vec![vec![1, 2], vec![], vec![]],
    );
    TaffyLayout::new().compute(&mut tree, 0, Size::new(200.0, 100.0))?;
    near(tree.geometry(1).location.y(), 10.0);
    near(tree.geometry(2).location.y(), 20.0);
    Ok(())
}
#[test]
fn hidden_descendants_have_zero_geometry() -> Result<(), LayoutError> {
    let hidden = LayoutInput {
        display: Display::None,
        ..fixed(100.0, 100.0)
    };
    let mut tree = Tree::new(
        vec![fixed(200.0, 100.0), hidden, fixed(20.0, 20.0)],
        vec![vec![1], vec![2], vec![]],
    );
    TaffyLayout::new().compute(&mut tree, 0, Size::new(200.0, 100.0))?;
    near(tree.geometry(1).size.width(), 0.0);
    near(tree.geometry(2).size.height(), 0.0);
    Ok(())
}
#[test]
fn invalid_measurement_does_not_publish_partial_geometry() {
    let mut tree = Tree::new(
        vec![fixed(200.0, 100.0), LayoutInput::default()],
        vec![vec![1], vec![]],
    );
    for node in &mut tree.nodes {
        node.layout.location = Point::new(77.0, 88.0);
    }
    if let Some(node) = tree.nodes.get_mut(1) {
        node.intrinsic.height = f64::NAN;
    }
    let result = TaffyLayout::new().compute(&mut tree, 0, Size::new(200.0, 100.0));
    assert!(matches!(result, Err(LayoutError::InvalidGeometry)));
    near(tree.geometry(0).location.x(), 77.0);
    near(tree.geometry(1).location.y(), 88.0);
}
#[test]
fn rejects_cycles_missing_children_and_depth() {
    let mut cycle = Tree::new(vec![LayoutInput::default()], vec![vec![0]]);
    assert!(matches!(
        TaffyLayout::new().compute(&mut cycle, 0, Size::new(100.0, 100.0)),
        Err(LayoutError::InvalidTree)
    ));
    let mut missing = Tree::new(vec![LayoutInput::default()], vec![vec![1]]);
    assert!(matches!(
        TaffyLayout::new().compute(&mut missing, 0, Size::new(100.0, 100.0)),
        Err(LayoutError::InvalidTree)
    ));
    let n = MAX_LAYOUT_DEPTH + 1;
    let styles = vec![LayoutInput::default(); n];
    let edges = (0..n)
        .map(|i| if i + 1 < n { vec![i + 1] } else { vec![] })
        .collect();
    let mut deep = Tree::new(styles, edges);
    assert!(matches!(
        TaffyLayout::new().compute(&mut deep, 0, Size::new(100.0, 100.0)),
        Err(LayoutError::ExcessiveDepth)
    ));
}

#[test]
fn supported_deep_tree_computes_all_layout_families() -> Result<(), LayoutError> {
    for (display, position) in [
        (Display::Block, Position::Static),
        (Display::Flex, Position::Static),
        (Display::Grid, Position::Static),
        (Display::Block, Position::Absolute),
        (Display::Flex, Position::Absolute),
        (Display::Grid, Position::Absolute),
    ] {
        let count = MAX_LAYOUT_DEPTH;
        let styles = vec![
            LayoutInput {
                display,
                position,
                ..fixed(100.0, 100.0)
            };
            count
        ];
        let edges = (0..count)
            .map(|i| if i + 1 < count { vec![i + 1] } else { vec![] })
            .collect();
        let mut tree = Tree::new(styles, edges);
        TaffyLayout::new().compute(&mut tree, 0, Size::new(100.0, 100.0))?;
        near(tree.geometry(count - 1).size.width(), 100.0);
    }
    Ok(())
}

#[test]
fn reverse_flex_uses_the_flow_start_by_default() -> Result<(), LayoutError> {
    let root = LayoutInput {
        flex_direction: FlexDirection::RowReverse,
        column_gap: Length::Points(10.0),
        ..fixed(200.0, 40.0)
    };
    let mut tree = Tree::new(
        vec![root, fixed(50.0, 20.0), fixed(50.0, 20.0)],
        vec![vec![1, 2], vec![], vec![]],
    );
    TaffyLayout::new().compute(&mut tree, 0, Size::new(200.0, 40.0))?;
    near(tree.geometry(1).location.x(), 150.0);
    near(tree.geometry(2).location.x(), 90.0);
    Ok(())
}

#[test]
fn external_tree_revalidates_and_republishes_all_boxes_on_cache_hits() -> Result<(), LayoutError> {
    fn assert_portable<T: Send + Sync>() {}
    assert_portable::<TaffyLayout<usize>>();
    let mut tree = branch_fixture();
    let viewport = Size::new(300.0, 300.0);
    let mut engine = TaffyLayout::new();
    engine.compute(&mut tree, 0, viewport)?;
    let expected: Vec<_> = tree.nodes.iter().map(|node| node.layout).collect();
    let first_order = std::mem::take(&mut tree.published);
    assert_eq!(first_order, [0, 1, 2, 3, 4]);

    tree.nodes
        .iter_mut()
        .for_each(|node| node.layout = LayoutBox::default());
    let clean = engine.compute(&mut tree, 0, viewport)?;
    assert_eq!(clean.visited_nodes, tree.nodes.len());
    assert_eq!(clean.resolved_styles, tree.nodes.len());
    assert_eq!(clean.layout_visits, 0);
    assert_eq!(clean.measured_nodes, 0);
    assert_eq!(tree.published, first_order);
    assert_eq!(
        tree.nodes
            .iter()
            .map(|node| node.layout)
            .collect::<Vec<_>>(),
        expected
    );

    tree.nodes[1].children.push(0);
    tree.published.clear();
    assert!(matches!(
        engine.compute(&mut tree, 0, viewport),
        Err(LayoutError::InvalidTree)
    ));
    tree.nodes[1].children.pop();
    assert!(tree.published.is_empty());
    assert_eq!(
        tree.nodes
            .iter()
            .map(|node| node.layout)
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(engine.cached_node_count(), expected.len());
    let restored = engine.compute(&mut tree, 0, viewport)?;
    assert_eq!(restored.layout_visits, 0);
    Ok(())
}

#[test]
fn block_parent_and_child_margins_collapse_until_padding_or_border_intervenes()
-> Result<(), LayoutError> {
    let root = LayoutInput {
        display: Display::Block,
        width: Length::Points(200.0),
        ..LayoutInput::default()
    };
    let wrapper = LayoutInput {
        display: Display::Block,
        ..LayoutInput::default()
    };
    let child = LayoutInput {
        margin: Edges {
            top: Length::Points(24.0),
            bottom: Length::Points(12.0),
            ..Edges::all(Length::Points(0.0))
        },
        ..fixed(100.0, 20.0)
    };
    let mut tree = Tree::new(
        vec![root, wrapper, child, fixed(100.0, 10.0)],
        vec![vec![1, 3], vec![2], vec![], vec![]],
    );
    let viewport = Size::new(200.0, 400.0);
    let mut engine = TaffyLayout::new();
    engine.compute(&mut tree, 0, viewport)?;
    near(tree.geometry(1).location.y(), 24.0);
    near(tree.geometry(2).location.y(), 0.0);
    near(tree.geometry(3).location.y(), 56.0);

    tree.nodes[1].style.padding.top = Length::Points(5.0);
    engine.compute(&mut tree, 0, viewport)?;
    near(tree.geometry(1).location.y(), 0.0);
    near(tree.geometry(2).location.y(), 29.0);
    assert_fresh_equivalent(&tree, viewport)?;

    tree.nodes[1].style.padding.top = Length::Points(0.0);
    tree.nodes[1].style.border_width = 2.0;
    engine.compute(&mut tree, 0, viewport)?;
    near(tree.geometry(1).location.y(), 0.0);
    near(tree.geometry(2).location.y(), 26.0);
    assert_fresh_equivalent(&tree, viewport)
}

#[test]
fn block_empty_box_collapses_positive_and_negative_margins() -> Result<(), LayoutError> {
    let root = LayoutInput {
        display: Display::Block,
        width: Length::Points(200.0),
        ..LayoutInput::default()
    };
    let mut first = fixed(100.0, 20.0);
    first.margin.bottom = Length::Points(20.0);
    let mut empty = LayoutInput {
        display: Display::Block,
        ..LayoutInput::default()
    };
    empty.margin.top = Length::Points(8.0);
    empty.margin.bottom = Length::Points(-6.0);
    let mut last = fixed(100.0, 10.0);
    last.margin.top = Length::Points(12.0);
    let mut tree = Tree::new(
        vec![root, first, empty, last],
        vec![vec![1, 2, 3], vec![], vec![], vec![]],
    );
    let viewport = Size::new(200.0, 200.0);
    let mut engine = TaffyLayout::new();
    engine.compute(&mut tree, 0, viewport)?;
    near(tree.geometry(3).location.y(), 34.0);
    near(tree.geometry(0).size.height(), 44.0);

    tree.nodes[2].style.border_width = 2.0;
    engine.compute(&mut tree, 0, viewport)?;
    near(tree.geometry(2).location.y(), 40.0);
    near(tree.geometry(3).location.y(), 50.0);
    assert_fresh_equivalent(&tree, viewport)
}

#[test]
fn block_contains_wrapping_flex_with_nested_grid() -> Result<(), LayoutError> {
    let root = LayoutInput {
        display: Display::Block,
        width: Length::Points(110.0),
        ..LayoutInput::default()
    };
    let flex = LayoutInput {
        flex_wrap: FlexWrap::Wrap,
        column_gap: Length::Points(10.0),
        row_gap: Length::Points(5.0),
        ..LayoutInput::default()
    };
    let grid = LayoutInput {
        display: Display::Grid,
        grid_template_columns: vec![TrackSize::Fr(1.0), TrackSize::Fr(2.0)],
        ..fixed(45.0, 20.0)
    };
    let mut tree = Tree::new(
        vec![
            root,
            flex,
            grid,
            fixed(45.0, 20.0),
            fixed(45.0, 20.0),
            LayoutInput::default(),
            LayoutInput::default(),
        ],
        vec![
            vec![1],
            vec![2, 3, 4],
            vec![5, 6],
            vec![],
            vec![],
            vec![],
            vec![],
        ],
    );
    TaffyLayout::new().compute(&mut tree, 0, Size::new(110.0, 200.0))?;
    near(tree.geometry(0).size.height(), 45.0);
    near(tree.geometry(3).location.x(), 55.0);
    near(tree.geometry(4).location.y(), 25.0);
    near(tree.geometry(5).size.width(), 15.0);
    near(tree.geometry(6).location.x(), 15.0);
    near(tree.geometry(6).size.width(), 30.0);
    Ok(())
}

#[test]
fn incremental_adapter_leaves_clean_descendant_geometry_at_the_caller() -> Result<(), LayoutError> {
    let mut tree = branch_fixture();
    let viewport = Size::new(300.0, 300.0);
    let mut engine = IncrementalTaffyLayout::default();
    engine.compute(&mut tree, 0, viewport, &HashMap::new())?;
    tree.published.clear();

    tree.nodes[2].style.height = Length::Points(21.0);
    let changes = HashMap::from([(
        2,
        LayoutChange::Update {
            parent: Some(1),
            style: true,
            children: false,
        },
    )]);
    let stats = engine.compute(&mut tree, 0, viewport, &changes)?;
    assert_eq!(stats.visited_nodes, 1);
    assert_eq!(stats.resolved_styles, 1);
    assert!(tree.published.contains(&2));
    assert!(!tree.published.contains(&4));
    near(tree.geometry(2).size.height(), 21.0);
    assert_fresh_equivalent(&tree, viewport)
}

#[test]
fn static_and_relative_insets_preserve_flow_in_all_layout_families() -> Result<(), LayoutError> {
    let viewport = Size::new(200.0, 100.0);
    for (display, sibling) in [
        (Display::Block, Point::new(0.0, 20.0)),
        (Display::Flex, Point::new(20.0, 0.0)),
        (Display::Grid, Point::new(50.0, 0.0)),
    ] {
        let root = LayoutInput {
            display,
            grid_template_columns: vec![TrackSize::Points(50.0); 2],
            grid_template_rows: vec![TrackSize::Points(30.0)],
            ..fixed(200.0, 100.0)
        };
        let first = LayoutInput {
            inset: Edges {
                left: Length::Points(-6.0),
                top: Length::Percent(0.25),
                ..Edges::all(Length::Auto)
            },
            ..fixed(20.0, 20.0)
        };
        let mut tree = Tree::new(
            vec![root, first, fixed(20.0, 20.0)],
            vec![vec![1, 2], vec![], vec![]],
        );
        let mut engine = IncrementalTaffyLayout::default();
        engine.compute(&mut tree, 0, viewport, &HashMap::new())?;
        assert_eq!(tree.geometry(1).location, Point::new(0.0, 0.0));
        assert_eq!(tree.geometry(2).location, sibling);

        tree.nodes[1].style.position = Position::Relative;
        engine.compute(
            &mut tree,
            0,
            viewport,
            &HashMap::from([(
                1,
                LayoutChange::Update {
                    parent: Some(0),
                    style: true,
                    children: false,
                },
            )]),
        )?;
        near(tree.geometry(1).location.x(), -6.0);
        near(
            tree.geometry(1).location.y(),
            match display {
                Display::Block | Display::Flex => 25.0,
                Display::Grid => 7.5,
                Display::None => 0.0,
            },
        );
        assert_eq!(tree.geometry(2).location, sibling);
        assert_fresh_equivalent(&tree, viewport)?;
    }
    Ok(())
}

#[test]
fn fixed_viewport_absolute_branch_recomputes_locally_and_rolls_back() -> Result<(), LayoutError> {
    let mut tree = branch_fixture();
    tree.nodes[1].style.position = Position::Absolute;
    tree.nodes[1].style.inset.right = Length::Percent(0.1);
    let viewport = Size::new(400.0, 300.0);
    let mut engine = IncrementalTaffyLayout::default();
    engine.compute(&mut tree, 0, viewport, &HashMap::new())?;
    near(tree.geometry(1).location.x(), 240.0);
    near(tree.geometry(3).location.y(), 0.0);

    let changes = HashMap::from([(
        2,
        LayoutChange::Update {
            parent: Some(1),
            style: false,
            children: false,
        },
    )]);
    let clean_measurements = tree.nodes[4].measure_calls;
    let before = [0, 1, 2, 3, 4].map(|id| tree.geometry(id));
    tree.nodes[2].revision += 1;
    tree.nodes[2].intrinsic.height = f64::NAN;
    tree.published.clear();
    assert!(matches!(
        engine.compute(&mut tree, 0, viewport, &changes),
        Err(LayoutError::InvalidGeometry)
    ));
    assert!(tree.published.is_empty());
    assert_eq!([0, 1, 2, 3, 4].map(|id| tree.geometry(id)), before);
    assert_eq!(engine.cached_node_count(), 5);

    // Retry the same pending revision. Only the absolute branch is published,
    // including its changed overflow before clipping.
    tree.nodes[2].intrinsic.height = 130.0;
    let stats = engine.compute(&mut tree, 0, viewport, &changes)?;
    assert_eq!(stats.visited_nodes, 1);
    assert_eq!(stats.resolved_styles, 0);
    tree.published.sort_unstable();
    assert_eq!(tree.published, [1, 2]);
    assert_eq!(tree.nodes[4].measure_calls, clean_measurements);
    near(tree.geometry(1).size.height(), 100.0);
    near(tree.geometry(1).content_size.height(), 130.0);
    assert_fresh_equivalent(&tree, viewport)
}

#[test]
fn absolute_intrinsic_size_and_containing_block_overflow_reach_the_owner() -> Result<(), LayoutError>
{
    let viewport = Size::new(300.0, 300.0);
    for (root_position, height) in [
        (Position::Relative, Length::Points(100.0)),
        (Position::Static, Length::Auto),
    ] {
        let mut tree = branch_fixture();
        tree.nodes[0].style.position = root_position;
        tree.nodes[1].style.position = Position::Absolute;
        tree.nodes[1].style.height = height;
        tree.nodes[1].style.inset.bottom = Length::Points(10.0);
        let mut engine = IncrementalTaffyLayout::default();
        engine.compute(&mut tree, 0, viewport, &HashMap::new())?;
        let clean_measurements = tree.nodes[4].measure_calls;

        tree.nodes[2].revision += 1;
        tree.nodes[2].intrinsic.height = 140.0;
        tree.published.clear();
        engine.compute(
            &mut tree,
            0,
            viewport,
            &HashMap::from([(
                2,
                LayoutChange::Update {
                    parent: Some(1),
                    style: false,
                    children: false,
                },
            )]),
        )?;
        assert!(tree.published.contains(&0));
        assert_eq!(tree.nodes[4].measure_calls, clean_measurements);
        if root_position == Position::Relative {
            near(tree.geometry(1).location.y(), 190.0);
            near(tree.geometry(0).content_size.height(), 330.0);
        } else {
            near(tree.geometry(1).location.y(), 150.0);
            near(tree.geometry(1).size.height(), 140.0);
        }
        assert_fresh_equivalent(&tree, viewport)?;
    }
    Ok(())
}

#[test]
fn removing_the_last_child_invalidates_the_scanning_adapter_parent() -> Result<(), LayoutError> {
    let mut tree = branch_fixture();
    let viewport = Size::new(300.0, 300.0);
    let mut engine = TaffyLayout::new();
    engine.compute(&mut tree, 0, viewport)?;
    near(tree.geometry(1).content_size.height(), 18.0);
    tree.nodes[1].children.clear();
    engine.compute(&mut tree, 0, viewport)?;
    assert_eq!(engine.cached_node_count(), 4);
    near(tree.geometry(1).content_size.height(), 0.0);
    assert_fresh_equivalent(&tree, viewport)
}

#[test]
fn overflowing_resolved_constraints_do_not_reach_measurement() -> Result<(), LayoutError> {
    let mut tree = Tree::new(vec![LayoutInput::default()], vec![vec![]]);
    let viewport = Size::new(100.0, 100.0);
    let mut engine = IncrementalTaffyLayout::default();
    engine.compute(&mut tree, 0, viewport, &HashMap::new())?;
    let measurements = tree.measures;
    tree.published.clear();

    tree.nodes[0].style.padding.left = Length::Percent(f64::from(f32::MAX));
    let changes = HashMap::from([(
        0,
        LayoutChange::Update {
            parent: None,
            style: true,
            children: false,
        },
    )]);
    assert!(matches!(
        engine.compute(&mut tree, 0, viewport, &changes),
        Err(LayoutError::InvalidGeometry)
    ));
    assert_eq!(tree.measures, measurements);
    assert!(tree.published.is_empty());
    Ok(())
}

#[test]
fn malformed_sparse_child_list_preserves_the_committed_pass() -> Result<(), LayoutError> {
    let mut tree = branch_fixture();
    let viewport = Size::new(300.0, 300.0);
    let mut engine = IncrementalTaffyLayout::default();
    engine.compute(&mut tree, 0, viewport, &HashMap::new())?;
    let changes = HashMap::from([(
        0,
        LayoutChange::Update {
            parent: None,
            style: false,
            children: true,
        },
    )]);
    tree.published.clear();
    tree.nodes[0].children.push(1);
    assert!(matches!(
        engine.compute(&mut tree, 0, viewport, &changes),
        Err(LayoutError::InvalidTree)
    ));
    assert!(tree.published.is_empty());
    assert_eq!(engine.cached_node_count(), 5);

    tree.nodes[0].children = vec![3, 1];
    engine.compute(&mut tree, 0, viewport, &changes)?;
    near(tree.geometry(3).location.y(), 0.0);
    near(tree.geometry(1).location.y(), 100.0);
    assert_fresh_equivalent(&tree, viewport)
}

#[test]
fn mixed_sparse_edits_match_fresh_layout_across_position_and_display_modes()
-> Result<(), LayoutError> {
    let mut tree = branch_fixture();
    let mut engine = IncrementalTaffyLayout::default();
    let mut viewport = Size::new(300.0, 300.0);
    engine.compute(&mut tree, 0, viewport, &HashMap::new())?;
    let mut parents = [None, Some(0), Some(1), Some(0), Some(3)];
    let mut seed = 29u64;
    for step in 0..300 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let id = ((seed >> 32) % 5) as usize;
        let state = (seed >> 40) as usize;
        let mut changes = HashMap::new();
        let mut update = |id, style, children| {
            changes.insert(
                id,
                LayoutChange::Update {
                    parent: parents[id],
                    style,
                    children,
                },
            );
        };
        match step % 7 {
            0 => {
                tree.nodes[id].style.display =
                    [Display::Block, Display::Flex, Display::Grid, Display::None][state % 4];
                update(id, true, false);
            }
            1 => {
                tree.nodes[id].style.position =
                    [Position::Static, Position::Relative, Position::Absolute][state % 3];
                tree.nodes[id].style.inset.bottom = Length::Percent(0.1);
                update(id, true, false);
            }
            2 => {
                tree.nodes[id].style.width =
                    [Length::Auto, Length::Points(73.0), Length::Percent(0.45)][state % 3];
                tree.nodes[id].style.padding = Edges::all(Length::Percent(0.04));
                update(id, true, false);
            }
            3 => {
                let leaf = [2, 4][state % 2];
                let old = parents[leaf].unwrap();
                let new = [0, 1, 3][(state >> 2) % 3];
                if old != new {
                    tree.nodes[old].children.retain(|&child| child != leaf);
                    tree.nodes[new].children.push(leaf);
                    update(old, false, true);
                    update(new, false, true);
                    parents[leaf] = Some(new);
                    changes.insert(
                        leaf,
                        LayoutChange::Update {
                            parent: Some(new),
                            style: false,
                            children: false,
                        },
                    );
                }
            }
            4 => {
                tree.nodes[id].intrinsic.height = (state % 130) as f64;
                tree.nodes[id].revision += 1;
                update(id, false, false);
            }
            5 => {
                tree.nodes[id].children.reverse();
                update(id, false, true);
            }
            _ => viewport = Size::new(100.0 + (state % 500) as f64, 330.0),
        }
        engine
            .compute(&mut tree, 0, viewport, &changes)
            .unwrap_or_else(|error| panic!("step {step}: {error}"));
        let mut fresh = tree.clone();
        TaffyLayout::new().compute(&mut fresh, 0, viewport)?;
        for id in 0..tree.nodes.len() {
            assert_eq!(
                tree.geometry(id),
                fresh.geometry(id),
                "step {step}, node {id}"
            );
        }
    }
    Ok(())
}

#[test]
fn sparse_layout_rejects_a_missing_cached_child_before_using_caller_accessors()
-> Result<(), LayoutError> {
    struct StrictTree(Tree);

    impl LayoutTree for StrictTree {
        type NodeId = usize;

        fn children(&self, id: usize) -> &[usize] {
            &self.0.nodes[id].children
        }

        fn layout_input(&self, id: usize) -> LayoutInput {
            self.0.nodes[id].style.clone()
        }

        fn set_layout(&mut self, id: usize, layout: LayoutBox) {
            self.0.nodes[id].layout = layout;
            self.0.published.push(id);
        }

        fn contains_node(&self, id: usize) -> bool {
            self.0.contains_node(id)
        }
    }

    let mut tree = StrictTree(branch_fixture());
    let viewport = Size::new(300.0, 300.0);
    let mut engine = IncrementalTaffyLayout::default();
    let mut full = TaffyLayout::new();
    full.compute(&mut tree, 0, viewport)?;
    engine.compute(&mut tree, 0, viewport, &HashMap::new())?;
    tree.0.published.clear();
    tree.0.nodes.pop();
    tree.0.nodes[3].style.padding = Edges::all(Length::Points(4.0));
    let mut changes = HashMap::from([(
        3,
        LayoutChange::Update {
            parent: Some(0),
            style: true,
            children: false,
        },
    )]);
    assert!(matches!(
        full.compute(&mut tree, 0, viewport),
        Err(LayoutError::InvalidTree)
    ));
    assert!(matches!(
        engine.compute(&mut tree, 0, viewport, &changes),
        Err(LayoutError::InvalidTree)
    ));
    assert!(tree.0.published.is_empty());
    assert_eq!(engine.cached_node_count(), 5);

    tree.0.nodes[3].children.clear();
    changes.insert(
        3,
        LayoutChange::Update {
            parent: Some(0),
            style: true,
            children: true,
        },
    );
    changes.insert(4, LayoutChange::Remove);
    engine.compute(&mut tree, 0, viewport, &changes)?;
    assert_eq!(engine.cached_node_count(), 4);
    assert!(tree.0.published.contains(&3));
    Ok(())
}

#[test]
fn sparse_layout_rejects_a_move_missing_from_the_destination_child_list() -> Result<(), LayoutError>
{
    let mut tree = branch_fixture();
    tree.nodes[2].style.position = Position::Absolute;
    tree.nodes[2].style.inset.top = Length::Points(7.0);
    let viewport = Size::new(300.0, 300.0);
    let mut engine = IncrementalTaffyLayout::default();
    engine.compute(&mut tree, 0, viewport, &HashMap::new())?;
    let before = [0, 1, 2, 3, 4].map(|id| tree.geometry(id));
    tree.published.clear();
    tree.nodes[1].children.clear();
    let changes = [(1, 0, true), (2, 3, false), (3, 0, true)]
        .map(|(id, parent, children)| {
            (
                id,
                LayoutChange::Update {
                    parent: Some(parent),
                    style: false,
                    children,
                },
            )
        })
        .into_iter()
        .collect();
    assert!(matches!(
        engine.compute(&mut tree, 0, viewport, &changes),
        Err(LayoutError::InvalidTree)
    ));
    assert!(tree.published.is_empty());
    assert_eq!([0, 1, 2, 3, 4].map(|id| tree.geometry(id)), before);

    tree.nodes[3].children.push(2);
    tree.nodes[1].children.push(2);
    let mut incomplete = changes.clone();
    incomplete.remove(&1);
    assert!(matches!(
        engine.compute(&mut tree, 0, viewport, &incomplete),
        Err(LayoutError::InvalidTree)
    ));
    assert!(tree.published.is_empty());
    assert_eq!([0, 1, 2, 3, 4].map(|id| tree.geometry(id)), before);

    tree.nodes[1].children.clear();
    engine.compute(&mut tree, 0, viewport, &changes)?;
    near(tree.geometry(2).location.y(), -93.0);
    assert_fresh_equivalent(&tree, viewport)
}

#[test]
fn sparse_layout_rejects_a_disconnected_cycle_in_reported_moves() -> Result<(), LayoutError> {
    let mut tree = branch_fixture();
    let viewport = Size::new(300.0, 300.0);
    let mut engine = IncrementalTaffyLayout::default();
    engine.compute(&mut tree, 0, viewport, &HashMap::new())?;
    tree.published.clear();
    tree.nodes[0].children.clear();
    tree.nodes[1].children.push(3);
    tree.nodes[3].children.push(1);
    let changes = HashMap::from([(0, None), (1, Some(3)), (3, Some(1))].map(|(id, parent)| {
        (
            id,
            LayoutChange::Update {
                parent,
                style: false,
                children: true,
            },
        )
    }));
    assert!(matches!(
        engine.compute(&mut tree, 0, viewport, &changes),
        Err(LayoutError::InvalidTree)
    ));
    assert!(tree.published.is_empty());
    assert_eq!(engine.cached_node_count(), 5);
    Ok(())
}
