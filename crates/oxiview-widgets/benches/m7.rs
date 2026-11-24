//! Window-free M7 workloads against the production extent index and retained collections.
#![allow(clippy::expect_used)] // An invalid fixture invalidates its measurement.

use oxiview_base::{Point, Size};
use oxiview_core::{NodeId, UiTree, node};
use oxiview_layout::{VariableExtent, VirtualExtent};
use oxiview_style::{FlexDirection, LayoutStyle, Length, Overflow, Position, Style};
use oxiview_widgets::collection::{RowOrderChange, TableColumn, VirtualList, VirtualTable};

const VIEWPORT: Size = Size::new(384.0, 240.0);
const TABLE_WINDOW: Size = Size::new(384.0, 268.0);
const ROWS: u64 = 100_000;

fn main() {
    divan::main();
}

fn panel(tree: &mut UiTree, height: f64) -> NodeId {
    tree.append(
        tree.root(),
        Style {
            layout: LayoutStyle {
                position: Position::Relative,
                width: Length::Points(VIEWPORT.width()),
                height: Length::Points(height),
                flex_shrink: 0.0,
                ..LayoutStyle::default()
            },
            overflow: Overflow::Clip,
            ..Style::default()
        },
    )
    .expect("benchmark viewport")
}

#[divan::bench(sample_count = 80, sample_size = 128)]
fn m7_variable_extent_update_lookup_ns(bencher: divan::Bencher) {
    let mut rows =
        VariableExtent::new((0..ROWS).map(|row| 24.0 + (row % 5) as f64 * 4.0)).expect("index");
    let mut step = 0;
    bencher.bench_local(|| {
        step = (step + 1) % 20_000;
        let cursor = (step * 31) % 10_000;
        let height = if (step / 10_000) % 2 == 0 { 53.0 } else { 26.5 };
        rows.set_size(divan::black_box(cursor), height)
            .expect("measure row");
        divan::black_box(rows.visible_range(divan::black_box(2_456_780.5), VIEWPORT.height(), 2));
    });
}

#[divan::bench(sample_count = 50, sample_size = 8)]
fn m7_virtual_list_100k_advance_ns(bencher: divan::Bencher) {
    let mut tree = UiTree::new().expect("tree");
    let body = panel(&mut tree, VIEWPORT.height());
    let mut list = VirtualList::new(24.0, 2).expect("list");
    list.set_rows(0..ROWS).expect("rows");
    let mut offset = 4_800.0;
    tree.set_scroll_content_extent(body, Size::new(VIEWPORT.width(), list.total_height()))
        .expect("scroll extent");
    tree.layout(VIEWPORT).expect("layout viewport");
    tree.set_scroll_offset(body, Point::new(0.0, offset))
        .expect("initial offset");
    let first = list
        .reconcile(&mut tree, body, offset, VIEWPORT, |_| node())
        .expect("mount initial range");
    assert_eq!(first.mounted, 14);
    let mut large = VirtualList::new(24.0, 2).expect("large list");
    let mut larger_tree = UiTree::new().expect("large tree");
    let larger_body = panel(&mut larger_tree, VIEWPORT.height());
    large.set_rows(0..2 * ROWS).expect("large data source");
    assert_eq!(
        large
            .reconcile(&mut larger_tree, larger_body, offset, VIEWPORT, |_| node())
            .expect("large visible window")
            .mounted,
        first.mounted
    );
    drop(large);
    drop(larger_tree);
    let advance = |tree: &mut UiTree, list: &mut VirtualList, offset| {
        tree.set_scroll_offset(body, Point::new(0.0, offset))
            .expect("scroll list");
        let stats = list
            .reconcile(tree, body, offset, VIEWPORT, |_| node())
            .expect("advance range");
        tree.layout(VIEWPORT).expect("layout mounted rows");
        stats
    };
    offset += 24.0;
    let step = advance(&mut tree, &mut list, offset);
    assert_eq!((step.mounted, step.constructed, step.disposed), (14, 1, 1));
    let mut step = 24.0;
    bencher.bench_local(|| {
        if offset == 7_200.0 || offset == 4_800.0 {
            step = -step;
        }
        offset += step;
        divan::black_box(advance(&mut tree, &mut list, offset));
    });
}

#[divan::bench(sample_count = 50, sample_size = 4)]
fn m7_virtual_table_diagonal_ns(bencher: divan::Bencher) {
    let mut tree = UiTree::new().expect("tree");
    tree.set_style(
        tree.root(),
        Style {
            layout: LayoutStyle {
                flex_direction: FlexDirection::Column,
                ..LayoutStyle::default()
            },
            ..Style::default()
        },
    )
    .expect("stack heading and body");
    let header = panel(&mut tree, 28.0);
    let body = panel(&mut tree, VIEWPORT.height());
    let mut table = VirtualTable::new(24.0, 28.0, 1, 1).expect("table");
    table
        .set_rows(0..ROWS, 1, RowOrderChange::DataUpdate)
        .expect("rows");
    table
        .set_columns((0..2_000).map(|key| TableColumn { key, width: 96.0 }))
        .expect("columns");
    tree.set_scroll_content_extent(body, table.total_size())
        .expect("scroll extent");
    tree.layout(TABLE_WINDOW).expect("layout viewports");
    let mut index = 1_000;
    let mut step_diagonally = |index| {
        let offset = Point::new(f64::from(index) * 96.0, f64::from(index) * 24.0);
        tree.set_scroll_offset(body, offset).expect("scroll table");
        let stats = table
            .reconcile(&mut tree, body, header, offset, VIEWPORT, |_| node())
            .expect("advance table");
        tree.layout(TABLE_WINDOW).expect("layout mounted cells");
        stats
    };
    let first = step_diagonally(index);
    assert_eq!(
        (
            first.rows.mounted,
            first.headers.mounted,
            first.cells.mounted
        ),
        (12, 6, 72)
    );
    index += 1;
    let step = step_diagonally(index);
    assert_eq!((step.cells.constructed, step.cells.disposed), (17, 17));
    let mut step = 1;
    bencher.bench_local(|| {
        if index == 1_500 || index == 1_000 {
            step = -step;
        }
        index += step;
        divan::black_box(step_diagonally(index));
    });
}
