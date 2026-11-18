// Test fixtures use assertions and unwraps to identify unexpected failures precisely.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use oxiview_base::{Color, CornerRadii};
use oxiview_platform::{PointerButton, WindowEvent};
use oxiview_style::{Length, Overflow, Position};
use std::{cell::Cell, time::Duration};

fn absolute(x: f64, y: f64, width: f64, height: f64) -> Style {
    let mut style = Style::default();
    style.layout.position = Position::Absolute;
    style.layout.width = Length::Points(width);
    style.layout.height = Length::Points(height);
    style.layout.inset.left = Length::Points(x);
    style.layout.inset.top = Length::Points(y);
    style
}
fn viewport() -> Size {
    Size::new(300.0, 200.0)
}
fn moved(x: f64, y: f64) -> WindowEvent {
    WindowEvent::PointerMoved {
        position: Point::new(x, y),
        timestamp: Duration::from_millis(1),
    }
}
fn button(x: f64, y: f64, pressed: bool) -> WindowEvent {
    WindowEvent::PointerButton {
        button: PointerButton::Primary,
        pressed,
        position: Point::new(x, y),
        timestamp: Duration::from_millis(2),
    }
}

#[test]
fn identities_and_atomic_reparenting() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let a = tree.append(root, Style::default()).unwrap();
    let b = tree.append(root, Style::default()).unwrap();
    let c = tree.append(root, Style::default()).unwrap();
    let d = tree.append(root, Style::default()).unwrap();
    tree.reparent(b, root, 3).unwrap();
    assert_eq!(tree.children(root).unwrap(), &[a, c, d, b]);
    assert!(matches!(
        tree.reparent(b, root, 4),
        Err(UiError::ChildIndex { .. })
    ));
    assert_eq!(tree.children(root).unwrap(), &[a, c, d, b]);
    tree.reparent(b, a, 0).unwrap();
    assert_eq!(tree.reparent(a, b, 0), Err(UiError::Cycle));
    assert_eq!(tree.parent(b).unwrap(), Some(a));
    tree.remove(a).unwrap();
    assert!(!tree.contains(a));
    assert!(!tree.contains(b));
    let replacement = tree.append(root, Style::default()).unwrap();
    assert_ne!(replacement, a);
    assert_eq!(tree.style(a), Err(UiError::StaleNode));
    let foreign = UiTree::new().unwrap();
    assert_eq!(tree.style(foreign.root()), Err(UiError::ForeignNode));
    assert_eq!(tree.remove(root), Err(UiError::RootOperation));
    tree.clear(root).unwrap();
    assert_eq!(tree.root(), root);
    assert_eq!(tree.len(), 1);
}

#[test]
fn depth_limit_rejects_before_mounting_and_deep_teardown_is_iterative() {
    let mut tree = UiTree::new().unwrap();
    let mut parent = tree.root();
    for _ in 1..MAX_DEPTH {
        parent = tree.append(parent, Style::default()).unwrap();
    }
    let count = tree.len();
    assert_eq!(
        tree.append(parent, Style::default()),
        Err(UiError::DepthLimit)
    );
    assert_eq!(tree.len(), count);
    tree.clear(tree.root()).unwrap();
    assert_eq!(tree.len(), 1);
}

#[test]
fn clean_and_paint_only_updates_skip_intrinsic_measurement() {
    let mut tree = UiTree::new().unwrap();
    let mut style = Style::default();
    style.layout.position = Position::Absolute;
    let child = tree.append(tree.root(), style).unwrap();
    let calls = Rc::new(Cell::new(0));
    let observed = Rc::clone(&calls);
    tree.set_measure(
        child,
        move |known: Dimensions<Option<f64>>, _: Dimensions<AvailableSpace>| {
            observed.set(observed.get() + 1);
            Dimensions {
                width: known.width.unwrap_or(60.0),
                height: known.height.unwrap_or(30.0),
            }
        },
    )
    .unwrap();
    tree.paint(viewport(), 1.0).unwrap();
    let measured = calls.get();
    assert!(measured > 0);
    tree.paint(viewport(), 1.0).unwrap();
    let mut style = tree.style(child).unwrap().clone();
    style.background = Color::from_rgba8(40, 90, 230, 255);
    tree.set_style(child, style).unwrap();
    tree.paint(viewport(), 1.0).unwrap();
    assert_eq!(calls.get(), measured);
    assert_eq!(tree.bounds(child).unwrap().size(), Size::new(60.0, 30.0));
}

#[test]
fn stacking_contexts_hit_transparent_nodes_and_apply_rounded_clips() {
    let mut tree = UiTree::new().unwrap();
    let mut wrapper_style = absolute(0.0, 0.0, 100.0, 100.0);
    wrapper_style.hit_test = false;
    let wrapper = tree.append(tree.root(), wrapper_style.clone()).unwrap();
    let mut inner_style = absolute(0.0, 0.0, 100.0, 100.0);
    inner_style.z_index = Some(9);
    let inner = tree.append(wrapper, inner_style).unwrap();
    let mut sibling_style = absolute(0.0, 0.0, 100.0, 100.0);
    sibling_style.z_index = Some(1);
    let sibling = tree.append(tree.root(), sibling_style).unwrap();
    tree.layout(viewport()).unwrap();
    assert_eq!(tree.hit_test(Point::new(50.0, 50.0)).unwrap(), Some(inner));
    wrapper_style.z_index = Some(0);
    tree.set_style(wrapper, wrapper_style.clone()).unwrap();
    assert_eq!(
        tree.hit_test(Point::new(50.0, 50.0)).unwrap(),
        Some(sibling)
    );
    tree.remove(sibling).unwrap();
    wrapper_style.overflow = Overflow::Clip;
    wrapper_style.border_radii = CornerRadii::uniform(30.0);
    tree.set_style(wrapper, wrapper_style).unwrap();
    tree.layout(viewport()).unwrap();
    assert_ne!(tree.hit_test(Point::new(1.0, 1.0)).unwrap(), Some(inner));
    assert_eq!(tree.hit_test(Point::new(50.0, 50.0)).unwrap(), Some(inner));
}

#[test]
fn boundary_deletion_cannot_retarget_the_same_down() {
    let mut tree = UiTree::new().unwrap();
    let q = tree
        .append(tree.root(), absolute(10.0, 10.0, 80.0, 60.0))
        .unwrap();
    let mut trace = Vec::new();
    let result = tree
        .dispatch(
            &button(20.0, 20.0, true),
            viewport(),
            |event, current, commands| {
                if event.current_target == q {
                    trace.push(event.kind);
                    if event.kind == PointerEventKind::Enter {
                        commands.remove(q);
                    }
                    if matches!(event.kind, PointerEventKind::Down | PointerEventKind::Leave) {
                        assert!(current.contains(q));
                    }
                }
            },
        )
        .unwrap();
    assert_eq!(
        trace,
        vec![
            PointerEventKind::Enter,
            PointerEventKind::Down,
            PointerEventKind::Leave
        ]
    );
    assert!(!tree.contains(q));
    assert!(
        result
            .reports
            .iter()
            .flat_map(|report| &report.outcomes)
            .all(|outcome| outcome.result.is_ok())
    );
}

#[test]
fn capture_routes_outside_and_final_up_releases_before_queued_removal() {
    let mut tree = UiTree::new().unwrap();
    let p = tree
        .append(tree.root(), absolute(10.0, 10.0, 80.0, 60.0))
        .unwrap();
    tree.dispatch(
        &button(20.0, 20.0, true),
        viewport(),
        |event, _, commands| {
            if event.current_target == p && event.kind == PointerEventKind::Down {
                commands.capture(p);
            }
        },
    )
    .unwrap();
    assert_eq!(tree.capture_holder(), Some(p));
    let mut targets = Vec::new();
    tree.dispatch(&moved(400.0, 300.0), viewport(), |event, _, _| {
        if event.kind == PointerEventKind::Move {
            targets.push(event.target);
        }
    })
    .unwrap();
    assert!(targets.iter().all(|id| *id == p));
    assert_eq!(tree.hover_target(), Some(p));
    let mut trace = Vec::new();
    tree.dispatch(
        &button(400.0, 300.0, false),
        viewport(),
        |event, _, commands| {
            if event.current_target == p {
                trace.push(event.kind);
                if event.kind == PointerEventKind::Up {
                    commands.remove(p);
                }
            }
        },
    )
    .unwrap();
    assert_eq!(trace, vec![PointerEventKind::Up, PointerEventKind::Leave]);
    assert_eq!(tree.capture_holder(), None);
    assert_eq!(tree.hover_target(), None);
    assert!(!tree.contains(p));
}

#[test]
fn cancel_reads_old_state_and_leave_reads_cleaned_state_before_removal() {
    let mut tree = UiTree::new().unwrap();
    let p = tree
        .append(tree.root(), absolute(10.0, 10.0, 80.0, 60.0))
        .unwrap();
    tree.dispatch(
        &button(20.0, 20.0, true),
        viewport(),
        |event, _, commands| {
            if event.current_target == p && event.kind == PointerEventKind::Down {
                commands.capture(p);
            }
        },
    )
    .unwrap();
    let mut trace = Vec::new();
    tree.dispatch(
        &moved(30.0, 20.0),
        viewport(),
        |event, current, commands| {
            if event.current_target != p {
                return;
            }
            trace.push(event.kind);
            match event.kind {
                PointerEventKind::Move => commands.remove(p),
                PointerEventKind::Cancel => {
                    assert_eq!(current.capture_holder(), Some(p));
                    assert_eq!(current.hover_target(), Some(p));
                    commands.stop_propagation();
                }
                PointerEventKind::Leave => {
                    assert_eq!(current.capture_holder(), None);
                    assert_ne!(current.hover_target(), Some(p));
                    assert!(current.contains(p));
                }
                _ => {}
            }
        },
    )
    .unwrap();
    assert_eq!(
        trace,
        vec![
            PointerEventKind::Move,
            PointerEventKind::Cancel,
            PointerEventKind::Leave
        ]
    );
    assert!(!tree.contains(p));
}

#[test]
fn failed_reserved_parent_invalidates_child_but_independent_edits_continue() {
    let mut tree = UiTree::new().unwrap();
    let r = tree
        .append(tree.root(), absolute(10.0, 10.0, 80.0, 60.0))
        .unwrap();
    let foreign = UiTree::new().unwrap();
    let mut ids = None;
    let result = tree
        .dispatch(
            &button(20.0, 20.0, true),
            viewport(),
            |event, current, commands| {
                if event.kind != PointerEventKind::Down || event.current_target != r {
                    return;
                }
                let p = commands.append(foreign.root(), Style::default()).unwrap();
                let q = commands.append(p, Style::default()).unwrap();
                assert_eq!(current.style(p), Err(UiError::NotYetCreated));
                commands.set_style(q, Style::default());
                let mut style = current.style(r).unwrap().clone();
                style.opacity = 0.5;
                commands.set_style(r, style);
                ids = Some((p, q));
            },
        )
        .unwrap();
    let (p, q) = ids.unwrap();
    assert_eq!(tree.style(p), Err(UiError::StaleNode));
    assert_eq!(tree.style(q), Err(UiError::StaleNode));
    let outcomes = &result.reports.last().unwrap().outcomes;
    assert_eq!(outcomes.len(), 4);
    assert!(
        outcomes
            .iter()
            .take(3)
            .all(|outcome| outcome.result.is_err())
    );
    assert!(outcomes.last().unwrap().result.is_ok());
    assert_eq!(tree.style(r).unwrap().opacity, 0.5);
}

#[test]
fn stationary_geometry_requires_opt_in_and_removed_enabled_nodes_still_trigger() {
    for enabled in [false, true] {
        let mut tree = UiTree::new().unwrap();
        let q = tree
            .append(tree.root(), absolute(0.0, 0.0, 80.0, 60.0))
            .unwrap();
        let mut p_style = absolute(0.0, 0.0, 80.0, 60.0);
        p_style.stationary_hover_updates = enabled;
        let p = tree.append(tree.root(), p_style).unwrap();
        tree.dispatch(&moved(20.0, 20.0), viewport(), |_, _, _| {})
            .unwrap();
        let mut entered_q = false;
        tree.dispatch(
            &button(20.0, 20.0, true),
            viewport(),
            |event, _, commands| {
                if event.current_target == p && event.kind == PointerEventKind::Down {
                    commands.remove(p);
                }
                if event.current_target == q && event.kind == PointerEventKind::Enter {
                    entered_q = true;
                }
            },
        )
        .unwrap();
        assert_eq!(entered_q, enabled);
    }
}

#[test]
fn idle_pointer_does_not_request_paint_and_interruption_is_not_duplicated() {
    let mut tree = UiTree::new().unwrap();
    tree.paint(viewport(), 1.0).unwrap();
    let result = tree
        .dispatch(&moved(20.0, 20.0), viewport(), |_, _, _| {})
        .unwrap();
    assert!(!result.changed);
    let interrupted = WindowEvent::PointerInterrupted {
        timestamp: Duration::from_millis(3),
    };
    let result = tree
        .dispatch(&interrupted, viewport(), |_, _, _| {})
        .unwrap();
    assert!(result.events > 0);
    assert_eq!(
        tree.dispatch(&interrupted, viewport(), |_, _, _| {})
            .unwrap()
            .events,
        0
    );
    assert!(tree.hover_path().is_empty());
}

#[test]
fn nested_opacity_contexts_apply_each_ancestor_clip_once() {
    use oxiview_scene::NodeKind;
    let mut tree = UiTree::new().unwrap();
    let mut root_style = tree.style(tree.root()).unwrap().clone();
    root_style.overflow = Overflow::Clip;
    tree.set_style(tree.root(), root_style).unwrap();
    let mut style = absolute(0.0, 0.0, 100.0, 100.0);
    style.opacity = 0.7;
    let a = tree.append(tree.root(), style.clone()).unwrap();
    let b = tree.append(a, style).unwrap();
    tree.append(b, absolute(0.0, 0.0, 50.0, 50.0)).unwrap();
    let painted = tree.paint(viewport(), 1.0).unwrap();
    let mut stack = vec![(painted.root, 0)];
    let mut max_clips = 0;
    while let Some((id, clips)) = stack.pop() {
        let clips = clips
            + usize::from(matches!(
                painted.scene.content(id).unwrap(),
                NodeKind::Clip(_)
            ));
        max_clips = max_clips.max(clips);
        for child in painted.scene.children(Some(id)).unwrap() {
            stack.push((*child, clips));
        }
    }
    assert_eq!(max_clips, 1);
}

#[test]
fn node_owned_resources_drop_in_child_list_postorder_with_root_retained() {
    struct Resource {
        name: &'static str,
        released: Rc<RefCell<Vec<&'static str>>>,
    }
    impl Resource {
        fn measure(&self) -> Dimensions<f64> {
            Dimensions {
                width: self.name.len() as f64,
                height: 1.0,
            }
        }
    }
    impl Drop for Resource {
        fn drop(&mut self) {
            self.released.borrow_mut().push(self.name);
        }
    }
    let released = Rc::new(RefCell::new(Vec::new()));
    let mut tree = UiTree::new().unwrap();
    let p = tree.append(tree.root(), Style::default()).unwrap();
    let q = tree.append(p, Style::default()).unwrap();
    let r = tree.append(p, Style::default()).unwrap();
    let s = tree.append(q, Style::default()).unwrap();
    for (id, name) in [
        (tree.root(), "root"),
        (p, "P"),
        (q, "Q"),
        (r, "R"),
        (s, "S"),
    ] {
        let resource = Resource {
            name,
            released: Rc::clone(&released),
        };
        tree.set_measure(
            id,
            move |_: Dimensions<Option<f64>>, _: Dimensions<AvailableSpace>| resource.measure(),
        )
        .unwrap();
    }
    tree.clear(tree.root()).unwrap();
    assert_eq!(*released.borrow(), vec!["S", "Q", "R", "P"]);
    drop(tree);
    assert_eq!(*released.borrow(), vec!["S", "Q", "R", "P", "root"]);
}

#[test]
fn cyclic_hover_mutations_stop_at_the_command_budget_with_reports() {
    let mut tree = UiTree::new().unwrap();
    let mut style = absolute(0.0, 0.0, 80.0, 60.0);
    style.stationary_hover_updates = true;
    let p = tree.append(tree.root(), style).unwrap();
    let result = tree
        .dispatch(
            &moved(20.0, 20.0),
            viewport(),
            |event, current, commands| {
                if event.current_target != p {
                    return;
                }
                if matches!(
                    event.kind,
                    PointerEventKind::Enter | PointerEventKind::Leave
                ) {
                    let mut style = current.style(p).unwrap().clone();
                    style.layout.display = if event.kind == PointerEventKind::Enter {
                        Display::None
                    } else {
                        Display::Flex
                    };
                    commands.set_style(p, style);
                }
            },
        )
        .unwrap();
    assert!(result.events <= 16384);
    let applied = result
        .reports
        .iter()
        .flat_map(|report| &report.outcomes)
        .filter(|outcome| outcome.result.is_ok())
        .count();
    assert_eq!(applied, 4096);
    assert!(
        result
            .reports
            .iter()
            .flat_map(|report| &report.outcomes)
            .any(|outcome| outcome.result == Err(UiError::BudgetExceeded))
    );
    assert!(tree.queue.is_empty());
    assert!(tree.contains(p));
}
