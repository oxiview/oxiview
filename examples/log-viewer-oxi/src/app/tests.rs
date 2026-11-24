use super::*;
use oxiview_core::FrameKind;
use oxiview_platform::{
    ClipboardRequest, KeyCode, KeyModifiers, LogicalKey, PointerButton, TextCommitKind,
    TextInputEdit, TextInputEvent, WindowWake,
};
use oxiview_widgets::collection::CellKey;
use std::sync::Arc;

struct Wake;

#[test]
fn measured_row_geometry_tracks_the_scene_across_row_boundaries() -> Result<(), Error> {
    let mut app = ViewerApp::new(None)?;
    let size = PhysicalSize {
        width: 2240,
        height: 1440,
    };
    let period = Duration::from_nanos(8_333_333);
    let mut signatures = std::collections::BTreeSet::new();
    for step in 1..=36 {
        app.scroll_sample(12.0)?;
        let scene = app.paint(
            FrameTiming {
                predicted_presentation_timestamp: period * step,
                refresh_interval: period,
                kind: FrameKind::Vsync,
            },
            size,
            2.0,
        )?;
        let PaintedListPosition {
            first_key: key,
            offset_y: offset,
            row_top_y: painted_top,
            scene_fingerprint,
        } = app.painted_list_position(&scene)?;
        assert!(signatures.insert(scene_fingerprint));
        assert_eq!(offset, f64::from(step) * 12.0);
        assert_eq!(key, (offset / view::LIST_HEIGHT).floor() as u64 + 1);
        assert_eq!(painted_top, (key - 1) as f64 * view::LIST_HEIGHT - offset);
    }
    Ok(())
}

impl WindowWake for Wake {
    fn wake(&self) -> Result<(), oxiview_platform::Error> {
        Ok(())
    }
}

struct Fixture {
    app: ViewerApp,
    scheduler: Scheduler,
    waker: WindowWaker,
    time: Duration,
}

impl Fixture {
    fn new() -> Result<Self, Error> {
        Ok(Self {
            app: ViewerApp::new(None)?,
            scheduler: Scheduler::default(),
            waker: WindowWaker::new(Arc::new(Wake)),
            time: Duration::from_millis(20),
        })
    }

    fn send(&mut self, event: WindowEvent) -> Result<(), Error> {
        self.app.event(&event, &mut self.scheduler, &self.waker)?;
        self.time += Duration::from_millis(20);
        Ok(())
    }

    fn pointer(&mut self, point: Point, pressed: bool) -> Result<(), Error> {
        self.send(WindowEvent::PointerButton {
            button: PointerButton::Primary,
            position: point,
            pressed,
            timestamp: self.time,
        })
    }

    fn click(&mut self, point: Point) -> Result<(), Error> {
        self.pointer(point, true)?;
        self.pointer(point, false)
    }

    fn move_pointer(&mut self, position: Point) -> Result<(), Error> {
        self.send(WindowEvent::PointerMoved {
            position,
            timestamp: self.time,
        })
    }

    fn click_node(&mut self, id: oxiview_core::NodeId) -> Result<(), Error> {
        let bounds = self.app.tree.bounds(id)?;
        let [l, t, r, b] = bounds.edges();
        self.click(Point::new((l + r) * 0.5, (t + b) * 0.5))
    }

    fn frame(&mut self) -> Result<(), Error> {
        self.send(WindowEvent::Frame {
            predicted_presentation_timestamp: self.time,
            refresh_interval: Duration::from_millis(16),
        })
    }

    fn key(&mut self, code: KeyCode, shift: bool) -> Result<(), Error> {
        for pressed in [true, false] {
            self.send(WindowEvent::Key {
                code,
                logical: LogicalKey::Named(code),
                modifiers: KeyModifiers {
                    shift,
                    ..Default::default()
                },
                pressed,
                repeat: false,
                timestamp: self.time,
            })?;
        }
        Ok(())
    }

    fn table(&mut self) -> Result<(), Error> {
        self.click_node(
            self.app
                .ui
                .mode
                .node(1)
                .ok_or_else(|| error("missing Table segment"))?,
        )?;
        assert!(self.app.is_table());
        Ok(())
    }

    fn paint(&mut self, width: u32, height: u32) -> Result<PaintedScene, Error> {
        self.app.paint(
            FrameTiming {
                predicted_presentation_timestamp: self.time,
                refresh_interval: Duration::from_millis(16),
                kind: FrameKind::Resize,
            },
            PhysicalSize { width, height },
            1.,
        )
    }
}

#[test]
fn wheel_and_width_change_keep_anchored_expanded_rows() -> Result<(), Error> {
    let mut fixture = Fixture::new()?;
    assert_eq!(fixture.app.first_visible_key(), Some(1));
    let node = fixture
        .app
        .list
        .row_node(6)
        .ok_or_else(|| error("missing sample record"))?;
    let bounds = fixture.app.tree.bounds(node)?;
    let origin = bounds.origin();
    fixture.click(Point::new(origin.x() + 14., origin.y() + 11.))?;
    assert_eq!(fixture.app.active_key(), Some(6));
    let index = fixture
        .app
        .list
        .index_of(6)
        .ok_or_else(|| error("missing expanded index"))?;
    let height = fixture
        .app
        .list
        .height(index)
        .ok_or_else(|| error("missing expanded height"))?;
    assert!(height > view::LIST_HEIGHT);
    fixture.app.scroll_sample(300.25)?;
    let body = fixture.app.ui.collection.get()?;
    let offset = fixture.app.tree.scroll_offset(body)?;
    let anchor = fixture.app.list.anchor(offset.y());
    let _scene = fixture.paint(850, 720)?;
    let new_offset = fixture.app.tree.scroll_offset(body)?;
    assert_eq!(fixture.app.list.anchor(new_offset.y()), anchor);
    assert!(
        fixture
            .app
            .list
            .height(index)
            .is_some_and(|new| new > height)
    );
    assert_eq!(fixture.app.active_key(), Some(6));
    assert!(fixture.app.tree.len() < 300);
    Ok(())
}

#[test]
fn table_copies_offscreen_cells_and_keeps_range_through_resize() -> Result<(), Error> {
    let mut fixture = Fixture::new()?;
    fixture.table()?;
    let level = Column::Level as u64;
    let first = fixture
        .app
        .table
        .cell_node(CellKey {
            row: 1,
            column: level,
        })
        .ok_or_else(|| error("missing Level cell"))?;
    fixture.click_node(first)?;
    for _ in 0..22 {
        fixture.key(KeyCode::ArrowDown, true)?;
    }
    fixture.key(KeyCode::ArrowRight, true)?;
    let range = fixture
        .app
        .table
        .selection()
        .ok_or_else(|| error("missing rectangle"))?;
    assert_eq!(
        range.anchor,
        CellKey {
            row: 1,
            column: level
        }
    );
    assert_eq!(
        range.active,
        CellKey {
            row: 23,
            column: Column::Source as u64
        }
    );
    assert!(fixture.app.first_visible_key().is_some_and(|key| key > 1));

    fixture.click_node(
        fixture
            .app
            .ui
            .copy
            .node()
            .ok_or_else(|| error("missing Copy button"))?,
    )?;
    let mut commands = Vec::new();
    fixture.app.flush_platform(&mut commands)?;
    let expected = (0..23)
        .map(|row| {
            format!(
                "{}\t{}",
                fixture.app.book.cell(row, Column::Level),
                fixture.app.book.cell(row, Column::Source)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(commands.iter().any(|command| matches!(
        command, PlatformCommand::Clipboard(ClipboardRequest::Write { text, .. }) if *text == expected
    )));

    let header = fixture
        .app
        .table
        .header_node(level)
        .ok_or_else(|| error("missing heading"))?;
    let bounds = fixture.app.tree.bounds(header)?;
    let [left, top, right, bottom] = bounds.edges();
    let grip = Point::new(right - 2., (top + bottom) * 0.5);
    let dragged = Point::new(grip.x() + 37., grip.y());
    fixture.pointer(grip, true)?;
    fixture.move_pointer(dragged)?;
    fixture.pointer(dragged, false)?;
    assert_eq!(fixture.app.table.selection(), Some(range));
    assert_eq!(fixture.app.table.column_width(2), Some(right - left + 37.));

    let thumb = fixture
        .app
        .ui
        .vertical
        .thumb_node()
        .ok_or_else(|| error("missing vertical thumb"))?;
    let body = fixture.app.ui.collection.get()?;
    let thumb_position = fixture.app.tree.bounds(thumb)?.origin();
    let handle = Point::new(thumb_position.x() + 4., thumb_position.y() + 8.);
    let moved = Point::new(handle.x(), handle.y() + 17.25);
    let offset = fixture.app.tree.scroll_offset(body)?;
    fixture.pointer(handle, true)?;
    fixture.move_pointer(moved)?;
    fixture.pointer(moved, false)?;
    assert!(fixture.app.tree.scroll_offset(body)?.y() > offset.y());
    assert_eq!(fixture.app.table.selection(), Some(range));

    fixture.click_node(header)?;
    let collapsed = fixture
        .app
        .table
        .selection()
        .ok_or_else(|| error("missing active cell after sort"))?;
    assert_eq!(fixture.app.book.sort(), Some((Column::Level, true)));
    assert_eq!(collapsed.anchor, range.active);
    assert_eq!(collapsed.active, range.active);
    Ok(())
}

#[test]
fn native_search_composition_filters_only_on_commit() -> Result<(), Error> {
    let mut fixture = Fixture::new()?;
    fixture.table()?;
    let selected = fixture
        .app
        .table
        .cell_node(CellKey {
            row: 1,
            column: Column::Time as u64,
        })
        .ok_or_else(|| error("missing Time cell"))?;
    fixture.click_node(selected)?;
    fixture.click_node(
        fixture
            .app
            .ui
            .search
            .node()
            .ok_or_else(|| error("missing search"))?,
    )?;
    let input = fixture
        .app
        .tree
        .focused_text_input_snapshot(fixture.app.viewport)?
        .ok_or_else(|| error("missing editor focus"))?;
    let query = "no-such-record";
    fixture.send(WindowEvent::TextInput(TextInputEvent {
        session: input.session,
        revision: input.revision,
        sequence: 1,
        timestamp: fixture.time,
        edit: TextInputEdit::SetMarked {
            range: 0..0,
            text: query.into(),
            selection: query.len()..query.len(),
        },
    }))?;
    assert_eq!(fixture.app.book.query(), "");
    assert_eq!(fixture.app.book.len(), 100_000);
    assert_eq!(fixture.app.active_key(), Some(1));
    fixture.send(WindowEvent::TextInput(TextInputEvent {
        session: input.session,
        revision: input.revision,
        sequence: 2,
        timestamp: fixture.time,
        edit: TextInputEdit::Commit {
            range: 0..query.len(),
            text: query.into(),
            kind: TextCommitKind::Composition,
        },
    }))?;
    assert_eq!(fixture.app.book.query(), query);
    assert!(fixture.app.book.is_empty());
    assert_eq!(fixture.app.first_visible_key(), None);
    assert_eq!(fixture.app.active_key(), None);
    let copy = fixture
        .app
        .ui
        .copy
        .node()
        .ok_or_else(|| error("missing Copy button"))?;
    assert!(fixture.app.tree.interaction_state(copy)?.disabled);
    Ok(())
}

#[test]
fn captured_table_range_auto_scrolls_at_both_edges() -> Result<(), Error> {
    let mut fixture = Fixture::new()?;
    fixture.table()?;
    let source = fixture
        .app
        .table
        .cell_node(CellKey {
            row: 1,
            column: Column::Source as u64,
        })
        .ok_or_else(|| error("missing Source cell"))?;
    let point = fixture.app.tree.bounds(source)?.origin();
    fixture.pointer(Point::new(point.x() + 10., point.y() + 10.), true)?;
    let body = fixture.app.ui.collection.get()?;
    assert_eq!(fixture.app.tree.capture_holder(), Some(body));
    let [_, _, right, bottom] = fixture.app.tree.bounds(body)?.edges();
    let beyond = Point::new(right + 24., bottom + 25.);
    fixture.move_pointer(beyond)?;
    for _ in 0..6 {
        fixture.frame()?;
    }
    let offset = fixture.app.tree.scroll_offset(body)?;
    assert!(offset.x() > 50. && offset.y() > 50.);
    let range = fixture
        .app
        .table
        .selection()
        .ok_or_else(|| error("missing dragged range"))?;
    assert_eq!(range.anchor.row, 1);
    assert!(range.active.row > 15);
    assert!(range.active.column > range.anchor.column);
    fixture.pointer(beyond, false)?;
    assert_eq!(fixture.app.tree.capture_holder(), None);
    assert!(!fixture.app.drag_can_scroll()?);
    Ok(())
}

#[test]
fn expanded_message_offers_unused_wheel_distance_to_the_list() -> Result<(), Error> {
    let mut fixture = Fixture::new()?;
    let _scene = fixture.paint(340, 800)?;
    let row = fixture
        .app
        .list
        .row_node(6)
        .ok_or_else(|| error("missing sample record"))?;
    let bounds = fixture.app.tree.bounds(row)?;
    let origin = bounds.origin();
    fixture.click(Point::new(origin.x() + 14., origin.y() + 10.))?;
    let row = fixture
        .app
        .list
        .row_node(6)
        .ok_or_else(|| error("missing expanded record"))?;
    let inner = fixture
        .app
        .tree
        .children(row)?
        .last()
        .copied()
        .ok_or_else(|| error("missing message viewport"))?;
    let maximum = fixture.app.tree.scroll_max_offset(inner)?.y();
    assert!(maximum > 0.);
    let position = fixture.app.tree.bounds(inner)?.origin();
    fixture.send(WindowEvent::Wheel {
        position: Point::new(position.x() + 10., position.y() + 5.),
        delta_x: 0.,
        delta_y: maximum + 25.,
        timestamp: fixture.time,
    })?;
    assert_eq!(fixture.app.tree.scroll_offset(inner)?.y(), maximum);
    assert_eq!(
        fixture
            .app
            .tree
            .scroll_offset(fixture.app.ui.collection.get()?)?
            .y(),
        25.
    );
    Ok(())
}

#[test]
fn upward_scroll_pauses_follow_even_after_scrolling_back_to_tail() -> Result<(), Error> {
    let mut fixture = Fixture::new()?;
    let follow = fixture
        .app
        .ui
        .follow
        .node()
        .ok_or_else(|| error("missing Follow switch"))?;
    fixture.click_node(follow)?;
    assert!(fixture.app.book.follow());
    let body = fixture.app.ui.collection.get()?;
    let maximum = fixture.app.tree.scroll_max_offset(body)?.y();
    assert_eq!(fixture.app.tree.scroll_offset(body)?.y(), maximum);
    fixture.app.scroll_sample(-175.5)?;
    assert!(!fixture.app.book.follow());
    assert_eq!(fixture.app.tree.scroll_offset(body)?.y(), maximum - 175.5);
    fixture.app.scroll_sample(250.)?;
    assert!(!fixture.app.book.follow());
    assert_eq!(fixture.app.tree.scroll_offset(body)?.y(), maximum);
    Ok(())
}
