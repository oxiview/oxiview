#![allow(clippy::unwrap_used)]

use super::*;
use crate::{FrameKind, FrameTiming, Scheduler};
use oxiview_base::Color;
use oxiview_platform::{
    KeyCode, KeyModifiers, LogicalKey, PointerButton, ScrollMomentum, ScrollPhase,
    SystemPreferences, WindowEvent,
};
use oxiview_scene::{Brush, Clip, NodeKind};
use oxiview_style::{Length, Position};
use std::time::Duration;

const SIZE: Size = Size::new(320.0, 220.0);
const INSIDE: Point = Point::new(65.0, 65.0);
const OUTSIDE: Point = Point::new(280.0, 75.0);
fn leaf_color() -> Color {
    Color::from_rgba8(83, 94, 107, 255)
}

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn absolute(x: f64, y: f64, width: f64, height: f64) -> Style {
    let mut style = Style::default();
    style.layout.position = Position::Absolute;
    style.layout.width = Length::Points(width);
    style.layout.height = Length::Points(height);
    style.layout.inset.left = Length::Points(x);
    style.layout.inset.top = Length::Points(y);
    style
}

fn fixture(extent: Size) -> (UiTree, NodeId, NodeId, NodeId) {
    let mut tree = UiTree::new().unwrap();
    let outer = tree
        .append(tree.root(), absolute(20.0, 20.0, 150.0, 140.0))
        .unwrap();
    let inner = tree
        .append(outer, absolute(20.0, 20.0, 80.0, 70.0))
        .unwrap();
    let mut style = absolute(20.0, 20.0, 60.0, 80.0);
    style.background = leaf_color();
    let leaf = tree.append(inner, style).unwrap();
    tree.set_scroll_content_extent(outer, extent).unwrap();
    tree.set_scroll_content_extent(inner, Size::new(80.0, 80.0))
        .unwrap();
    tree.layout(SIZE).unwrap();
    (tree, outer, inner, leaf)
}

fn wheel(position: Point, x: f64, y: f64) -> WindowEvent {
    WindowEvent::Wheel {
        position,
        delta_x: x,
        delta_y: y,
        timestamp: ms(1),
    }
}

fn phase(kind: ScrollPhase, position: Point, y: f64, time: u64) -> WindowEvent {
    WindowEvent::Scroll {
        position,
        delta_x: 0.0,
        delta_y: y,
        phase: kind,
        timestamp: ms(time),
    }
}

fn dispatch(tree: &mut UiTree, event: WindowEvent) -> UiDispatchResult {
    tree.dispatch(&event, SIZE, |_, _, _| {}).unwrap()
}

#[test]
fn nested_wheel_routes_each_axis_and_paint_hit_share_fractional_geometry() {
    let (mut tree, outer, inner, leaf) = fixture(Size::new(500.0, 3_000_000.0));
    tree.paint(SIZE, 1.5).unwrap();
    let before = tree.diagnostics();
    assert!(dispatch(&mut tree, wheel(INSIDE, 17.5, 24.0)).changed);
    assert_eq!(tree.scroll_offset(inner).unwrap(), Point::new(0.0, 10.0));
    assert_eq!(tree.scroll_offset(outer).unwrap(), Point::new(17.5, 14.0));
    assert_eq!(
        tree.layout_bounds(leaf).unwrap().origin(),
        Point::new(60.0, 60.0)
    );
    let translated = Rect::new(42.5, 36.0, 102.5, 116.0);
    assert_eq!(tree.bounds(leaf).unwrap(), translated);
    assert_eq!(tree.hit_test(Point::new(43.0, 50.0)).unwrap(), Some(leaf));
    assert_ne!(tree.hit_test(Point::new(43.0, 98.0)).unwrap(), Some(leaf));

    let painted = tree.paint(SIZE, 1.5).unwrap();
    let mut pending = vec![(painted.root, Vec::new())];
    let mut found = false;
    while let Some((id, mut clips)) = pending.pop() {
        match painted.scene.content(id).unwrap() {
            NodeKind::Clip(Clip::RoundedRect(rect, _)) => clips.push(*rect),
            NodeKind::Draw(Primitive::Fill(rect, _, Brush::Solid(color)))
                if *color == leaf_color() =>
            {
                assert_eq!(*rect, translated);
                assert_eq!(
                    clips,
                    [
                        tree.scroll_viewport_bounds(outer).unwrap(),
                        tree.scroll_viewport_bounds(inner).unwrap(),
                    ]
                );
                found = true;
            }
            _ => {}
        }
        for child in painted.scene.children(Some(id)).unwrap() {
            pending.push((*child, clips.clone()));
        }
    }
    assert!(found);
    assert_eq!(tree.diagnostics().layout_passes, before.layout_passes);
    assert_eq!(tree.len(), 4);

    // Reverse direction with a hit still inside the nested viewport.
    dispatch(&mut tree, wheel(Point::new(45.0, 45.0), -50.0, -90.0));
    assert_eq!(tree.scroll_offset(inner).unwrap(), Point::default());
    assert_eq!(tree.scroll_offset(outer).unwrap(), Point::default());
}

#[test]
fn gesture_and_native_momentum_keep_the_original_path_until_a_fresh_begin() {
    let (mut tree, outer, inner, _) = fixture(Size::new(150.0, 640.0));
    for event in [
        phase(ScrollPhase::Begin(ScrollMomentum::Native), INSIDE, 3.0, 1),
        phase(ScrollPhase::Update, OUTSIDE, 4.0, 2),
        phase(ScrollPhase::End, OUTSIDE, 1.0, 3),
    ] {
        dispatch(&mut tree, event);
    }
    assert_eq!(tree.scroll_offset(inner).unwrap().y(), 8.0);
    tree.paint(SIZE, 1.0).unwrap();
    assert!(!tree.scroll_animation_active());
    assert!(!tree.advance_scroll(ms(8)).unwrap().changed);

    dispatch(
        &mut tree,
        phase(ScrollPhase::MomentumBegin, OUTSIDE, 7.0, 9),
    );
    dispatch(
        &mut tree,
        phase(ScrollPhase::MomentumUpdate, OUTSIDE, 5.0, 10),
    );
    assert_eq!(tree.scroll_offset(inner).unwrap().y(), 10.0);
    assert_eq!(tree.scroll_offset(outer).unwrap().y(), 10.0);

    dispatch(
        &mut tree,
        phase(ScrollPhase::Begin(ScrollMomentum::None), OUTSIDE, 50.0, 11),
    );
    dispatch(
        &mut tree,
        phase(ScrollPhase::MomentumEnd, OUTSIDE, 40.0, 12),
    );
    dispatch(&mut tree, phase(ScrollPhase::Update, INSIDE, 50.0, 13));
    assert_eq!(tree.scroll_offset(outer).unwrap().y(), 10.0);
    dispatch(&mut tree, wheel(INSIDE, 0.0, 10.0));
    assert_eq!(tree.scroll_offset(outer).unwrap().y(), 20.0);
}

#[test]
fn latched_gesture_skips_released_viewport_identity() {
    let (mut tree, outer, inner, _) = fixture(Size::new(150.0, 640.0));
    dispatch(
        &mut tree,
        phase(ScrollPhase::Begin(ScrollMomentum::None), INSIDE, 0.0, 1),
    );
    tree.remove(inner).unwrap();
    let replacement = tree
        .append(outer, absolute(20.0, 20.0, 80.0, 70.0))
        .unwrap();
    tree.set_scroll_content_extent(replacement, Size::new(80.0, 200.0))
        .unwrap();
    dispatch(&mut tree, phase(ScrollPhase::Update, INSIDE, 30.0, 2));
    assert_eq!(tree.scroll_offset(replacement).unwrap().y(), 0.0);
    assert_eq!(tree.scroll_offset(outer).unwrap().y(), 30.0);
}

fn release(tree: &mut UiTree) {
    dispatch(
        tree,
        phase(
            ScrollPhase::Begin(ScrollMomentum::Framework),
            INSIDE,
            0.0,
            10,
        ),
    );
    dispatch(tree, phase(ScrollPhase::Update, OUTSIDE, 12.0, 20));
    dispatch(tree, phase(ScrollPhase::End, OUTSIDE, 0.0, 22));
    assert!(tree.scroll_animation_active());
    tree.paint(SIZE, 1.0).unwrap();
}

fn frame(timestamp: Duration) -> WindowEvent {
    WindowEvent::Frame {
        predicted_presentation_timestamp: timestamp,
        refresh_interval: ms(16),
    }
}

#[test]
fn injected_frame_time_drives_decay_and_stops_exactly_at_the_extent() {
    let (mut dense, dense_outer, _, _) = fixture(Size::new(150.0, 2_000.0));
    let (mut sparse, sparse_outer, _, _) = fixture(Size::new(150.0, 2_000.0));
    release(&mut dense);
    release(&mut sparse);
    for time in [32, 42, 52, 62] {
        dispatch(&mut dense, frame(ms(time)));
    }
    sparse.advance_scroll(ms(62)).unwrap();
    let dense_offset = dense.scroll_offset(dense_outer).unwrap().y();
    let sparse_offset = sparse.scroll_offset(sparse_outer).unwrap().y();
    assert!(dense_offset > 2.0);
    assert!((dense_offset - sparse_offset).abs() < 1e-10);
    sparse.advance_scroll(ms(40)).unwrap();
    assert_eq!(
        sparse.scroll_offset(sparse_outer).unwrap().y(),
        sparse_offset
    );

    let (mut tree, outer, inner, _) = fixture(Size::new(150.0, 150.0));
    release(&mut tree);
    let mut scheduler = Scheduler::default();
    assert!(!scheduler.schedule().request_frame);
    scheduler.set_scroll_animation_active(tree.scroll_animation_active());
    assert!(scheduler.schedule().request_frame);

    let at = ms(38);
    assert!(dispatch(&mut tree, frame(at)).changed);
    assert_eq!(tree.scroll_offset(outer).unwrap().y(), 10.0);
    assert_eq!(tree.scroll_offset(inner).unwrap().y(), 10.0);
    assert!(!tree.scroll_animation_active());
    scheduler.set_scroll_animation_active(tree.scroll_animation_active());
    scheduler.invalidate();
    assert!(scheduler.frame());
    tree.paint(SIZE, 1.0).unwrap();
    scheduler.produced(
        FrameTiming {
            predicted_presentation_timestamp: at,
            refresh_interval: ms(16),
            kind: FrameKind::Vsync,
        },
        at,
    );
    for opportunity in 1..=1_000 {
        assert!(!scheduler.schedule().request_frame);
        assert!(!dispatch(&mut tree, frame(at + ms(opportunity * 16))).changed);
        assert!(!scheduler.frame());
    }
}

#[test]
fn new_gesture_capture_window_lifecycle_and_reduced_motion_interrupt_decay() {
    for interruption in [
        phase(ScrollPhase::Begin(ScrollMomentum::None), INSIDE, 0.0, 24),
        WindowEvent::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
            position: INSIDE,
            timestamp: ms(24),
        },
        WindowEvent::VisibilityChanged {
            visible: false,
            timestamp: ms(24),
        },
        WindowEvent::ActivationChanged {
            active: false,
            timestamp: ms(24),
        },
        WindowEvent::PointerInterrupted { timestamp: ms(24) },
        WindowEvent::PreferencesChanged {
            preferences: SystemPreferences {
                reduced_motion: Some(true),
                ..Default::default()
            },
        },
    ] {
        let (mut tree, outer, inner, _) = fixture(Size::new(150.0, 2_000.0));
        release(&mut tree);
        let mut scheduler = Scheduler::default();
        scheduler.set_scroll_animation_active(true);
        assert!(scheduler.schedule().request_frame);
        let before = tree.scroll_offset(outer).unwrap();
        tree.dispatch(&interruption, SIZE, |event, _, commands| {
            if event.kind == PointerEventKind::Down && event.current_target == inner {
                commands.capture(inner);
            }
        })
        .unwrap();
        if matches!(&interruption, WindowEvent::PointerButton { .. }) {
            assert_eq!(tree.capture_holder(), Some(inner));
        }
        assert!(!tree.scroll_animation_active());
        scheduler.set_scroll_animation_active(tree.scroll_animation_active());
        tree.paint(SIZE, 1.0).unwrap();
        assert!(!dispatch(&mut tree, frame(ms(38))).changed);
        assert_eq!(tree.scroll_offset(outer).unwrap(), before);
        assert!(!scheduler.frame());
        assert!(!scheduler.schedule().request_frame);

        if matches!(&interruption, WindowEvent::PointerButton { .. }) {
            tree.dispatch(
                &WindowEvent::PointerMoved {
                    position: OUTSIDE,
                    timestamp: ms(39),
                },
                SIZE,
                |event, _, commands| {
                    if event.kind == PointerEventKind::Move && event.current_target == inner {
                        commands.set_scroll_offset(outer, Point::new(0.0, 350.5));
                    }
                },
            )
            .unwrap();
            assert_eq!(tree.scroll_offset(outer).unwrap().y(), 350.5);
            assert!(!tree.scroll_animation_active());
        }
    }
}

#[test]
fn scroll_reconciles_opted_in_hover_and_keeps_pointer_local_geometry_current() {
    let mut tree = UiTree::new().unwrap();
    let mut style = absolute(0.0, 0.0, 80.0, 50.0);
    style.stationary_hover_updates = true;
    let view = tree.append(tree.root(), style).unwrap();
    let a = tree.append(view, absolute(0.0, 0.0, 80.0, 25.0)).unwrap();
    let b = tree.append(view, absolute(0.0, 25.0, 80.0, 25.0)).unwrap();
    tree.set_scroll_content_extent(view, Size::new(80.0, 130.0))
        .unwrap();
    let pointer = Point::new(4.0, 12.0);
    dispatch(
        &mut tree,
        WindowEvent::PointerMoved {
            position: pointer,
            timestamp: ms(0),
        },
    );
    assert_eq!(tree.hover_target(), Some(a));
    let mut events = Vec::new();
    tree.dispatch(&wheel(pointer, 0.0, 22.5), SIZE, |event, _, _| {
        events.push((event.kind, event.current_target, event.local_position));
    })
    .unwrap();
    assert_eq!(
        events,
        [
            (PointerEventKind::Leave, a, Point::new(4.0, 34.5)),
            (PointerEventKind::Enter, b, Point::new(4.0, 9.5)),
        ]
    );
    assert_eq!(tree.hover_target(), Some(b));
    tree.set_scroll_offset(view, Point::default()).unwrap();
    tree.flush_updates(SIZE, |_, _, _| {}).unwrap();
    assert_eq!(tree.hover_target(), Some(a));

    // A jump can move the entire mounted range out of the viewport. A virtual
    // row mounted after input should still join the stationary hover path.
    dispatch(&mut tree, wheel(pointer, 0.0, 70.0));
    assert_eq!(tree.hover_target(), Some(view));
    let rows = tree
        .mount_many([(view, node().style(absolute(0.0, 70.0, 80.0, 25.0)))])
        .unwrap();
    assert_eq!(tree.hover_target(), Some(rows[0]));
}

#[test]
fn extent_and_layout_changes_clamp_and_keyboard_focus_reveals_hidden_content() {
    let mut tree = UiTree::new().unwrap();
    let container = tree
        .append(tree.root(), absolute(0.0, 0.0, 100.0, 75.0))
        .unwrap();
    let target = tree
        .append(container, absolute(30.0, 90.0, 30.0, 20.0))
        .unwrap();
    assert_eq!(
        tree.scroll_offset(container),
        Err(UiError::ScrollUnavailable)
    );
    assert_eq!(
        tree.set_scroll_content_extent(container, Size::new(f64::NAN, 20.0)),
        Err(UiError::InvalidScrollGeometry)
    );
    tree.set_scroll_content_extent(container, Size::new(100.0, 200.0))
        .unwrap();
    tree.set_focus_policy(target, FocusPolicy::Tab).unwrap();
    dispatch(
        &mut tree,
        WindowEvent::Key {
            code: KeyCode::Tab,
            logical: LogicalKey::Named(KeyCode::Tab),
            modifiers: KeyModifiers::default(),
            pressed: true,
            repeat: false,
            timestamp: ms(1),
        },
    );
    assert_eq!(tree.focused_node(), Some(target));
    assert_eq!(tree.scroll_offset(container).unwrap().y(), 35.0);
    assert_eq!(tree.bounds(target).unwrap().edges()[3], 75.0);
    tree.set_scroll_offset(container, Point::new(25.0, 300.0))
        .unwrap();
    assert_eq!(
        tree.scroll_offset(container).unwrap(),
        Point::new(0.0, 125.0)
    );
    tree.set_scroll_content_extent(container, Size::new(100.0, 160.0))
        .unwrap();
    assert_eq!(tree.scroll_offset(container).unwrap().y(), 85.0);
    tree.set_style(container, absolute(0.0, 0.0, 100.0, 140.0))
        .unwrap();
    tree.layout(SIZE).unwrap();
    assert_eq!(tree.scroll_offset(container).unwrap().y(), 20.0);
}

#[test]
fn programmatic_focus_reveals_a_node_moved_since_the_last_layout() {
    let mut tree = UiTree::new().unwrap();
    let view = tree
        .append(tree.root(), absolute(0.0, 0.0, 100.0, 75.0))
        .unwrap();
    let target = tree.append(view, absolute(0.0, 0.0, 30.0, 20.0)).unwrap();
    tree.set_scroll_content_extent(view, Size::new(100.0, 200.0))
        .unwrap();
    tree.set_focus_policy(target, FocusPolicy::Programmatic)
        .unwrap();
    tree.layout(SIZE).unwrap();

    tree.set_style(target, absolute(0.0, 100.0, 30.0, 20.0))
        .unwrap();
    tree.focus(target).unwrap();

    assert_eq!(tree.focused_node(), Some(target));
    assert_eq!(tree.scroll_offset(view).unwrap().y(), 45.0);
    assert_eq!(tree.bounds(target).unwrap().edges()[3], 75.0);
}

#[test]
fn batch_mounts_virtual_children_with_one_layout_and_rolls_back_on_error() {
    let (mut tree, outer, inner, _) = fixture(Size::new(900.0, 3_000_000.0));
    let before = tree.diagnostics().layout_passes;
    let roots = tree
        .mount_many((0..30).map(|index| {
            let parent = if index < 15 { outer } else { inner };
            (
                parent,
                node().style(absolute(0.0, f64::from(index) * 20.0, 72.0, 20.0)),
            )
        }))
        .unwrap();
    assert_eq!(roots.len(), 30);
    assert_eq!(tree.diagnostics().layout_passes - before, 1);
    assert_eq!(tree.parent(roots[0]).unwrap(), Some(outer));
    assert_eq!(tree.parent(roots[29]).unwrap(), Some(inner));

    let count = tree.len();
    let another = UiTree::new().unwrap();
    assert!(matches!(
        tree.mount_many([(inner, node()), (another.root(), node())]),
        Err(UiError::ForeignNode)
    ));
    assert_eq!(tree.len(), count);

    let invalid = Style {
        opacity: -1.0,
        ..Default::default()
    };
    assert!(matches!(
        tree.mount_many([
            (outer, node()),
            (inner, node().bind_style(move || Ok(invalid.clone())))
        ]),
        Err(UiError::Style(_))
    ));
    tree.flush_updates(SIZE, |_, _, _| {}).unwrap();
    assert_eq!(tree.len(), count);
}
