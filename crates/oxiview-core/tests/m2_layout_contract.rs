//! Shared public-boundary fixtures with two independently implemented engines.
#![allow(clippy::unwrap_used)]

use oxiview_base::{Point, Rect, Size};
use oxiview_core::{NodeId, UiError, UiTree};
use oxiview_layout::{
    Alignment, AvailableSpace, Dimensions, FlexDirection, InputError, LayoutBox, LayoutEngine,
    LayoutError, LayoutInput, LayoutStats, LayoutTree, Length, MAX_LAYOUT_DEPTH, TaffyLayout,
};
use oxiview_style::{
    Alignment as StyleAlignment, Display, FlexDirection as StyleDirection, Position, Style,
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

/// Arranges static, start-aligned columns of preferred-width/height boxes.
/// Auto-sized leaves use the caller's measurement, and auto-sized columns
/// take the maximum child width and the sum of child heights.
#[derive(Default)]
struct ColumnLayout {
    cached: HashMap<NodeId, (SizeInput, Size)>,
}

#[derive(PartialEq)]
struct SizeInput {
    preferred: Dimensions<Option<f64>>,
    available: Dimensions<AvailableSpace>,
    revision: u64,
    children: Vec<Size>,
}

struct ColumnPass<'a> {
    previous: &'a HashMap<NodeId, (SizeInput, Size)>,
    cached: HashMap<NodeId, (SizeInput, Size)>,
    output: HashMap<NodeId, LayoutBox>,
    visited: HashSet<NodeId>,
    stats: LayoutStats,
}

impl ColumnPass<'_> {
    fn arrange(
        &mut self,
        tree: &mut impl LayoutTree<NodeId = NodeId>,
        node: NodeId,
        available: Dimensions<AvailableSpace>,
        depth: usize,
    ) -> Result<Size, LayoutError> {
        if depth > MAX_LAYOUT_DEPTH {
            return Err(LayoutError::ExcessiveDepth);
        }
        if !self.visited.insert(node) || !tree.contains_node(node) {
            return Err(LayoutError::InvalidTree);
        }
        self.stats.visited_nodes += 1;
        let input = tree.layout_input(node);
        input.validate()?;
        if input
            != (LayoutInput {
                width: input.width,
                height: input.height,
                flex_direction: FlexDirection::Column,
                align_items: Alignment::Start,
                ..LayoutInput::default()
            })
        {
            return Err(InputError {
                property: "reference layout mode",
            }
            .into());
        }
        let preferred = Dimensions {
            width: resolve(input.width, available.width),
            height: resolve(input.height, available.height),
        };
        let children = tree.children(node).to_vec();
        let child_space = Dimensions {
            width: preferred
                .width
                .map_or(available.width, AvailableSpace::Definite),
            height: AvailableSpace::MaxContent,
        };
        let mut sizes = Vec::with_capacity(children.len());
        let (mut width, mut height) = (0.0_f64, 0.0);
        for child in children {
            let size = self.arrange(tree, child, child_space, depth + 1)?;
            self.output
                .get_mut(&child)
                .ok_or(LayoutError::InvalidTree)?
                .location = Point::new(0.0, height);
            width = width.max(size.width());
            height += size.height();
            sizes.push(size);
        }
        let key = SizeInput {
            preferred,
            available,
            revision: tree.content_revision(node),
            children: sizes,
        };
        let size = if let Some((old_key, size)) = self.previous.get(&node)
            && *old_key == key
        {
            self.stats.cache_hits += 1;
            *size
        } else {
            self.stats.computed_nodes += 1;
            if key.children.is_empty() && (preferred.width.is_none() || preferred.height.is_none())
            {
                self.stats.measured_nodes += 1;
                let measured = tree.measure(node, preferred, key.available);
                if !valid(Size::new(measured.width, measured.height)) {
                    return Err(LayoutError::InvalidGeometry);
                }
                width = measured.width;
                height = measured.height;
            }
            Size::new(
                preferred.width.unwrap_or(width),
                preferred.height.unwrap_or(height),
            )
        };
        if !valid(size) || !valid(Size::new(width, height)) {
            return Err(LayoutError::InvalidGeometry);
        }
        self.output.insert(
            node,
            LayoutBox {
                size,
                content_size: size,
                ..LayoutBox::default()
            },
        );
        self.cached.insert(node, (key, size));
        Ok(size)
    }
}

impl LayoutEngine<NodeId> for ColumnLayout {
    fn compute<T: LayoutTree<NodeId = NodeId>>(
        &mut self,
        tree: &mut T,
        root: NodeId,
        viewport: Size,
    ) -> Result<LayoutStats, LayoutError> {
        if !valid(viewport) {
            return Err(LayoutError::InvalidViewport);
        }
        let mut pass = ColumnPass {
            previous: &self.cached,
            cached: HashMap::new(),
            output: HashMap::new(),
            visited: HashSet::new(),
            stats: LayoutStats::default(),
        };
        pass.arrange(
            tree,
            root,
            Dimensions {
                width: AvailableSpace::Definite(viewport.width()),
                height: AvailableSpace::Definite(viewport.height()),
            },
            1,
        )?;
        let ColumnPass {
            cached,
            output,
            stats,
            ..
        } = pass;
        for (node, layout) in output {
            tree.set_layout(node, layout);
        }
        self.cached = cached;
        Ok(stats)
    }

    fn layout_boundary(&self, node: NodeId) -> Option<bool> {
        // A column's parent uses only its returned size. When both axes
        // resolved, that size came entirely from the committed input.
        let (input, _) = self.cached.get(&node)?;
        Some(input.preferred.width.is_some() && input.preferred.height.is_some())
    }
}

fn resolve(length: Length, available: AvailableSpace) -> Option<f64> {
    match (length, available) {
        (Length::Points(value), _) => Some(value),
        (Length::Percent(fraction), AvailableSpace::Definite(basis)) => Some(fraction * basis),
        _ => None,
    }
}

fn valid(size: Size) -> bool {
    [size.width(), size.height()]
        .into_iter()
        .all(|axis| (0.0..=f64::from(f32::MAX)).contains(&axis))
}

fn column(width: oxiview_style::Length, height: oxiview_style::Length) -> Style {
    let mut style = Style::default();
    style.layout.width = width;
    style.layout.height = height;
    style.layout.flex_direction = StyleDirection::Column;
    style.layout.align_items = StyleAlignment::Start;
    style
}

fn constraints_and_reuse(mut engine: impl LayoutEngine<NodeId>) {
    use oxiview_style::Length::{Auto, Percent, Points};
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    tree.set_style(root, column(Auto, Auto)).unwrap();
    let leaf = tree.append(root, column(Percent(0.5), Auto)).unwrap();
    let sibling = tree
        .append(root, column(Points(20.0), Points(15.0)))
        .unwrap();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&requests);
    tree.set_measure(
        leaf,
        move |known: Dimensions<Option<f64>>, available: Dimensions<AvailableSpace>| {
            log.borrow_mut().push((known, available));
            let width = known.width.unwrap_or(10.0);
            Dimensions {
                width,
                height: known.height.unwrap_or(width / 4.0),
            }
        },
    )
    .unwrap();
    let sibling_calls = Rc::new(Cell::new(0));
    let measured = Rc::clone(&sibling_calls);
    tree.set_measure(sibling, move |known: Dimensions<Option<f64>>, _| {
        measured.set(measured.get() + 1);
        Dimensions {
            width: known.width.unwrap_or(20.0),
            height: known.height.unwrap_or(15.0),
        }
    })
    .unwrap();

    for (width, leaf_width, leaf_height) in [(160.0, 80.0, 20.0), (240.0, 120.0, 30.0)] {
        let viewport = Size::new(width, 100.0);
        let first = requests.borrow().len();
        let first_sibling = sibling_calls.get();
        let stats = tree.layout_with(&mut engine, viewport).unwrap();
        assert_eq!(stats.visited_nodes, 3);
        assert!(stats.computed_nodes > 0);
        assert_eq!(
            stats.measured_nodes,
            requests.borrow().len() - first + sibling_calls.get() - first_sibling
        );
        assert!(
            requests.borrow()[first..].iter().any(|(known, space)| {
                known.width == Some(leaf_width)
                    && known.height.is_none()
                    && matches!(space.width, AvailableSpace::Definite(width) if width > 0.0)
                    && matches!(
                        space.height,
                        AvailableSpace::MinContent | AvailableSpace::MaxContent
                    )
            }),
            "{:?}",
            requests.borrow()
        );
        assert_eq!(
            tree.bounds(root).unwrap(),
            Rect::new(0.0, 0.0, width, 100.0)
        );
        assert_eq!(
            tree.bounds(leaf).unwrap(),
            Rect::new(0.0, 0.0, leaf_width, leaf_height)
        );
        assert_eq!(
            tree.bounds(sibling).unwrap(),
            Rect::new(0.0, leaf_height, 20.0, leaf_height + 15.0)
        );

        let count = requests.borrow().len();
        let sibling_count = sibling_calls.get();
        let clean = tree.layout_with(&mut engine, viewport).unwrap();
        assert_eq!(clean.computed_nodes, 0);
        assert_eq!(clean.measured_nodes, 0);
        assert!(clean.cache_hits > 0);
        assert_eq!(requests.borrow().len(), count);
        assert_eq!(sibling_calls.get(), sibling_count);
    }
}

fn counted_leaf(tree: &mut UiTree, parent: NodeId) -> (NodeId, Rc<Cell<f64>>, Rc<Cell<usize>>) {
    use oxiview_style::Length::{Auto, Points};
    let leaf = tree.append(parent, column(Points(30.0), Auto)).unwrap();
    let height = Rc::new(Cell::new(12.0));
    let calls = Rc::new(Cell::new(0));
    let measured_height = Rc::clone(&height);
    let measured_calls = Rc::clone(&calls);
    tree.set_measure(leaf, move |known: Dimensions<Option<f64>>, _| {
        measured_calls.set(measured_calls.get() + 1);
        Dimensions {
            width: known.width.unwrap_or(30.0),
            height: known.height.unwrap_or_else(|| measured_height.get()),
        }
    })
    .unwrap();
    (leaf, height, calls)
}

fn ancestor_sizing_and_failures(mut engine: impl LayoutEngine<NodeId>, fixed_is_boundary: bool) {
    use oxiview_style::Length::{Auto, Points};
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    tree.set_style(root, column(Auto, Auto)).unwrap();
    let content = tree.append(root, column(Points(60.0), Auto)).unwrap();
    let fixed = tree
        .append(root, column(Points(80.0), Points(40.0)))
        .unwrap();
    let marker = tree
        .append(root, column(Points(15.0), Points(5.0)))
        .unwrap();
    let (content_leaf, content_height, content_calls) = counted_leaf(&mut tree, content);
    let (fixed_leaf, fixed_height, fixed_calls) = counted_leaf(&mut tree, fixed);
    let viewport = Size::new(200.0, 150.0);
    assert_eq!(engine.layout_boundary(fixed), None);
    tree.layout_with(&mut engine, viewport).unwrap();
    assert_eq!(engine.layout_boundary(content), Some(false));
    assert_eq!(engine.layout_boundary(fixed), Some(fixed_is_boundary));
    assert_eq!(tree.bounds(content).unwrap().size().height(), 12.0);
    assert_eq!(tree.bounds(fixed).unwrap().size().height(), 40.0);
    assert_eq!(tree.bounds(marker).unwrap().origin().y(), 52.0);

    let unrelated = content_calls.get();
    fixed_height.set(26.0);
    tree.invalidate_measure(fixed_leaf).unwrap();
    tree.layout_with(&mut engine, viewport).unwrap();
    assert_eq!(content_calls.get(), unrelated);
    assert_eq!(tree.bounds(fixed_leaf).unwrap().size().height(), 26.0);
    assert_eq!(tree.bounds(fixed).unwrap().size().height(), 40.0);
    assert_eq!(tree.bounds(marker).unwrap().origin().y(), 52.0);

    let unrelated = fixed_calls.get();
    content_height.set(32.0);
    tree.invalidate_measure(content_leaf).unwrap();
    tree.layout_with(&mut engine, viewport).unwrap();
    assert_eq!(fixed_calls.get(), unrelated);
    assert_eq!(tree.bounds(content).unwrap().size().height(), 32.0);
    assert_eq!(tree.bounds(fixed).unwrap().origin().y(), 32.0);
    assert_eq!(tree.bounds(marker).unwrap().origin().y(), 72.0);

    let ids = [root, content, fixed, marker, content_leaf, fixed_leaf];
    let before = ids.map(|node| tree.bounds(node).unwrap());
    let diagnostics = tree.diagnostics();
    for (invalid, error) in [
        (Size::new(f64::NAN, 100.0), UiError::InvalidViewport),
        (
            Size::new(f64::MAX, 100.0),
            UiError::Layout(LayoutError::InvalidViewport.to_string()),
        ),
    ] {
        assert_eq!(tree.layout_with(&mut engine, invalid), Err(error));
        assert_eq!(ids.map(|node| tree.bounds(node).unwrap()), before);
        assert_eq!(tree.diagnostics(), diagnostics);
    }
    // The content branch changes before the later fixed branch emits invalid
    // geometry. Neither geometry nor successful-work counters may be published.
    content_height.set(18.0);
    fixed_height.set(f64::NAN);
    tree.set_style(fixed, column(Points(80.0), Auto)).unwrap();
    tree.invalidate_measure(content_leaf).unwrap();
    tree.invalidate_measure(fixed_leaf).unwrap();
    assert_eq!(
        tree.layout_with(&mut engine, viewport),
        Err(UiError::Layout(LayoutError::InvalidGeometry.to_string()))
    );
    assert_eq!(ids.map(|node| tree.bounds(node).unwrap()), before);
    assert_eq!(tree.diagnostics(), diagnostics);
    assert_eq!(engine.layout_boundary(fixed), Some(fixed_is_boundary));
    fixed_height.set(28.0);
    tree.set_style(fixed, column(Points(80.0), Points(40.0)))
        .unwrap();
    tree.invalidate_measure(fixed_leaf).unwrap();
    tree.layout_with(&mut engine, viewport).unwrap();
    assert_eq!(tree.bounds(marker).unwrap().origin().y(), 58.0);

    tree.remove(fixed).unwrap();
    let replacement = tree
        .append(root, column(Points(80.0), Points(40.0)))
        .unwrap();
    assert_eq!(engine.layout_boundary(fixed), Some(fixed_is_boundary));
    assert_eq!(engine.layout_boundary(replacement), None);
    tree.layout_with(&mut engine, viewport).unwrap();
    assert_eq!(engine.layout_boundary(fixed), None);
    assert_eq!(engine.layout_boundary(replacement), Some(fixed_is_boundary));
}

#[test]
fn reference_engine_satisfies_retained_layout_contract() {
    constraints_and_reuse(ColumnLayout::default());
    ancestor_sizing_and_failures(ColumnLayout::default(), true);
}

#[test]
fn taffy_satisfies_retained_layout_contract() {
    constraints_and_reuse(TaffyLayout::new());
    ancestor_sizing_and_failures(TaffyLayout::new(), false);
}

#[test]
fn taffy_reports_suppressed_and_viewport_boundaries_under_committed_constraints() {
    use oxiview_style::Length::{Auto, Points};
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let mut root_style = column(Auto, Auto);
    tree.set_style(root, root_style.clone()).unwrap();
    let wrapper = tree.append(root, column(Points(120.0), Auto)).unwrap();
    let mut absolute_style = column(Points(80.0), Points(40.0));
    absolute_style.layout.position = Position::Absolute;
    let absolute = tree.append(wrapper, absolute_style.clone()).unwrap();
    let (leaf, height, _) = counted_leaf(&mut tree, absolute);
    let mut hidden_style = column(Auto, Auto);
    hidden_style.layout.display = Display::None;
    let hidden = tree.append(root, hidden_style.clone()).unwrap();
    let hidden_child = tree.append(hidden, absolute_style.clone()).unwrap();

    let mut engine = TaffyLayout::new();
    let viewport = Size::new(200.0, 150.0);
    tree.layout_with(&mut engine, viewport).unwrap();
    assert_eq!(engine.layout_boundary(absolute), Some(true));
    assert_eq!(engine.layout_boundary(hidden), Some(true));
    assert_eq!(engine.layout_boundary(hidden_child), Some(false));
    let absolute_box = tree.bounds(absolute).unwrap();

    height.set(70.0);
    tree.invalidate_measure(leaf).unwrap();
    tree.layout_with(&mut engine, viewport).unwrap();
    assert_eq!(tree.bounds(absolute).unwrap(), absolute_box);
    assert_eq!(tree.bounds(leaf).unwrap().size().height(), 70.0);
    assert_eq!(engine.layout_boundary(absolute), Some(true));

    // A positioned root would receive the box's overflow, so the pending
    // containing block change would remove the certification if committed.
    root_style.layout.position = Position::Relative;
    tree.set_style(root, root_style.clone()).unwrap();
    height.set(f64::NAN);
    tree.invalidate_measure(leaf).unwrap();
    assert_eq!(
        tree.layout_with(&mut engine, viewport),
        Err(UiError::Layout(LayoutError::InvalidGeometry.to_string()))
    );
    assert_eq!(tree.bounds(absolute).unwrap(), absolute_box);
    assert_eq!(engine.layout_boundary(absolute), Some(true));

    height.set(75.0);
    tree.invalidate_measure(leaf).unwrap();
    tree.layout_with(&mut engine, viewport).unwrap();
    assert_eq!(engine.layout_boundary(absolute), Some(false));

    root_style.layout.position = Position::Static;
    tree.set_style(root, root_style).unwrap();
    absolute_style.layout.height = Auto;
    tree.set_style(absolute, absolute_style).unwrap();
    hidden_style.layout.display = Display::Flex;
    tree.set_style(hidden, hidden_style).unwrap();
    tree.layout_with(&mut engine, viewport).unwrap();
    assert_eq!(engine.layout_boundary(absolute), Some(false));
    assert_eq!(engine.layout_boundary(hidden), Some(false));
    assert_eq!(engine.layout_boundary(hidden_child), Some(true));

    engine.clear_cache();
    assert_eq!(engine.layout_boundary(hidden_child), None);
}

#[test]
fn reference_engine_rejects_unsupported_layout_without_publishing_geometry() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let mut style = column(oxiview_style::Length::Auto, oxiview_style::Length::Auto);
    tree.set_style(root, style.clone()).unwrap();
    let mut engine = ColumnLayout::default();
    let viewport = Size::new(80.0, 60.0);
    tree.layout_with(&mut engine, viewport).unwrap();
    let before = tree.bounds(root).unwrap();
    style.layout.display = Display::Grid;
    tree.set_style(root, style).unwrap();
    assert_eq!(
        tree.layout_with(&mut engine, Size::new(120.0, 90.0)),
        Err(UiError::Layout(
            "invalid value for reference layout mode".into()
        ))
    );
    assert_eq!(tree.bounds(root).unwrap(), before);
}
