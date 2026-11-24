use super::*;
use oxiview_base::{Point, Rect, Size};
use oxiview_core::{UiTree, component, node};
use oxiview_style::{FlexDirection, LayoutStyle, Length, Overflow, Position, Style};
use std::borrow::Cow;

fn positioned(width: f64, height: f64) -> Style {
    Style {
        layout: LayoutStyle {
            position: Position::Relative,
            width: Length::Points(width),
            height: Length::Points(height),
            flex_shrink: 0.0,
            ..LayoutStyle::default()
        },
        overflow: Overflow::Clip,
        ..Style::default()
    }
}

#[test]
fn measuring_and_reordering_unmounted_rows_restores_the_keyed_anchor() -> Result<(), CollectionError>
{
    let mut list = VirtualList::new(20.0, 2)?;
    list.set_rows(10..100_010)?;
    let anchor = list
        .anchor(1_000_004.5)
        .ok_or(CollectionError::UnknownKey(0))?;
    assert_eq!(
        anchor,
        ListAnchor {
            key: 50_010,
            intra_row: 4.5
        }
    );
    list.set_height(15, 87.5)?;
    assert_eq!(list.offset_for_anchor(anchor), Some(1_000_072.0));
    list.set_rows((10..100_010).rev())?;
    assert_eq!(
        list.height(list.index_of(15).unwrap_or_default()),
        Some(87.5)
    );
    assert_eq!(
        list.offset_for_anchor(anchor)
            .and_then(|offset| list.anchor(offset)),
        Some(anchor)
    );
    list.reset_heights(16.0)?;
    assert_eq!(list.offset_for_anchor(anchor), Some(799_988.5));
    list.set_height(anchor.key, 3.0)?;
    assert_eq!(
        list.offset_for_anchor(anchor)
            .and_then(|offset| list.anchor(offset))
            .map(|anchor| anchor.key),
        Some(anchor.key)
    );
    list.append_rows(200_000..200_002)?;
    assert_eq!(list.key_at(100_001), Some(200_001));
    Ok(())
}

#[test]
fn list_retains_intersection_and_bounds_construction_at_either_data_size()
-> Result<(), CollectionError> {
    let mut counts = Vec::new();
    let mut layout_work = Vec::new();
    for size in [100_000, 200_000] {
        let mut tree = UiTree::new()?;
        let root = tree.root();
        let viewport = Size::new(200.0, 100.0);
        let parent = tree.append(root, positioned(200.0, 100.0))?;
        tree.layout(viewport)?;
        let mut list = VirtualList::new(20.0, 2)?;
        list.set_rows(1..=size)?;
        let mut built = 0;
        let first = list.reconcile(&mut tree, parent, 20_000.0, viewport, |_| {
            built += 1;
            node()
        })?;
        assert_eq!(first.mounted, 9);
        assert_eq!(first.constructed, 9);
        let shared = list
            .row_node(1003)
            .ok_or(CollectionError::UnknownKey(1003))?;
        let departed = list.row_node(999).ok_or(CollectionError::UnknownKey(999))?;
        let before = tree.diagnostics();
        let next = list.reconcile(&mut tree, parent, 20_020.0, viewport, |_| {
            built += 1;
            node()
        })?;
        assert_eq!((next.mounted, next.constructed, next.disposed), (9, 1, 1));
        assert_eq!(list.row_node(1003), Some(shared));
        assert!(!tree.contains(departed));
        tree.layout(viewport)?;
        let after = tree.diagnostics();
        assert_eq!(after.layout_passes - before.layout_passes, 1);
        layout_work.push(after.computed_layout_nodes - before.computed_layout_nodes);
        assert_eq!(
            tree.bounds(shared)?,
            Rect::new(0.0, 20_040.0, 200.0, 20_060.0)
        );
        let tail = list.total_height() - 54.5;
        assert_eq!(
            list.visible_rows(tail, 100.0),
            (size as usize - 3)..size as usize
        );
        let jump = list.reconcile(&mut tree, parent, tail, viewport, |_| {
            built += 1;
            node()
        })?;
        assert_eq!((jump.mounted, jump.constructed, jump.disposed), (5, 5, 9));
        assert_eq!(tree.len(), 12);
        assert!(list.row_node(size).is_some());
        counts.push(built);
        list.set_rows([])?;
        assert_eq!(
            list.reconcile(&mut tree, parent, 0.0, viewport, |_| node())?
                .mounted,
            0
        );
        list.set_rows([size])?;
        assert_eq!(
            list.reconcile(&mut tree, parent, 0.0, viewport, |_| node())?
                .mounted,
            1
        );
        list.clear(&mut tree)?;
        assert_eq!(tree.len(), 2);
    }
    assert_eq!(counts, [15, 15]);
    assert!(layout_work.first().is_some_and(|count| *count > 0));
    assert_eq!(layout_work.first(), layout_work.last());
    Ok(())
}

#[test]
fn failed_build_releases_partial_roots_and_can_retry() -> Result<(), CollectionError> {
    let mut tree = UiTree::new()?;
    let parent = tree.root();
    let mut list = VirtualList::new(24.0, 0)?;
    list.set_rows([1, 2, 3])?;
    let viewport = Size::new(100.0, 100.0);
    let failed = list.reconcile(&mut tree, parent, 0.0, viewport, |row| {
        if row.key == 2 {
            component(|| Err(oxiview_core::UiError::DepthLimit))
        } else {
            node()
        }
    });
    assert!(matches!(
        failed,
        Err(CollectionError::Ui(oxiview_core::UiError::DepthLimit))
    ));
    assert_eq!(tree.len(), 1);
    assert_eq!(list.mounted_rows().count(), 0);
    assert_eq!(
        list.reconcile(&mut tree, parent, 0.0, viewport, |_| node())?
            .mounted,
        3
    );
    Ok(())
}

#[test]
fn table_copies_the_unmounted_rectangle_and_applies_revision_rules() -> Result<(), CollectionError>
{
    let mut table = VirtualTable::new(24.0, 28.0, 1, 1)?;
    table.set_columns([10, 20, 30].map(|key| TableColumn { key, width: 150.0 }))?;
    table.set_rows([7, 2, 8, 6], 17, RowOrderChange::DataUpdate)?;
    let active = CellKey { row: 2, column: 20 };
    let anchor = CellKey { row: 8, column: 30 };
    table.select(anchor, false);
    table.select(active, true);
    assert_eq!(table.selection(), Some(CellSelection { anchor, active }));
    assert_eq!(table.cell_node(active), None);
    assert!(table.is_selected(CellKey { row: 8, column: 20 }));
    assert!(!table.is_selected(CellKey { row: 7, column: 30 }));
    let expected = "\"two\tvalue\"\t\"quoted \"\"cell\"\"\"\n\"line\nbreak\"\t\"with\rreturn\"";
    let copy = table.copy_tsv(|revision, cell| {
        assert_eq!(revision, 17);
        Cow::Borrowed(match (cell.row, cell.column) {
            (2, 20) => "two\tvalue",
            (2, 30) => "quoted \"cell\"",
            (8, 20) => "line\nbreak",
            _ => "with\rreturn",
        })
    });
    assert_eq!(copy.as_deref(), Some(expected));
    table.append_rows([9], 18)?;
    table.resize_column(20, 110.5)?;
    assert_eq!(table.selection(), Some(CellSelection { anchor, active }));
    table.set_rows([6, 2, 8, 9, 7], 19, RowOrderChange::DataUpdate)?;
    assert_eq!(table.selection(), Some(CellSelection { anchor, active }));
    assert_eq!(
        table
            .copy_tsv(|revision, cell| Cow::Owned(format!("{revision}/{}", cell.row)))
            .as_deref(),
        Some("19/2\t19/2\n19/8\t19/8")
    );
    table.set_rows([6, 2, 7, 8, 9], 20, RowOrderChange::DataUpdate)?;
    assert_eq!(
        table.selection(),
        Some(CellSelection {
            anchor: active,
            active
        })
    );
    table.select(anchor, true);
    table.set_rows([8, 6, 2, 7, 9], 21, RowOrderChange::SortOrFilter)?;
    assert_eq!(
        table.selection(),
        Some(CellSelection {
            anchor,
            active: anchor
        })
    );
    table.set_rows([2, 7, 9], 22, RowOrderChange::SortOrFilter)?;
    assert_eq!(table.selection(), None);
    assert_eq!(table.copy_tsv(|_, _| Cow::Borrowed("")), None);
    Ok(())
}

#[test]
fn diagonal_table_window_reuses_intersection_and_aligns_headings() -> Result<(), CollectionError> {
    let mut tree = UiTree::new()?;
    let root = tree.root();
    tree.set_style(
        root,
        Style {
            layout: LayoutStyle {
                flex_direction: FlexDirection::Column,
                ..LayoutStyle::default()
            },
            ..Style::default()
        },
    )?;
    let header = tree.append(root, positioned(300.0, 28.0))?;
    let body = tree.append(root, positioned(300.0, 96.0))?;
    let mut table = VirtualTable::new(24.0, 28.0, 1, 1)?;
    table.set_rows(0..100_000, 1, RowOrderChange::DataUpdate)?;
    table.set_columns((0..2_000).map(|key| TableColumn { key, width: 100.0 }))?;
    let viewport = Size::new(300.0, 96.0);
    let window = Size::new(300.0, 124.0);
    tree.set_scroll_content_extent(body, table.total_size())?;
    tree.layout(window)?;
    tree.set_scroll_offset(body, Point::new(1_000.0, 2_400.0))?;
    let first = table.reconcile(
        &mut tree,
        body,
        header,
        Point::new(1_000.0, 2_400.0),
        viewport,
        |_| node(),
    )?;
    assert_eq!(
        (
            first.rows.mounted,
            first.headers.mounted,
            first.cells.mounted
        ),
        (6, 5, 30)
    );
    let key = CellKey {
        row: 102,
        column: 12,
    };
    let shared = table
        .cell_node(key)
        .ok_or(CollectionError::UnknownKey(102))?;
    let title = table
        .header_node(12)
        .ok_or(CollectionError::UnknownKey(12))?;
    let removed = table
        .cell_node(CellKey { row: 99, column: 9 })
        .ok_or(CollectionError::UnknownKey(99))?;
    table.select(key, false);
    let dest = table
        .navigate(99_997, 1_987, true)
        .ok_or(CollectionError::UnknownKey(0))?;
    let reveal = table.reveal(dest, Point::default(), viewport);
    assert_eq!(reveal, Some(Point::new(199_700.0, 2_399_904.0)));
    let offset = Point::new(1_100.0, 2_424.0);
    tree.set_scroll_offset(body, offset)?;
    let step = table.reconcile(&mut tree, body, header, offset, viewport, |_| node())?;
    assert_eq!(
        (
            step.rows.constructed,
            step.headers.constructed,
            step.cells.constructed,
            step.cells.disposed
        ),
        (1, 1, 10, 10)
    );
    assert_eq!(table.cell_node(key), Some(shared));
    assert_eq!(table.header_node(12), Some(title));
    assert!(!tree.contains(removed));
    assert!(table.is_selected(key));
    tree.layout(window)?;
    let body_bounds = tree.bounds(body)?;
    let cell = tree.bounds(shared)?;
    let header_bounds = tree.bounds(header)?;
    let heading = tree.bounds(title)?;
    assert_eq!(
        cell.origin().x() - body_bounds.origin().x(),
        heading.origin().x() - header_bounds.origin().x()
    );
    assert_eq!(cell.size(), Size::new(100.0, 24.0));
    assert_eq!(cell.origin().y() - body_bounds.origin().y(), 24.0);
    assert_eq!(heading.origin().y(), header_bounds.origin().y());
    assert_eq!(heading.size(), Size::new(100.0, 28.0));
    table.resize_column(11, 140.0)?;
    tree.set_scroll_content_extent(body, table.total_size())?;
    table.reconcile(&mut tree, body, header, offset, viewport, |_| node())?;
    tree.layout(window)?;
    assert_eq!(
        tree.bounds(shared)?.origin().x() - body_bounds.origin().x(),
        140.0
    );
    assert_eq!(
        tree.bounds(title)?.origin().x() - header_bounds.origin().x(),
        140.0
    );
    assert_eq!(table.selection().map(|s| s.active), Some(dest));
    table.clear(&mut tree)?;
    assert_eq!(tree.len(), 3);
    Ok(())
}
