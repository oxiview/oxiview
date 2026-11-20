//! Owned UI diagnostic snapshots describe committed work without triggering it.
// Assertions and unwraps are limited to deterministic test fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use oxiview_base::{Color, CornerRadii, Point, Rect, Size};
use oxiview_core::{NodeId, UiDiagnostics, UiTree};
use oxiview_layout::{
    AvailableSpace, Dimensions, LayoutBox, LayoutEngine, LayoutError, LayoutStats, LayoutTree,
    TaffyLayout,
};
use oxiview_scene::{Brush, Primitive};
use oxiview_style::{Length, Position, Style};
use std::{
    cell::Cell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tracing::{
    Event, Metadata, Subscriber,
    span::{Attributes, Id, Record},
};

fn viewport() -> Size {
    Size::new(200.0, 100.0)
}

#[test]
fn snapshots_are_read_only_and_clean_or_paint_only_work_skips_layout() {
    let mut tree = UiTree::new().unwrap();
    let mut style = Style::default();
    style.layout.position = Position::Absolute;
    let leaf = tree.append(tree.root(), style).unwrap();
    let calls = Rc::new(Cell::new(0u64));
    let measured = Rc::clone(&calls);
    tree.set_measure(
        leaf,
        move |known: Dimensions<Option<f64>>, _: Dimensions<AvailableSpace>| {
            measured.set(measured.get() + 1);
            Dimensions {
                width: known.width.unwrap_or(40.0),
                height: known.height.unwrap_or(20.0),
            }
        },
    )
    .unwrap();
    for _ in 0..100 {
        assert_eq!(tree.diagnostics(), UiDiagnostics::default());
    }
    assert_eq!(calls.get(), 0);

    tree.paint(viewport(), 1.0).unwrap();
    let first = tree.diagnostics();
    assert_eq!(first.layout_passes, 1);
    assert!(first.computed_layout_nodes > 0);
    assert!(first.measured_nodes > 0);
    assert_eq!(first.measured_nodes, calls.get());
    assert_eq!(first.scene_builds, 1);
    assert_eq!(first.scene_updates, 0);
    assert_eq!(first.scene_cache_hits, 0);
    assert_eq!(first.paint_node_visits, 2);
    assert_eq!(first.painted_nodes, 2);

    tree.layout(viewport()).unwrap();
    assert_eq!(tree.diagnostics(), first);
    tree.paint(viewport(), 1.0).unwrap();
    let cached = tree.diagnostics();
    assert_eq!(cached.layout_passes, first.layout_passes);
    assert_eq!(cached.measured_nodes, first.measured_nodes);
    assert_eq!(cached.scene_builds, 1);
    assert_eq!(cached.scene_cache_hits, 1);
    assert_eq!(cached.paint_node_visits, first.paint_node_visits);
    assert_eq!(cached.painted_nodes, first.painted_nodes);

    let mut style = tree.style(leaf).unwrap().clone();
    style.background = Color::from_rgba8(40, 90, 200, 255);
    tree.set_style(leaf, style).unwrap();
    tree.paint(viewport(), 1.0).unwrap();
    let updated = tree.diagnostics();
    assert_eq!(updated.scene_builds, cached.scene_builds);
    assert_eq!(updated.scene_updates, cached.scene_updates + 1);
    assert_eq!(updated.paint_node_visits, cached.paint_node_visits + 1);
    tree.paint(viewport(), 2.0).unwrap();
    let repaint = tree.diagnostics();
    assert_eq!(repaint.layout_passes, first.layout_passes);
    assert_eq!(repaint.measured_nodes, first.measured_nodes);
    assert_eq!(calls.get(), first.measured_nodes);
    assert_eq!(repaint.scene_builds, 2);
    assert_eq!(repaint.scene_updates, 1);
    assert_eq!(repaint.scene_cache_hits, 1);
    assert_eq!(repaint.paint_node_visits, updated.paint_node_visits + 2);
    assert_eq!(repaint.painted_nodes, first.painted_nodes);

    tree.invalidate_measure(leaf).unwrap();
    tree.layout(viewport()).unwrap();
    assert_eq!(tree.diagnostics().layout_passes, 2);
    assert!(tree.diagnostics().measured_nodes > first.measured_nodes);
    assert_eq!(tree.diagnostics().measured_nodes, calls.get());
    let snapshot = tree.diagnostics();
    for _ in 0..100 {
        assert_eq!(tree.diagnostics(), snapshot);
    }
    assert_eq!(calls.get(), snapshot.measured_nodes);
}

#[test]
fn repeated_color_edits_to_one_leaf_visit_only_that_leaf() {
    let mut tree = UiTree::new().unwrap();
    let mut style = Style::default();
    style.layout.position = Position::Absolute;
    style.layout.width = Length::Points(8.0);
    style.layout.height = Length::Points(8.0);
    let leaf = tree.append(tree.root(), style.clone()).unwrap();
    for _ in 0..1023 {
        tree.append(tree.root(), style.clone()).unwrap();
    }
    tree.paint(viewport(), 1.0).unwrap();
    let before = tree.diagnostics();
    assert_eq!(before.paint_node_visits, 1025);

    style.background = Color::from_rgba8(40, 90, 200, 255);
    assert!(tree.set_style(leaf, style.clone()).unwrap());
    style.background = Color::from_rgba8(200, 90, 40, 255);
    assert!(tree.set_style(leaf, style).unwrap());
    assert!(
        tree.flush_updates(viewport(), |_, _, _| {})
            .unwrap()
            .changed
    );
    tree.paint(viewport(), 1.0).unwrap();
    assert_eq!(
        tree.diagnostics(),
        UiDiagnostics {
            scene_updates: before.scene_updates + 1,
            paint_node_visits: before.paint_node_visits + 1,
            ..before
        }
    );
    assert!(
        !tree
            .flush_updates(viewport(), |_, _, _| {})
            .unwrap()
            .changed
    );
}

struct ReportedLayout {
    stats: LayoutStats,
    fail: bool,
}
impl LayoutEngine<NodeId> for ReportedLayout {
    fn compute<T: LayoutTree<NodeId = NodeId>>(
        &mut self,
        tree: &mut T,
        root: NodeId,
        viewport: Size,
    ) -> Result<LayoutStats, LayoutError> {
        tree.set_layout(
            root,
            LayoutBox {
                location: Point::default(),
                size: if self.fail {
                    Size::new(999.0, 999.0)
                } else {
                    viewport
                },
                ..LayoutBox::default()
            },
        );
        if self.fail {
            Err(LayoutError::InvalidGeometry)
        } else {
            Ok(self.stats)
        }
    }
}

#[test]
fn custom_engine_statistics_accumulate_only_after_successful_commit() {
    let mut tree = UiTree::new().unwrap();
    let stats = LayoutStats {
        visited_nodes: 11,
        resolved_styles: 6,
        layout_visits: 13,
        computed_nodes: 7,
        measured_nodes: 3,
        cache_hits: 4,
    };
    let mut engine = ReportedLayout { stats, fail: false };
    assert_eq!(tree.layout_with(&mut engine, viewport()).unwrap(), stats);
    assert_eq!(
        tree.diagnostics(),
        UiDiagnostics {
            layout_passes: 1,
            layout_input_visits: 11,
            resolved_styles: 6,
            layout_node_visits: 13,
            computed_layout_nodes: 7,
            measured_nodes: 3,
            layout_cache_hits: 4,
            ..UiDiagnostics::default()
        }
    );
    let committed = tree.bounds(tree.root()).unwrap();
    let before = tree.diagnostics();
    engine.fail = true;
    assert!(
        tree.layout_with(&mut engine, Size::new(300.0, 200.0))
            .is_err()
    );
    assert_eq!(tree.bounds(tree.root()).unwrap(), committed);
    assert_eq!(tree.diagnostics(), before);
    engine.fail = false;
    tree.layout_with(&mut engine, viewport()).unwrap();
    assert_eq!(tree.diagnostics().layout_passes, 2);
    assert_eq!(tree.diagnostics().computed_layout_nodes, 14);
    assert_eq!(tree.diagnostics().measured_nodes, 6);
    assert_eq!(tree.diagnostics().layout_cache_hits, 8);
}

#[test]
fn forced_cached_layout_counts_the_pass_and_actual_cache_hits() {
    let mut tree = UiTree::new().unwrap();
    let leaf = tree.append(tree.root(), Style::default()).unwrap();
    tree.set_intrinsic_size(leaf, Size::new(40.0, 20.0))
        .unwrap();
    let mut engine = TaffyLayout::new();
    tree.layout_with(&mut engine, viewport()).unwrap();
    let before = tree.diagnostics();
    let stats = tree.layout_with(&mut engine, viewport()).unwrap();
    assert!(stats.cache_hits > 0);
    assert_eq!(stats.measured_nodes, 0);
    let after = tree.diagnostics();
    assert_eq!(after.layout_passes, before.layout_passes + 1);
    assert_eq!(after.measured_nodes, before.measured_nodes);
    assert_eq!(
        after.layout_cache_hits - before.layout_cache_hits,
        u64::try_from(stats.cache_hits).unwrap()
    );
}

#[test]
fn failed_default_measurement_and_scene_build_do_not_claim_completed_work() {
    let mut tree = UiTree::new().unwrap();
    let mut style = Style::default();
    style.layout.position = Position::Absolute;
    let leaf = tree.append(tree.root(), style).unwrap();
    tree.set_intrinsic_size(leaf, Size::new(40.0, 20.0))
        .unwrap();
    tree.paint(viewport(), 1.0).unwrap();
    let before = tree.diagnostics();
    let geometry = tree.bounds(leaf).unwrap();
    tree.set_measure(
        leaf,
        |_: Dimensions<Option<f64>>, _: Dimensions<AvailableSpace>| Dimensions {
            width: f64::NAN,
            height: 20.0,
        },
    )
    .unwrap();
    assert!(tree.layout(viewport()).is_err());
    assert_eq!(tree.diagnostics(), before);
    assert_eq!(tree.bounds(leaf).unwrap(), geometry);

    tree.set_measure(
        leaf,
        |_: Dimensions<Option<f64>>, _: Dimensions<AvailableSpace>| Dimensions {
            width: 40.0,
            height: 20.0,
        },
    )
    .unwrap();
    tree.layout(viewport()).unwrap();
    let before = tree.diagnostics();
    tree.set_content(
        leaf,
        vec![Primitive::Stroke(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            CornerRadii::default(),
            -1.0,
            Brush::Solid(Color::from_rgba8(0, 0, 0, 255)),
        )],
    )
    .unwrap();
    assert!(tree.paint(viewport(), 1.0).is_err());
    assert_eq!(tree.diagnostics(), before);
}

#[test]
fn large_reported_counts_saturate_instead_of_wrapping() {
    let mut tree = UiTree::new().unwrap();
    let mut engine = ReportedLayout {
        stats: LayoutStats {
            computed_nodes: usize::MAX,
            measured_nodes: usize::MAX,
            cache_hits: usize::MAX,
            ..LayoutStats::default()
        },
        fail: false,
    };
    tree.layout_with(&mut engine, viewport()).unwrap();
    engine.stats = LayoutStats {
        computed_nodes: 42,
        measured_nodes: 42,
        cache_hits: 42,
        ..LayoutStats::default()
    };
    tree.layout_with(&mut engine, viewport()).unwrap();
    let expected = u64::try_from(usize::MAX)
        .unwrap_or(u64::MAX)
        .saturating_add(42);
    let diagnostics = tree.diagnostics();
    assert_eq!(diagnostics.layout_passes, 2);
    assert_eq!(diagnostics.computed_layout_nodes, expected);
    assert_eq!(diagnostics.measured_nodes, expected);
    assert_eq!(diagnostics.layout_cache_hits, expected);
}

struct HandlerSpans(Arc<AtomicU64>);

impl Subscriber for HandlerSpans {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.name() == "ui_pointer_handler"
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(self.0.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, _: &Event<'_>) {}
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

#[test]
fn dispatch_counts_only_invoked_pointer_handlers_including_cancel() {
    use oxiview_core::PointerEventKind::{Cancel, Down, Enter, Leave};
    use oxiview_platform::{PointerButton, WindowEvent};
    use std::{cell::RefCell, time::Duration};

    let spans = Arc::new(AtomicU64::new(0));
    let _subscriber = tracing::subscriber::set_default(HandlerSpans(Arc::clone(&spans)));
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let parent = tree.append(root, Style::default()).unwrap();
    let leaf_style = {
        let mut style = Style::default();
        style.layout.width = oxiview_style::Length::Points(40.0);
        style.layout.height = oxiview_style::Length::Points(40.0);
        style
    };
    let leaf = tree.append(parent, leaf_style).unwrap();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let trace = Rc::clone(&calls);
    tree.set_pointer_handler(root, move |event, _, _| {
        trace.borrow_mut().push((event.current_target, event.kind));
    })
    .unwrap();
    let trace = Rc::clone(&calls);
    tree.set_pointer_handler(leaf, move |event, _, commands| {
        trace.borrow_mut().push((event.current_target, event.kind));
        if event.kind == Down {
            commands.stop_propagation();
            commands.capture(leaf);
        }
    })
    .unwrap();

    let reports = Rc::new(Cell::new(0));
    let seen = Rc::clone(&reports);
    tree.set_batch_report_handler(move |_, _, _| seen.set(seen.get() + 1))
        .unwrap();

    let mut observed = Vec::new();
    let mut observe =
        |event: &oxiview_core::UiPointerEvent, _: &UiTree, _: &mut oxiview_core::UiCommands| {
            observed.push((event.current_target, event.kind));
        };
    let down = WindowEvent::PointerButton {
        button: PointerButton::Primary,
        pressed: true,
        position: Point::new(5.0, 5.0),
        timestamp: Duration::ZERO,
    };
    let result = tree.dispatch(&down, viewport(), &mut observe).unwrap();
    assert_eq!(result.events, 4);
    assert_eq!(reports.get(), 1);
    assert_eq!(tree.capture_holder(), Some(leaf));
    assert_eq!(
        *calls.borrow(),
        [(root, Enter), (leaf, Enter), (leaf, Down)]
    );
    assert_eq!(tree.diagnostics().dispatched_handlers, 3);
    assert_eq!(spans.load(Ordering::Relaxed), 3);

    let interrupt = WindowEvent::PointerInterrupted {
        timestamp: Duration::from_millis(1),
    };
    let result = tree.dispatch(&interrupt, viewport(), &mut observe).unwrap();
    assert_eq!(result.events, 6);
    assert_eq!(tree.capture_holder(), None);
    assert_eq!(
        &calls.borrow()[3..],
        [(leaf, Cancel), (root, Cancel), (leaf, Leave), (root, Leave)]
    );
    assert_eq!(tree.diagnostics().dispatched_handlers, 7);
    assert_eq!(spans.load(Ordering::Relaxed), 7);
    assert_eq!(
        tree.dispatch(&interrupt, viewport(), &mut observe)
            .unwrap()
            .events,
        0
    );
    assert_eq!(tree.diagnostics().dispatched_handlers, 7);
    assert_eq!(
        observed,
        [
            (root, Enter),
            (parent, Enter),
            (leaf, Enter),
            (leaf, Down),
            (leaf, Cancel),
            (parent, Cancel),
            (root, Cancel),
            (leaf, Leave),
            (parent, Leave),
            (root, Leave),
        ]
    );
}
