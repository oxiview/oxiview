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
use oxiview_style::{Position, Style};
use std::{cell::Cell, rc::Rc};

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
    assert_eq!(first.scene_cache_hits, 0);

    tree.layout(viewport()).unwrap();
    assert_eq!(tree.diagnostics(), first);
    tree.paint(viewport(), 1.0).unwrap();
    let cached = tree.diagnostics();
    assert_eq!(cached.layout_passes, first.layout_passes);
    assert_eq!(cached.measured_nodes, first.measured_nodes);
    assert_eq!(cached.scene_builds, 1);
    assert_eq!(cached.scene_cache_hits, 1);

    let mut style = tree.style(leaf).unwrap().clone();
    style.background = Color::from_rgba8(40, 90, 200, 255);
    tree.set_style(leaf, style).unwrap();
    tree.paint(viewport(), 1.0).unwrap();
    tree.paint(viewport(), 2.0).unwrap();
    let repaint = tree.diagnostics();
    assert_eq!(repaint.layout_passes, first.layout_passes);
    assert_eq!(repaint.measured_nodes, first.measured_nodes);
    assert_eq!(calls.get(), first.measured_nodes);
    assert_eq!(repaint.scene_builds, 3);
    assert_eq!(repaint.scene_cache_hits, 1);

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
            computed_layout_nodes: 7,
            measured_nodes: 3,
            layout_cache_hits: 4,
            scene_builds: 0,
            scene_cache_hits: 0
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
            visited_nodes: 0,
            computed_nodes: usize::MAX,
            measured_nodes: usize::MAX,
            cache_hits: usize::MAX,
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
