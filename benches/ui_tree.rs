//! Retained UI benchmarks. Each mutation sample starts from the same tree state.
#![allow(clippy::expect_used)]

use oxiview_base::{Color, Point, Size};
use oxiview_core::{NodeId, UiTree};
use oxiview_style::{Length, Position, Style};

const VIEWPORT: Size = Size::new(720.0, 480.0);
const ROWS: u32 = 24;
const COLUMNS: u32 = 40;

fn main() {
    divan::main();
}

fn fixed(width: f64, height: f64) -> Style {
    let mut style = Style::default();
    style.layout.width = Length::Points(width);
    style.layout.height = Length::Points(height);
    style
}

struct PreparedTree {
    tree: UiTree,
    first_row: NodeId,
    last_row: NodeId,
    first_leaf: NodeId,
}

fn prepare() -> PreparedTree {
    let mut tree = UiTree::new().expect("root");
    let root = tree.root();
    let mut first_row = root;
    let mut last_row = root;
    let mut first_leaf = root;
    for y in 0..ROWS {
        // Each fixed, out-of-flow row is independent of its siblings.
        let mut row = fixed(VIEWPORT.width(), 20.0);
        row.layout.position = Position::Absolute;
        row.layout.inset.top = Length::Points(f64::from(y) * 20.0);
        let row = tree.append(root, row).expect("row");
        last_row = row;
        if y == 0 {
            first_row = row;
        }
        for x in 0..COLUMNS {
            let mut tile = fixed(16.0, 16.0);
            tile.background = Color::from_rgba8(48, 92, 160, 255);
            let tile = tree.append(row, tile).expect("tile");
            if x == 0 && y == 0 {
                first_leaf = tile;
            }
        }
    }
    tree.paint(VIEWPORT, 1.0).expect("warm preparation");
    PreparedTree {
        tree,
        first_row,
        last_row,
        first_leaf,
    }
}

#[divan::bench(sample_count = 100, sample_size = 10)]
fn arena_edits(bencher: divan::Bencher) {
    let PreparedTree {
        mut tree,
        first_row,
        last_row,
        ..
    } = prepare();
    // Warm the spare slot before measuring steady append/move/remove work.
    let spare = tree.append(first_row, fixed(16.0, 16.0)).expect("spare");
    tree.remove(spare).expect("spare removal");
    bencher
        .with_inputs(|| fixed(16.0, 16.0))
        .bench_local_values(|style| {
            let id = tree
                .append(divan::black_box(first_row), style)
                .expect("append");
            tree.reparent(id, last_row, 0).expect("cross-row move");
            tree.remove(id).expect("remove");
            id
        });
}

#[divan::bench(sample_count = 100, sample_size = 1000)]
fn clean_preparation(bencher: divan::Bencher) {
    let mut tree = prepare().tree;
    bencher.bench_local(|| {
        tree.paint(divan::black_box(VIEWPORT), 1.0)
            .expect("cached scene")
    });
}

#[divan::bench(sample_count = 40, sample_size = 1)]
fn contained_layout_change(bencher: divan::Bencher) {
    bencher
        .with_inputs(|| {
            let prepared = prepare();
            let mut replacement = prepared
                .tree
                .style(prepared.first_leaf)
                .expect("tile style")
                .clone();
            replacement.layout.height = Length::Points(18.0);
            (prepared, Some(replacement))
        })
        .bench_local_refs(|(prepared, replacement)| {
            let id = prepared.first_leaf;
            let style = replacement.take().expect("prepared style");
            prepared.tree.set_style(id, style).expect("layout edit");
            prepared
                .tree
                .layout(divan::black_box(VIEWPORT))
                .expect("contained layout")
        });
}

#[divan::bench(sample_count = 100, sample_size = 100)]
fn paint_only_update(bencher: divan::Bencher) {
    let mut fixture = prepare();
    let leaf = fixture.first_leaf;
    let original = fixture.tree.style(leaf).expect("tile style").clone();
    let mut highlighted = original.clone();
    highlighted.background = Color::from_rgba8(192, 72, 40, 255);
    let mut highlight = false;
    bencher
        .with_inputs(|| {
            highlight = !highlight;
            if highlight {
                highlighted.clone()
            } else {
                original.clone()
            }
        })
        .bench_local_values(|style| {
            assert!(fixture.tree.set_style(leaf, style).expect("paint edit"));
            // A submitted frame has released its Scene before the next edit.
            divan::black_box(fixture.tree.paint(VIEWPORT, 1.0).expect("repaint"));
        });
}

#[divan::bench(sample_count = 100, sample_size = 10)]
fn hit_testing(bencher: divan::Bencher) {
    let PreparedTree {
        tree, first_leaf, ..
    } = prepare();
    // The first leaf is deep in the reverse traversal of final paint order.
    let point = Point::new(8.0, 8.0);
    assert_eq!(tree.hit_test(point).expect("fixture hit"), Some(first_leaf));
    bencher.bench_local(|| tree.hit_test(divan::black_box(point)).expect("hit test"));
}
