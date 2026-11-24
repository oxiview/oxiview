//! Seeded reader journey, geometry checks, and shared collection input helpers.

use super::{
    ReplayReport, Result, check,
    window::{Window, center, hit_within},
};
use crate::{app::ViewerApp, model::Column, view};
use oxiview_base::{Point, Rect};
use oxiview_platform::{
    KeyCode, KeyModifiers, ScrollMomentum, ScrollPhase, TextCommitKind, TextInputEdit,
};
use oxiview_widgets::collection::CellKey;

const SEED_ROW: u64 = 52_006;

pub(super) fn run(window: &Window, report: &mut ReplayReport) -> Result<()> {
    window.inspect(|app| {
        check(
            app.book.total() == 100_000 && app.first_visible_key() == Some(1),
            || "Seeded launch did not show the first of 100,000 records".into(),
        )?;
        list_mounts(app)
    })?;
    trackpad(window)?;
    report.passed("fractional-wheel-framework-momentum-and-thumb-interruption");

    let point = window.inspect(|app| Ok(center(body(app)?)))?;
    window.wheel(
        point,
        0.,
        f64::from((SEED_ROW - 1) as u32) * view::LIST_HEIGHT + 11.25,
    )?;
    let disclosure = window.inspect(|app| {
        list_mounts(app)?;
        check(app.first_visible_key() == Some(SEED_ROW), || {
            format!(
                "Distant scroll reached row {:?}, expected {SEED_ROW}",
                app.first_visible_key()
            )
        })?;
        let bounds = row(app, SEED_ROW)?;
        let visible = body(app)?;
        let clipped = Point::new(bounds.origin().x() + 12., visible.origin().y() - 1.);
        let live = Point::new(clipped.x(), visible.origin().y() + 6.);
        let node = app
            .list
            .row_node(SEED_ROW)
            .ok_or("Expected distant row to be mounted")?;
        check(
            bounds.origin().y() < clipped.y()
                && !hit_within(app, clipped, node)?
                && hit_within(app, live, node)?,
            || {
                format!(
                    "Fractional row clipping disagrees with hit testing at {clipped:?} and {live:?}"
                )
            },
        )?;
        Ok(live)
    })?;
    window.click(disclosure)?;
    let original_height = window.inspect(|app| {
        let height = row(app, SEED_ROW)?.size().height();
        check(
            app.active_key() == Some(SEED_ROW)
                && app.tree.focused_node() == Some(app.ui.collection.get()?)
                && height > view::LIST_HEIGHT,
            || {
                format!(
                    "Visible disclosure did not focus and expand row {SEED_ROW}, height {height}"
                )
            },
        )?;
        Ok(height)
    })?;

    window.resize(640, 720)?;
    let clipped = window.inspect(move |app| {
        let expanded = row(app, SEED_ROW)?;
        let bounds = body(app)?;
        let node = app.list.row_node(SEED_ROW).ok_or("Expected expanded row")?;
        let clipped = Point::new(expanded.origin().x() + 12., bounds.origin().y() - 1.);
        let visible = Point::new(clipped.x(), bounds.origin().y() + 4.);
        check(
            expanded.size().height() > original_height
                && expanded.origin().y() < clipped.y()
                && !hit_within(app, clipped, node)?
                && hit_within(app, visible, node)?,
            || format!("Narrow resize did not preserve the clipped expanded row and its hit region ({expanded:?}, prior height {original_height})"),
        )?;
        Ok(clipped)
    })?;
    window.click(clipped)?;
    window.inspect(move |app| {
        let height = row(app, SEED_ROW)?.size().height();
        check(
            height > original_height && app.active_key() == Some(SEED_ROW),
            || "A pointer outside the viewport interacted with the clipped expanded row".into(),
        )
    })?;
    report.capture(window, "expanded-list.png")?;
    report.passed("seeded-100k-bounded-mounts-expanded-row-clipping-and-remeasurement");

    window.control("Table", |app| app.ui.mode.node(1))?;
    let raw = window.inspect(|app| {
        table_mounts(app)?;
        check(app.is_table() && app.active_key() == Some(SEED_ROW), || {
            "Switching to the table lost the selected log record".into()
        })?;
        let index = app
            .book
            .index_of(SEED_ROW)
            .ok_or("Missing selected seed record")?;
        Ok(app
            .book
            .record_at(index)
            .ok_or("Missing seeded row")?
            .raw
            .clone())
    })?;
    window.click(window.inspect(|app| cell_point(app, cell(SEED_ROW, Column::Number)))?)?;
    window.copy(|| window.shortcut(KeyCode::KeyC), &raw)?;
    report.passed("table-mode-keeps-key-and-row-header-copies-raw");

    let query = format!("request {SEED_ROW:06x}");
    window.shortcut(KeyCode::KeyF)?;
    let base = window.input_snapshot(|snapshot| snapshot.text.is_empty())?;
    window.edit(
        &base,
        TextInputEdit::SetMarked {
            range: 0..0,
            text: query.clone(),
            selection: query.len()..query.len(),
        },
    )?;
    let marked = window.input_snapshot(|snapshot| {
        snapshot.session == base.session
            && snapshot.acknowledged == base.acknowledged + 1
            && snapshot.text == query
            && snapshot.marked == Some(0..query.len())
    })?;
    window.inspect(move |app| {
        let search = app.ui.search.node().ok_or("Missing search control")?;
        check(
            app.book.len() == 100_000
                && app.book.query().is_empty()
                && app.tree.focused_node() == Some(search)
                && contains(app.tree.bounds(search)?, center(marked.caret)),
            || "IME preedit filtered the stream or published its candidate outside the search field".into(),
        )
    })?;
    let marked = window.input_snapshot(|snapshot| snapshot.marked.is_some())?;
    window.edit(
        &marked,
        TextInputEdit::Commit {
            range: 0..query.len(),
            text: query.clone(),
            kind: TextCommitKind::Composition,
        },
    )?;
    window.input_snapshot(|snapshot| {
        snapshot.acknowledged == marked.acknowledged + 1
            && snapshot.marked.is_none()
            && snapshot.text == query
    })?;
    let severity = window.inspect(move |app| {
        let record = app
            .book
            .record_at(0)
            .ok_or("Committed search returned no rows")?;
        check(
            app.book.query() == query && app.book.len() == 1 && record.id == SEED_ROW,
            || format!("IME commit did not filter to the expected key {SEED_ROW}"),
        )?;
        Ok(record.severity)
    })?;
    window.control("Matching level", move |app| {
        app.ui.severity[severity as usize].node()
    })?;
    window.inspect(move |app| {
        check(
            app.book.is_empty()
                && !app.book.severity_enabled(severity)
                && app.active_key().is_none()
                && app.tree.focused_node() == app.ui.severity[severity as usize].node(),
            || "Severity filter failed to clear the selected result and retain focus".into(),
        )
    })?;
    window.key(KeyCode::Space, KeyModifiers::default())?;
    window.inspect(move |app| {
        check(
            app.book.len() == 1 && app.book.severity_enabled(severity),
            || "Keyboard Space did not re-enable the focused severity checkbox".into(),
        )
    })?;
    search(window, "")?;
    window.inspect(|app| {
        check(
            app.book.len() == 100_000 && app.book.query().is_empty(),
            || "Clearing search did not restore all seeded results".into(),
        )
    })?;
    report.passed("ime-candidate-preedit-commit-severity-keyboard-and-clear");
    Ok(())
}

fn trackpad(window: &Window) -> Result<()> {
    let point = window.inspect(|app| Ok(center(body(app)?)))?;
    window.wheel(point, 0., 3.5)?;
    window.inspect(|app| {
        let y = offset(app)?.y();
        check(y == 3.5, || {
            format!("Fractional wheel delivered offset {y} instead of 3.5")
        })
    })?;
    window.scroll(point, 0., ScrollPhase::Begin(ScrollMomentum::Framework))?;
    window.advance()?;
    window.scroll(point, 24., ScrollPhase::Update)?;
    window.advance()?;
    window.scroll(point, 24., ScrollPhase::Update)?;
    window.advance()?;
    window.scroll(point, 0., ScrollPhase::End)?;
    let direct = window.inspect(|app| Ok(offset(app)?.y()))?;
    window.advance()?;
    let thumb = window.inspect(move |app| {
        let y = offset(app)?.y();
        check(app.tree.scroll_animation_active() && y > direct, || {
            format!("Framework momentum did not advance on a virtual frame ({direct} to {y})")
        })?;
        let node = app
            .ui
            .vertical
            .thumb_node()
            .ok_or("Missing vertical thumb")?;
        let point = center(app.tree.bounds(node)?);
        let track = app
            .ui
            .vertical
            .node()
            .ok_or("Missing vertical scrollbar track")?;
        check(hit_within(app, point, track)?, || {
            "Vertical thumb center did not reach its capture track".into()
        })?;
        Ok(point)
    })?;
    window.button(true, thumb)?;
    let stopped = window.inspect(|app| {
        check(!app.tree.scroll_animation_active(), || {
            "Thumb press did not interrupt momentum".into()
        })?;
        offset(app)
    })?;
    window.advance()?;
    window.advance()?;
    window.button(false, thumb)?;
    window.inspect(move |app| {
        check(offset(app)? == stopped, || {
            "Interrupted momentum continued to scroll".into()
        })
    })?;
    window.wheel(point, 0., -1_000.)?;
    window.inspect(|app| {
        check(offset(app)?.y() == 0., || {
            "Discrete wheel did not return to the top".into()
        })
    })
}

fn list_mounts(app: &ViewerApp) -> Result<()> {
    let viewport = body(app)?;
    let visible = app
        .list
        .visible_rows(offset(app)?.y(), viewport.size().height());
    let mounted = app.tree.children(app.ui.collection.get()?)?.len();
    let all_visible = visible.clone().all(|index| {
        app.list
            .key_at(index)
            .and_then(|key| app.list.row_node(key))
            .is_some()
    });
    check(
        !visible.is_empty()
            && all_visible
            && mounted == app.list.mounted_rows().count()
            && mounted <= visible.len() + 6,
        || {
            format!(
                "List mounted {mounted} rows for visible range {visible:?}, including at most six overscan rows"
            )
        },
    )
}

pub(super) fn table_mounts(app: &ViewerApp) -> Result<()> {
    let viewport = body(app)?;
    let offset = offset(app)?;
    let rows = app.table.visible_rows(offset.y(), viewport.size().height());
    let columns = app
        .table
        .visible_columns(offset.x(), viewport.size().width());
    let cells: Vec<_> = app.table.mounted_cells().map(|(key, _)| key).collect();
    let mounted_rows = app.tree.children(app.ui.collection.get()?)?.len();
    let mounted_headers = app.tree.children(app.ui.header.get()?)?.len();
    let all_visible = rows.clone().all(|r| {
        columns.clone().all(|c| {
            app.table
                .row_key(r)
                .zip(app.table.column_key(c))
                .is_some_and(|(row, column)| cells.contains(&CellKey { row, column }))
        })
    });
    let distinct_rows = cells
        .iter()
        .map(|key| key.row)
        .collect::<std::collections::HashSet<_>>()
        .len();
    check(
        !rows.is_empty()
            && !columns.is_empty()
            && all_visible
            && distinct_rows == mounted_rows
            && mounted_rows <= rows.len() + 6
            && mounted_headers == app.table.mounted_headers().count()
            && mounted_headers <= columns.len() + 2
            && cells.len() <= (rows.len() + 6) * (columns.len() + 2),
        || {
            format!(
                "Table mounted {} rows, {} columns and {} cells for visible rows {rows:?} and columns {columns:?}",
                distinct_rows,
                mounted_headers,
                cells.len()
            )
        },
    )
}

pub(super) fn body(app: &ViewerApp) -> Result<Rect> {
    Ok(app.tree.scroll_viewport_bounds(app.ui.collection.get()?)?)
}

pub(super) fn offset(app: &ViewerApp) -> Result<Point> {
    Ok(app.tree.scroll_offset(app.ui.collection.get()?)?)
}

fn row(app: &ViewerApp, key: u64) -> Result<Rect> {
    Ok(app.tree.bounds(
        app.list
            .row_node(key)
            .ok_or_else(|| format!("Row {key} is unmounted"))?,
    )?)
}

pub(super) fn cell(row: u64, column: Column) -> CellKey {
    CellKey {
        row,
        column: column as u64,
    }
}

pub(super) fn cell_point(app: &ViewerApp, key: CellKey) -> Result<Point> {
    let node = app
        .table
        .cell_node(key)
        .ok_or_else(|| format!("Cell {key:?} is unmounted"))?;
    let rect = app.tree.bounds(node)?;
    let [left, top, right, bottom] = body(app)?.edges();
    let [x0, y0, x1, y1] = rect.edges();
    let point = Point::new(
        (x0.max(left) + x1.min(right)) * 0.5,
        (y0.max(top) + y1.min(bottom)) * 0.5,
    );
    check(hit_within(app, point, node)?, || {
        format!("Cell {key:?} is clipped or covered at {point:?} ({rect:?})")
    })?;
    Ok(point)
}

fn contains(rect: Rect, point: Point) -> bool {
    let [left, top, right, bottom] = rect.edges();
    (left..right).contains(&point.x()) && (top..bottom).contains(&point.y())
}

pub(super) fn search(window: &Window, value: &str) -> Result<()> {
    window.shortcut(KeyCode::KeyF)?;
    let snapshot = window.input_snapshot(|_| true)?;
    window.edit(
        &snapshot,
        TextInputEdit::Commit {
            range: 0..snapshot.text.len(),
            text: value.into(),
            kind: TextCommitKind::Typing,
        },
    )?;
    window.input_snapshot(|next| {
        next.session == snapshot.session
            && next.acknowledged == snapshot.acknowledged + 1
            && next.text == value
    })?;
    Ok(())
}
