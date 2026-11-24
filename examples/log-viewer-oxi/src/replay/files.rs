//! Real file changes and expected clipboard bytes driven through the viewer's platform ports.

use super::{
    ReplayReport, Result, check,
    collection::{body, cell, cell_point, offset, search, table_mounts},
    window::{Window, center, hit_within},
};
use crate::{
    app::ViewerApp,
    model::{Column, Severity},
};
use oxiview_base::Point;
use oxiview_platform::{KeyCode, KeyModifiers};
use oxiview_widgets::collection::CellSelection;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const FILE_ROWS: usize = 96;
const RANGE_START: usize = 5;
const RANGE_END: usize = 45;
const LONG_ROW: usize = 80;

pub(super) struct Fixture {
    directory: PathBuf,
    stream: PathBuf,
    other: PathBuf,
    rows: Vec<Line>,
}

impl Fixture {
    pub(super) fn new(output: &Path) -> Result<Self> {
        let directory = output.join(".log-fixture");
        fs::create_dir(&directory)?;
        let stream = directory.join("stream.log");
        let other = directory.join("other.log");
        let rows: Vec<_> = (0..FILE_ROWS).map(Line::new).collect();
        let mut file = super::new_file(&stream)?;
        for row in &rows {
            writeln!(file, "{}", row.raw)?;
        }
        fs::write(&other, "[INFO] new/worker: alternate source\n")?;
        Ok(Self {
            directory,
            stream,
            other,
            rows,
        })
    }

    pub(super) fn remove(self) -> Result<()> {
        fs::remove_dir_all(self.directory)?;
        Ok(())
    }
}

struct Line {
    time: String,
    level: &'static str,
    source: &'static str,
    thread: &'static str,
    message: String,
    raw: String,
}

impl Line {
    fn new(index: usize) -> Self {
        let time = format!("11:42:{:02}.{:03}", index % 60, index);
        let level = ["WARN", "ERROR", "DEBUG", "INFO"][index % 4];
        let source = "gateway";
        let thread = "worker1";
        let message = match index {
            RANGE_END => format!("survivor {index}\twith \"quoted\" text\rand a carriage return"),
            LONG_ROW => "A long message that wraps and overflows the inline detail. ".repeat(70),
            _ => format!("record {index}"),
        };
        let raw = format!("{time} [{level}] {source}/{thread}: {message}");
        Self {
            time,
            level,
            source,
            thread,
            message,
            raw,
        }
    }

    fn tsv(&self) -> String {
        // These fixtures target the exact exported columns and quoting independently of LogBook.
        let message = if self.message.contains(['\t', '"', '\r']) {
            format!("\"{}\"", self.message.replace('"', "\"\""))
        } else {
            self.message.clone()
        };
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            self.time,
            self.level,
            self.source,
            message,
            self.thread,
            self.raw.len()
        )
    }
}

pub(super) fn run(window: &Window, fixture: &Fixture, report: &mut ReplayReport) -> Result<()> {
    window.open(Ok(Some(fixture.stream.clone())))?;
    let tail = wait_tail(
        window,
        "real log Open",
        "stream.log",
        FILE_ROWS,
        "record 95",
    )?;
    window.inspect(|app| {
        check(
            app.tree
                .text_source(app.ui.status.get()?)?
                .starts_with("Opened ")
                && app.book.record_at(0).is_some_and(|line| line.id > 100_000),
            || "Open did not publish a fresh source and clear its opening status".into(),
        )
    })?;

    append(&fixture.stream, b"[INFO] gateway/worker1: live-tail\n")?;
    let new_tail = wait_tail(
        window,
        "watcher append while following",
        "stream.log",
        FILE_ROWS + 1,
        "live-tail",
    )?;
    check(new_tail > tail, || {
        "A new complete line reused the previous tail identity".into()
    })?;
    window.wheel(window.inspect(|app| Ok(center(body(app)?)))?, 0., -100_000.)?;
    window.inspect(|app| {
        check(!app.book.follow() && offset(app)?.y() == 0., || {
            "Scrolling to the beginning did not pause Follow".into()
        })
    })?;
    report.passed("native-open-watch-append-follow-and-pause");

    rectangle(window, fixture, report)?;
    follow(window, fixture, report)?;
    nested_scroll(window, report)?;
    sources(window, fixture, report)?;
    report.capture(window, "switched-source.png")?;
    Ok(())
}

fn rectangle(window: &Window, fixture: &Fixture, report: &mut ReplayReport) -> Result<()> {
    let (start, end) = window.inspect(|app| {
        let key = |index| {
            app.book
                .record_at(index)
                .map(|row| row.id)
                .ok_or("Fixture row missing")
        };
        Ok((key(RANGE_START)?, key(RANGE_END)?))
    })?;
    let anchor = cell(start, Column::Time);
    let active = cell(end, Column::Bytes);
    let selection = CellSelection { anchor, active };
    window.click(window.inspect(move |app| cell_point(app, anchor))?)?;
    window.inspect(move |app| {
        check(
            app.table.selection()
                == Some(CellSelection {
                    anchor,
                    active: anchor,
                })
                && app.tree.focused_node() == Some(app.ui.collection.get()?)
                && app.table.cell_node(cell(start, Column::Bytes)).is_none(),
            || {
                "Initial Time cell did not focus the table or the far columns were already mounted"
                    .into()
            },
        )
    })?;
    let shift = KeyModifiers {
        shift: true,
        ..KeyModifiers::default()
    };
    for _ in RANGE_START..RANGE_END {
        window.key(KeyCode::ArrowDown, shift)?;
    }
    for _ in (Column::Time as u64)..(Column::Bytes as u64) {
        window.key(KeyCode::ArrowRight, shift)?;
    }
    let reading_position = window.inspect(move |app| {
        table_mounts(app)?;
        check(
            app.table.selection() == Some(selection)
                && app.table.cell_node(anchor).is_none()
                && app.table.cell_node(cell(end, Column::Time)).is_none()
                && cell_point(app, active).is_ok(),
            || format!("Shift range did not track the visible active cell across recycled rows and columns: {:?}", app.table.selection()),
        )?;
        let body = body(app)?;
        let header = app.tree.bounds(app.ui.header.get()?)?;
        let node = app.table.header_node(Column::Bytes as u64).ok_or("Missing visible Bytes heading")?;
        let cell_bounds = app.tree.bounds(app.table.cell_node(active).ok_or("Active cell missing")?)?;
        let heading_bounds = app.tree.bounds(node)?;
        check(
            (heading_bounds.origin().x() - cell_bounds.origin().x()).abs() < 0.01
                && header.edges()[3] <= body.origin().y()
                && hit_within(app, center(heading_bounds), node)?,
            || format!("Pinned heading did not stay aligned and hittable while scrolling: {heading_bounds:?} and {cell_bounds:?}"),
        )?;
        Ok((app.first_visible_key(), offset(app)?))
    })?;
    let tsv = fixture.rows[RANGE_START..=RANGE_END]
        .iter()
        .map(Line::tsv)
        .collect::<Vec<_>>()
        .join("\n");
    window.copy(|| window.shortcut(KeyCode::KeyC), &tsv)?;

    append(
        &fixture.stream,
        b"[WARN] gateway/worker1: arrived while selecting\n",
    )?;
    let appended = wait_book(window, "append with offscreen selection", move |app| {
        Ok((app.book.total() == FILE_ROWS + 2
            && app
                .book
                .record_at(FILE_ROWS + 1)
                .is_some_and(|row| row.message == "arrived while selecting"))
        .then_some((
            app.first_visible_key(),
            offset(app)?,
            app.table.selection(),
            app.book.unseen(),
        )))
    })?;
    check(
        appended == (reading_position.0, reading_position.1, Some(selection), 1),
        || format!("The append moved the paused viewport or changed the keyed range: {appended:?}"),
    )?;
    window.copy(|| window.shortcut(KeyCode::KeyC), &tsv)?;
    report.capture(window, "offscreen-table-range.png")?;

    drag_horizontal_home(window)?;
    let (grip, width) = window.inspect(|app| {
        let level = app
            .table
            .header_node(Column::Level as u64)
            .ok_or("Level heading missing")?;
        let rect = app.tree.bounds(level)?;
        let grip = Point::new(rect.edges()[2] - 2., center(rect).y());
        check(hit_within(app, grip, level)?, || {
            "Level resize grip was not hittable".into()
        })?;
        Ok((grip, rect.size().width()))
    })?;
    window.drag(grip, Point::new(grip.x() + 36., grip.y()))?;
    window.inspect(move |app| {
        let updated = app.table.column_width(Column::Level as usize).ok_or("Missing Level width")?;
        check(app.table.selection() == Some(selection) && (updated - width - 36.).abs() < 0.01, || {
            format!("Column resize did not change width by 36 or preserve the range: {width} to {updated}")
        })
    })?;
    window.copy(|| window.control("Copy", |app| app.ui.copy.node()), &tsv)?;

    let level = window.inspect(|app| {
        let node = app
            .table
            .header_node(Column::Level as u64)
            .ok_or("Missing Level heading")?;
        Ok(center(app.tree.bounds(node)?))
    })?;
    window.click(level)?;
    window.inspect(move |app| {
        let sorted = (0..app.book.len())
            .filter_map(|index| app.book.record_at(index).map(|row| row.severity))
            .is_sorted();
        check(
            sorted
                && app.book.sort() == Some((Column::Level, true))
                && app
                    .book
                    .record_at(0)
                    .is_some_and(|row| row.severity == Severity::Debug)
                && app.table.selection()
                    == Some(CellSelection {
                        anchor: active,
                        active,
                    })
                && !app.book.follow(),
            || {
                format!(
                    "Sorting did not order severity and collapse to the active cell: {:?}",
                    app.table.selection()
                )
            },
        )
    })?;
    let bytes = fixture.rows[RANGE_END].raw.len().to_string();
    window.copy(|| window.shortcut(KeyCode::KeyC), &bytes)?;
    report.passed("row-and-column-recycling-exact-tsv-append-resize-and-sort");

    window.click(window.inspect(move |app| cell_point(app, active))?)?;
    window.key(KeyCode::ArrowLeft, shift)?;
    let thread = cell(end, Column::Thread);
    window.inspect(move |app| {
        check(
            app.table.selection()
                == Some(CellSelection {
                    anchor: active,
                    active: thread,
                }),
            || "The table did not extend the range left from the sorted active cell".into(),
        )
    })?;
    search(window, "survivor")?;
    window.inspect(move |app| {
        check(
            app.book.len() == 1
                && app.table.selection()
                    == Some(CellSelection {
                        anchor: thread,
                        active: thread,
                    })
                && app.tree.focused_node() == app.ui.search.node(),
            || {
                "Filtering did not collapse the range to the surviving cell or retain editor focus"
                    .into()
            },
        )
    })?;
    window.copy(
        || window.control("Copy", |app| app.ui.copy.node()),
        fixture.rows[RANGE_END].thread,
    )?;
    search(window, "no such fixture record")?;
    window.inspect(|app| {
        let copy = app.ui.copy.node().ok_or("Missing Copy")?;
        check(
            app.book.is_empty()
                && app.table.selection().is_none()
                && app.table.mounted_cells().next().is_none()
                && app.tree.focused_node() == app.ui.search.node()
                && app.tree.interaction_state(copy)?.disabled,
            || "Empty result left a selection, mounted cell, or enabled Copy".into(),
        )
    })?;
    search(window, "")?;
    report.passed("filter-collapses-to-surviving-cell-clears-removed-key-and-disables-copy");
    Ok(())
}

fn drag_horizontal_home(window: &Window) -> Result<()> {
    let thumb = window.inspect(|app| {
        let node = app
            .ui
            .horizontal
            .thumb_node()
            .ok_or("Missing horizontal scrollbar thumb")?;
        let point = center(app.tree.bounds(node)?);
        let track = app
            .ui
            .horizontal
            .node()
            .ok_or("Missing horizontal scrollbar track")?;
        check(hit_within(app, point, track)?, || {
            "Horizontal thumb center did not reach its capture track".into()
        })?;
        Ok(point)
    })?;
    window.drag(thumb, Point::new(0., thumb.y()))?;
    window.inspect(|app| {
        check(offset(app)?.x() == 0., || {
            "Captured horizontal drag did not return to the leading columns".into()
        })
    })
}

fn follow(window: &Window, fixture: &Fixture, report: &mut ReplayReport) -> Result<()> {
    let point = window.inspect(|app| Ok(center(body(app)?)))?;
    window.wheel(point, 0., 100_000.)?;
    window.inspect(|app| {
        check(
            !app.book.follow() && app.book.unseen() == 1 && offset(app)?.y() == maximum(app)?.y(),
            || "Manually reaching the bottom should preserve the pause and notice".into(),
        )
    })?;
    window.control("New record notice", |app| app.ui.resume.node())?;
    wait_tail(
        window,
        "notice restoring arrival order",
        "stream.log",
        FILE_ROWS + 2,
        "arrived while selecting",
    )?;

    window.wheel(window.inspect(|app| Ok(center(body(app)?)))?, 0., -370.25)?;
    let reading_position = window.inspect(|app| Ok((app.first_visible_key(), offset(app)?)))?;
    append(&fixture.stream, b"[WARN] gateway/worker1: split \xe4")?;
    let partial = wait_book(window, "visible incomplete UTF-8 line", move |app| {
        let Some(row) = app
            .book
            .record_at(FILE_ROWS + 2)
            .filter(|row| row.message == "split ")
        else {
            return Ok(None);
        };
        check(
            !app.book.follow()
                && app.book.unseen() == 1
                && (app.first_visible_key(), offset(app)?) == reading_position,
            || "Paused viewport moved or counted the provisional line incorrectly".into(),
        )?;
        Ok(Some(row.id))
    })?;
    append(&fixture.stream, b"\xb8\xad")?;
    wait_book(window, "partial record updated in place", move |app| {
        let Some(row) = app
            .book
            .record_at(FILE_ROWS + 2)
            .filter(|row| row.message == "split 中")
        else {
            return Ok(None);
        };
        check(row.id == partial && app.book.unseen() == 1, || {
            "Extending the provisional line changed its identity or incremented the arrival notice"
                .into()
        })?;
        Ok(Some(()))
    })?;
    append(
        &fixture.stream,
        b" done\r\n[ERROR] gateway/worker1: newest\n",
    )?;
    wait_book(
        window,
        "partial line completed before another matching arrival",
        move |app| {
            if app.book.total() != FILE_ROWS + 4 {
                return Ok(None);
            }
            check(
                !app.book.follow()
                    && app.book.unseen() == 2
                    && app
                        .book
                        .record_at(FILE_ROWS + 2)
                        .is_some_and(|row| row.id == partial && row.message == "split 中 done")
                    && (app.first_visible_key(), offset(app)?) == reading_position,
                || "Finishing the partial line changed identity, order or paused position".into(),
            )?;
            Ok(Some(()))
        },
    )?;
    window.control("Follow", |app| app.ui.follow.node())?;
    wait_tail(
        window,
        "explicit Follow switch",
        "stream.log",
        FILE_ROWS + 4,
        "newest",
    )?;
    report.passed("pause-anchor-and-distinct-partial-utf8-arrivals-explicit-resume");
    Ok(())
}

fn nested_scroll(window: &Window, report: &mut ReplayReport) -> Result<()> {
    window.wheel(
        window.inspect(|app| Ok(center(body(app)?)))?,
        -1_000.,
        -260.25,
    )?;
    let key = window.inspect(|app| {
        Ok(app
            .book
            .record_at(LONG_ROW)
            .ok_or("Missing long file record")?
            .id)
    })?;
    window.click(window.inspect(move |app| cell_point(app, cell(key, Column::Number)))?)?;
    window.control("List", |app| app.ui.mode.node(0))?;
    let disclosure = window.inspect(move |app| {
        let node = app
            .list
            .row_node(key)
            .ok_or("Long row was not mounted in the list")?;
        let bounds = app.tree.bounds(node)?;
        let point = Point::new(bounds.origin().x() + 12., center(bounds).y());
        check(hit_within(app, point, node)?, || {
            "Long row disclosure is not visible".into()
        })?;
        Ok(point)
    })?;
    window.click(disclosure)?;

    let (detail, position, limit, outer) = window.inspect(move |app| {
        let row = app.list.row_node(key).ok_or("Missing expanded log row")?;
        let detail = app
            .tree
            .children(row)?
            .last()
            .copied()
            .ok_or("Missing inline detail")?;
        let limit = app.tree.scroll_max_offset(detail)?.y();
        let position = center(app.tree.scroll_viewport_bounds(detail)?);
        check(limit > 23.5 && hit_within(app, position, detail)?, || {
            format!(
                "Expanded inline detail did not have a hittable nested viewport, extent {limit}"
            )
        })?;
        Ok((detail, position, limit, offset(app)?.y()))
    })?;
    window.wheel(position, 0., limit - 7.25)?;
    window.inspect(move |app| {
        check(
            (app.tree.scroll_offset(detail)?.y() - (limit - 7.25)).abs() < 0.01
                && offset(app)?.y() == outer,
            || "Outer list moved before the inline detail reached its boundary".into(),
        )
    })?;
    window.wheel(position, 0., 23.5)?;
    window.inspect(move |app| {
        let inner = app.tree.scroll_offset(detail)?.y();
        let outer_delta = offset(app)?.y() - outer;
        check(
            (inner - limit).abs() < 0.01 && (outer_delta - 16.25).abs() < 0.01,
            || format!("Nested scrolling consumed {inner} of {limit} inside and transferred {outer_delta} instead of 16.25"),
        )
    })?;

    window.control("Table", |app| app.ui.mode.node(1))?;
    window.control("Follow", |app| app.ui.follow.node())?;
    wait_tail(
        window,
        "follow after nested list scrolling",
        "stream.log",
        FILE_ROWS + 4,
        "newest",
    )?;
    report.passed("nested-inline-detail-transfers-only-unused-scroll-distance");
    Ok(())
}

fn sources(window: &Window, fixture: &Fixture, report: &mut ReplayReport) -> Result<()> {
    let previous = window.inspect(|app| {
        let key = app
            .book
            .record_at(FILE_ROWS + 3)
            .ok_or("Tail missing before truncation")?
            .id;
        Ok(key)
    })?;
    window.wheel(window.inspect(|app| Ok(center(body(app)?)))?, -1_000., 0.)?;
    window.click(window.inspect(move |app| cell_point(app, cell(previous, Column::Number)))?)?;
    fs::write(&fixture.stream, "[WARN] replacement/worker: shortened\n")?;
    let truncated = wait_tail(
        window,
        "truncated watched log",
        "stream.log",
        1,
        "shortened",
    )?;
    window.inspect(move |app| {
        check(
            truncated > previous
                && app.active_key().is_none()
                && app.book.index_of(previous).is_none(),
            || "Truncation reused a key or retained a selection from the old generation".into(),
        )
    })?;

    let retired = fixture.directory.join("retired.log");
    fs::rename(&fixture.stream, &retired)?;
    wait_book(window, "active watcher path error", |app| {
        let status = app.tree.text_source(app.ui.status.get()?)?;
        Ok((status.contains("stream.log:")
            && app
                .book
                .record_at(0)
                .is_some_and(|row| row.message == "shortened"))
        .then_some(()))
    })?;
    let incoming = fixture.directory.join("incoming.log");
    fs::write(&incoming, "[WARN] replacement/worker: shortened\n")?;
    fs::rename(&incoming, &fixture.stream)?;
    let rotated = wait_book(
        window,
        "replacement at the watched pathname with identical bytes",
        move |app| {
            let id = app
                .book
                .record_at(0)
                .filter(|row| row.message == "shortened" && row.id > truncated)
                .map(|row| row.id);
            Ok(id)
        },
    )?;
    window.open(Ok(None))?;
    window.inspect(move |app| {
        check(
            app.book.record_at(0).is_some_and(|row| row.id == rotated)
                && app.book.label() == "stream.log",
            || "Cancelling the Open panel changed the active source".into(),
        )
    })?;

    let missing = fixture.directory.join("missing.log");
    window.open(Ok(Some(missing)))?;
    wait_book(
        window,
        "read failure from a missing selected path",
        move |app| {
            let status = app.tree.text_source(app.ui.status.get()?)?;
            Ok((status.contains("missing.log:")
                && app.book.record_at(0).is_some_and(|row| row.id == rotated))
            .then_some(()))
        },
    )?;
    window.open(Err("injected Open panel failure".into()))?;
    window.inspect(move |app| {
        check(
            app.tree.text_source(app.ui.status.get()?)?
                == "Open failed: injected Open panel failure"
                && app.book.record_at(0).is_some_and(|row| row.id == rotated),
            || "Panel error did not surface while retaining the active source".into(),
        )
    })?;
    append(&fixture.stream, b"[INFO] old: still watched\n")?;
    wait_tail(
        window,
        "retained watcher after cancelled and failed Open",
        "stream.log",
        2,
        "still watched",
    )?;

    append(&fixture.stream, b"[WARN] old: queued near switch\n")?;
    window.open(Ok(Some(fixture.other.clone())))?;
    let alternate = wait_tail(
        window,
        "switch to another local file",
        "other.log",
        1,
        "alternate source",
    )?;
    append(&fixture.stream, b"[ERROR] old: late old-source noise\n")?;
    append(&fixture.other, b"[INFO] new/worker: alternate appended\n")?;
    let switched = wait_tail(
        window,
        "new watcher after old-source append",
        "other.log",
        2,
        "alternate appended",
    )?;
    window.inspect(move |app| {
        check(
            switched > alternate
                && app.book.index_of(rotated).is_none()
                && app.book.record_at(0).is_some_and(|row| row.id == alternate)
                && app.book.total() == 2,
            || "Events from the retired watcher contaminated the replacement stream".into(),
        )
    })?;
    report.passed("watcher-truncate-identical-rotation-read-error-cancel-and-switch");
    Ok(())
}

fn wait_tail(
    window: &Window,
    name: &str,
    source: &'static str,
    count: usize,
    message: &'static str,
) -> Result<u64> {
    wait_book(window, name, move |app| {
        if app.book.label() != source || app.book.total() != count {
            return Ok(None);
        }
        let Some(row) = app
            .book
            .record_at(count - 1)
            .filter(|row| row.message == message)
        else {
            return Ok(None);
        };
        let offset = offset(app)?;
        let viewport = body(app)?;
        let visible = app.table.visible_rows(offset.y(), viewport.size().height());
        check(
            app.is_table()
                && app.book.follow()
                && app.book.sort().is_none()
                && app.book.unseen() == 0
                && (offset.y() - maximum(app)?.y()).abs() < 0.01
                && visible.contains(&(count - 1))
                && app.table.mounted_cells().any(|(key, _)| key.row == row.id),
            || {
                format!(
                    "Follow did not reveal latest matching row {message:?}: offset {offset:?}, visible {visible:?}"
                )
            },
        )?;
        Ok(Some(row.id))
    })
}

fn wait_book<T: Send + 'static>(
    window: &Window,
    name: &str,
    expected: impl Fn(&ViewerApp) -> Result<Option<T>> + Send + Sync + 'static,
) -> Result<T> {
    let expected = std::sync::Arc::new(expected);
    window
        .until(name, || {
            let expected = expected.clone();
            window.inspect(move |app| expected(app))
        })
        .map_err(|reason| {
            let state = window.inspect(|app| {
                Ok(format!(
                    "source {}, {} matching of {}, revision {}, status {:?}",
                    app.book.label(),
                    app.book.len(),
                    app.book.total(),
                    app.book.revision(),
                    app.tree.text_source(app.ui.status.get()?)?,
                ))
            });
            match state {
                Ok(state) => format!("{reason} ({state})").into(),
                Err(_) => reason,
            }
        })
}

fn maximum(app: &ViewerApp) -> Result<Point> {
    Ok(app.tree.scroll_max_offset(app.ui.collection.get()?)?)
}

fn append(path: &Path, bytes: &[u8]) -> Result<()> {
    OpenOptions::new()
        .append(true)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}
