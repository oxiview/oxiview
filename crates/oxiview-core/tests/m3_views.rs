//! Public Builder, reactive lifetime, keyed reconciliation, and callback contracts.
// Integration fixtures use assertions and unwraps to identify broken contracts.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use oxiview_base::{Color, Point, Size};
use oxiview_core::{PointerEventKind, UiError, UiTree, component, keyed_for, node, show};
use oxiview_platform::{PointerButton, WindowEvent};
use oxiview_reactive::{ReactiveError, Scope, Signal, signal};
use oxiview_style::{Length, Position, Style};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};

fn viewport() -> Size {
    Size::new(500.0, 300.0)
}
fn style(width: f64) -> Style {
    let mut style = Style::default();
    style.layout.position = Position::Absolute;
    style.layout.width = Length::Points(width);
    style.layout.height = Length::Points(40.0);
    style.background = Color::from_rgba8(50, 100, 180, 255);
    style
}
fn down() -> WindowEvent {
    WindowEvent::PointerButton {
        button: PointerButton::Primary,
        pressed: true,
        position: Point::new(10.0, 10.0),
        timestamp: Duration::from_millis(1),
    }
}
fn ui(error: ReactiveError) -> UiError {
    UiError::Reactive(error.to_string())
}

#[test]
fn keyed_for_retries_a_failed_item_binding_without_remounting_new_rows() {
    let scope = Scope::new().unwrap();
    let rows = scope.run(|| signal(vec![(1_u8, 10_i32)])).unwrap();
    let view = keyed_for(
        rows,
        |row| row.0,
        |row| {
            Ok(node().bind_style(move || {
                let (_, width) = row.get()?;
                if width < 0 {
                    return Err(ReactiveError::User("negative row width".into()));
                }
                Ok(style(f64::from(width)))
            }))
        },
    );
    let mut tree = UiTree::new().unwrap();
    let container = tree.mount(tree.root(), view).unwrap();
    let first = tree.children(container).unwrap()[0];

    rows.set(vec![(2, 20), (1, -1)]).unwrap();
    let failed = tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    assert!(
        failed.reports.iter().flat_map(|report| &report.outcomes)
            .any(|outcome| matches!(&outcome.result, Err(UiError::Reactive(message)) if message.contains("negative row width")))
    );
    let second = tree.children(container).unwrap()[0];
    assert_eq!(tree.children(container).unwrap(), &[second, first]);

    rows.set(vec![(1, 30), (2, 40)]).unwrap();
    let recovered = tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    assert!(
        recovered
            .reports
            .iter()
            .flat_map(|report| &report.outcomes)
            .all(|outcome| outcome.result.is_ok())
    );
    assert_eq!(tree.children(container).unwrap(), &[first, second]);
    assert_eq!(
        tree.style(first).unwrap().layout.width,
        Length::Points(30.0)
    );
    assert_eq!(
        tree.style(second).unwrap().layout.width,
        Length::Points(40.0)
    );
}

#[test]
fn keyed_for_discards_staged_rows_when_a_later_factory_fails() {
    let scope = Scope::new().unwrap();
    let rows = scope.run(|| signal(vec![1_u8])).unwrap();
    let staged = Rc::new(Cell::new(None));
    let observed = Rc::clone(&staged);
    let view = keyed_for(
        rows,
        |value| *value,
        move |item| {
            match item.get().map_err(ui)? {
                2 => observed.set(Some(item)),
                3 => return Err(UiError::Reactive("row factory failed".into())),
                _ => {}
            }
            Ok(node())
        },
    );
    let mut tree = UiTree::new().unwrap();
    let container = tree.mount(tree.root(), view).unwrap();
    let original = tree.children(container).unwrap()[0];
    rows.set(vec![2, 3]).unwrap();
    let failed = tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    assert!(
        failed
            .reports
            .iter()
            .flat_map(|report| &report.outcomes)
            .any(|outcome| outcome.result.is_err())
    );
    assert_eq!(tree.children(container).unwrap(), &[original]);
    assert_eq!(staged.get().unwrap().get(), Err(ReactiveError::Disposed));
    rows.set(vec![2, 1]).unwrap();
    let recovered = tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    assert!(
        recovered
            .reports
            .iter()
            .flat_map(|report| &report.outcomes)
            .all(|outcome| outcome.result.is_ok())
    );
    assert_eq!(tree.children(container).unwrap().len(), 2);
    assert_eq!(tree.children(container).unwrap()[1], original);
    assert_eq!(staged.get().unwrap().get(), Ok(2));
}

#[test]
fn function_component_runs_once_and_binding_disposes_with_node() {
    let app_scope = Scope::new().unwrap();
    let width = app_scope.run(|| signal(80.0)).unwrap();
    let runs = Rc::new(Cell::new(0));
    let reads = Rc::new(Cell::new(0));
    let local = Rc::new(Cell::new(None::<Signal<i32>>));
    let mut tree = UiTree::new().unwrap();
    let observed_runs = Rc::clone(&runs);
    let observed_reads = Rc::clone(&reads);
    let observed_local = Rc::clone(&local);
    let view = component(move || {
        observed_runs.set(observed_runs.get() + 1);
        observed_local.set(Some(signal(7).map_err(ui)?));
        Ok(node().bind_style(move || {
            observed_reads.set(observed_reads.get() + 1);
            Ok(style(width.get()?))
        }))
    });
    let id = tree.mount(tree.root(), view).unwrap();
    assert_eq!(runs.get(), 1);
    assert_eq!(tree.style(id).unwrap().layout.width, Length::Points(80.0));
    width.set(120.0).unwrap();
    assert_eq!(tree.style(id).unwrap().layout.width, Length::Points(80.0));
    tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    assert_eq!(tree.style(id).unwrap().layout.width, Length::Points(120.0));
    assert_eq!(runs.get(), 1);
    tree.remove(id).unwrap();
    assert_eq!(local.get().unwrap().get(), Err(ReactiveError::Disposed));
    let count = reads.get();
    width.set(160.0).unwrap();
    tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    assert_eq!(reads.get(), count);
}

#[test]
fn show_releases_hidden_branch_and_remounts_only_on_transition() {
    let scope = Scope::new().unwrap();
    let visible = scope.run(|| signal(true)).unwrap();
    let created = Rc::new(Cell::new(0));
    let local = Rc::new(Cell::new(None::<Signal<bool>>));
    let made = Rc::clone(&created);
    let made_local = Rc::clone(&local);
    let view = show(visible, move || {
        made.set(made.get() + 1);
        made_local.set(Some(signal(true).map_err(ui)?));
        Ok(node().style(style(90.0)))
    });
    let mut tree = UiTree::new().unwrap();
    let container = tree.mount(tree.root(), view).unwrap();
    let first = tree.children(container).unwrap()[0];
    let first_local = local.get().unwrap();
    visible.set(true).unwrap();
    tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    assert_eq!(tree.children(container).unwrap(), &[first]);
    assert_eq!(created.get(), 1);
    visible.set(false).unwrap();
    tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    assert!(!tree.contains(first));
    assert!(tree.children(container).unwrap().is_empty());
    assert_eq!(first_local.get(), Err(ReactiveError::Disposed));
    visible.set(true).unwrap();
    tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    assert_ne!(tree.children(container).unwrap()[0], first);
    assert_eq!(created.get(), 2);
    tree.remove(container).unwrap();
    visible.set(false).unwrap();
    assert!(
        tree.flush_updates(viewport(), |_, _, _| {})
            .unwrap()
            .reports
            .is_empty()
    );
}

#[test]
fn keyed_for_preserves_ids_and_item_signals_and_rejects_duplicates() {
    let scope = Scope::new().unwrap();
    let rows = scope
        .run(|| signal(vec![("a", 80.0), ("b", 90.0)]))
        .unwrap();
    let records = Rc::new(RefCell::new(HashMap::new()));
    let made = Rc::new(Cell::new(0));
    let recorded = Rc::clone(&records);
    let created = Rc::clone(&made);
    let view = keyed_for(
        rows,
        |item| item.0,
        move |item| {
            created.set(created.get() + 1);
            let name = item.get().map_err(ui)?.0;
            let mounted = Rc::clone(&recorded);
            Ok(node()
                .bind_style(move || Ok(style(item.get()?.1)))
                .on_mount(move |id| {
                    mounted.borrow_mut().insert(name, (id, item));
                }))
        },
    );
    let mut tree = UiTree::new().unwrap();
    let container = tree.mount(tree.root(), view).unwrap();
    let a = records.borrow()["a"];
    let b = records.borrow()["b"];
    rows.set(vec![("b", 130.0), ("a", 80.0), ("c", 50.0)])
        .unwrap();
    tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    let c = records.borrow()["c"];
    assert_eq!(tree.children(container).unwrap(), &[b.0, a.0, c.0]);
    assert_eq!(tree.style(b.0).unwrap().layout.width, Length::Points(130.0));
    assert_eq!(made.get(), 3);
    rows.set(vec![("b", 140.0), ("c", 50.0)]).unwrap();
    tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    assert!(!tree.contains(a.0));
    assert_eq!(a.1.get(), Err(ReactiveError::Disposed));
    assert_eq!(tree.children(container).unwrap(), &[b.0, c.0]);
    rows.set(vec![("b", 1.0), ("b", 2.0)]).unwrap();
    let result = tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    assert!(
        result
            .reports
            .iter()
            .flat_map(|report| &report.outcomes)
            .any(|outcome| outcome.result == Err(UiError::DuplicateKey))
    );
    assert_eq!(tree.children(container).unwrap(), &[b.0, c.0]);
    assert_eq!(tree.style(b.0).unwrap().layout.width, Length::Points(140.0));
}

#[test]
fn node_handler_signal_writes_share_submission_order_and_reports_are_deferred() {
    let scope = Scope::new().unwrap();
    let width = scope.run(|| signal(80.0)).unwrap();
    let mut tree = UiTree::new().unwrap();
    let id = tree
        .mount(
            tree.root(),
            node()
                .bind_style(move || Ok(style(width.get()?)))
                .on_pointer(move |event, current, commands| {
                    if event.kind == PointerEventKind::Down {
                        commands.set_style(event.current_target, style(100.0));
                        width.set(120.0).unwrap();
                        commands.set_style(event.current_target, style(140.0));
                        assert_eq!(
                            current.style(event.current_target).unwrap().layout.width,
                            Length::Points(80.0)
                        );
                    }
                }),
        )
        .unwrap();
    let reports = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&reports);
    tree.set_batch_report_handler(move |report, current, commands| {
        observed.borrow_mut().push((
            report.outcomes.len(),
            current.style(id).unwrap().layout.width,
        ));
        if observed.borrow().len() == 1 {
            commands.set_style(id, style(160.0));
        }
    })
    .unwrap();
    let result = tree
        .dispatch(&down(), viewport(), |event, current, _| {
            if event.kind == PointerEventKind::Down {
                assert_eq!(
                    current.style(id).unwrap().layout.width,
                    Length::Points(80.0)
                );
            }
        })
        .unwrap();
    assert_eq!(
        *reports.borrow(),
        vec![(3, Length::Points(140.0)), (1, Length::Points(160.0))]
    );
    assert_eq!(result.reports.len(), 2);
    assert_eq!(tree.style(id).unwrap().layout.width, Length::Points(160.0));
}

#[test]
fn shutdown_cancels_pending_creation_and_terminal_reports_cannot_restart_work() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let sender = tree.command_sender();
    let reserved = sender.append(root, Style::default()).unwrap();
    let callbacks = Rc::new(Cell::new(0));
    let observed = Rc::clone(&callbacks);
    tree.set_batch_report_handler(move |_, current, commands| {
        observed.set(observed.get() + 1);
        assert!(!current.contains(reserved));
        assert_eq!(
            commands.append(root, Style::default()),
            Err(UiError::QueueClosed)
        );
        commands.set_style(root, Style::default());
    })
    .unwrap();
    let result = tree
        .dispatch(&WindowEvent::Destroyed, viewport(), |_, _, _| {})
        .unwrap();
    assert_eq!(callbacks.get(), 1);
    assert_eq!(result.reports.len(), 2);
    assert!(
        result
            .reports
            .iter()
            .flat_map(|report| &report.outcomes)
            .all(|outcome| outcome.result == Err(UiError::QueueClosed))
    );
    assert_eq!(
        sender.set_style(root, Style::default()),
        Err(UiError::QueueClosed)
    );
    assert!(tree.is_empty());
}

#[test]
fn invalid_initial_binding_rolls_back_new_subtree_and_disposes_its_scope() {
    let mut tree = UiTree::new().unwrap();
    let existing = tree.append(tree.root(), Style::default()).unwrap();
    let local = Rc::new(Cell::new(None::<Signal<f64>>));
    let saved = Rc::clone(&local);
    let view = component(move || {
        let width = signal(f64::NAN).map_err(ui)?;
        saved.set(Some(width));
        Ok(node().bind_style(move || Ok(style(width.get()?))))
    });
    assert!(tree.mount(tree.root(), view).is_err());
    assert_eq!(tree.children(tree.root()).unwrap(), &[existing]);
    assert_eq!(local.get().unwrap().get(), Err(ReactiveError::Disposed));
}

#[test]
fn recursive_component_descriptions_are_rejected_before_stack_exhaustion() {
    fn nested(depth: usize) -> oxiview_core::View {
        component(move || {
            Ok(if depth == 0 {
                node()
            } else {
                nested(depth - 1)
            })
        })
    }
    let mut tree = UiTree::new().unwrap();
    assert_eq!(
        tree.mount(tree.root(), nested(100)),
        Err(UiError::DepthLimit)
    );
    assert_eq!(tree.len(), 1);
}

#[test]
fn hiding_show_in_the_same_reactive_batch_cancels_its_queued_bindings() {
    for hide_first in [false, true] {
        let scope = Scope::new().unwrap();
        let visible = scope.run(|| signal(true)).unwrap();
        let width = scope.run(|| signal(80.0)).unwrap();
        let child_local = Rc::new(Cell::new(None::<Signal<i32>>));
        let saved = Rc::clone(&child_local);
        let mut tree = UiTree::new().unwrap();
        let container = tree
            .mount(
                tree.root(),
                show(visible, move || {
                    saved.set(Some(signal(9).map_err(ui)?));
                    Ok(node().bind_style(move || Ok(style(width.get()?))))
                }),
            )
            .unwrap();
        let child = tree.children(container).unwrap()[0];
        let survivor = tree
            .mount(
                tree.root(),
                node().bind_style(move || Ok(style(width.get()?))),
            )
            .unwrap();
        oxiview_reactive::batch(|| {
            if hide_first {
                visible.set(false)?;
                width.set(120.0)?;
            } else {
                width.set(120.0)?;
                visible.set(false)?;
            }
            Ok(())
        })
        .unwrap();
        let result = tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
        assert!(
            result
                .reports
                .iter()
                .flat_map(|report| &report.outcomes)
                .all(|outcome| outcome.result.is_ok()),
            "ordinary branch disposal must not reject its own automatic bindings"
        );
        if hide_first {
            assert!(
                result
                    .reports
                    .iter()
                    .map(|report| report.canceled_bindings)
                    .sum::<usize>()
                    > 0
            );
        }
        assert!(!tree.contains(child));
        assert_eq!(
            child_local.get().unwrap().get(),
            Err(ReactiveError::Disposed)
        );
        assert_eq!(
            tree.style(survivor).unwrap().layout.width,
            Length::Points(120.0)
        );
        let sender = tree.command_sender();
        sender.set_style(child, style(160.0)).unwrap();
        let explicit = tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
        assert!(
            explicit
                .reports
                .iter()
                .flat_map(|report| &report.outcomes)
                .any(|outcome| outcome.result == Err(UiError::StaleNode)),
            "an explicit stale-node mutation must still be rejected"
        );
    }
}

#[test]
fn resources_created_by_a_node_handler_belong_to_its_mount_scope() {
    let created = Rc::new(Cell::new(None::<Signal<i32>>));
    let observed = Rc::clone(&created);
    let mut tree = UiTree::new().unwrap();
    let id = tree
        .mount(
            tree.root(),
            node().style(style(80.0)).on_pointer(move |event, _, _| {
                if event.kind == PointerEventKind::Down {
                    observed.set(Some(signal(17).unwrap()));
                }
            }),
        )
        .unwrap();
    tree.dispatch(&down(), viewport(), |_, _, _| {}).unwrap();
    assert_eq!(created.get().unwrap().get().unwrap(), 17);
    tree.remove(id).unwrap();
    assert_eq!(created.get().unwrap().get(), Err(ReactiveError::Disposed));
}

#[test]
fn automatic_binding_updates_during_shutdown_are_canceled_without_rejecting_state_cleanup() {
    let scope = Scope::new().unwrap();
    let pressed = scope.run(|| signal(false)).unwrap();
    let mut tree = UiTree::new().unwrap();
    let id = tree
        .mount(
            tree.root(),
            node()
                .bind_style(move || Ok(style(if pressed.get()? { 100.0 } else { 80.0 })))
                .on_pointer(move |event, _, commands| match event.kind {
                    PointerEventKind::Down => {
                        pressed.set(true).unwrap();
                        commands.capture(event.current_target);
                    }
                    PointerEventKind::Cancel => pressed.set(false).unwrap(),
                    _ => {}
                }),
        )
        .unwrap();
    tree.dispatch(&down(), viewport(), |_, _, _| {}).unwrap();
    assert_eq!(tree.capture_holder(), Some(id));
    let sender = tree.command_sender();
    let result = tree
        .dispatch(&WindowEvent::Destroyed, viewport(), |_, _, _| {})
        .unwrap();
    assert!(!pressed.get().unwrap());
    assert!(
        result
            .reports
            .iter()
            .flat_map(|report| &report.outcomes)
            .all(|outcome| outcome.result.is_ok())
    );
    assert!(
        result
            .reports
            .iter()
            .map(|report| report.canceled_bindings)
            .sum::<usize>()
            > 0
    );
    assert_eq!(sender.set_style(id, style(80.0)), Err(UiError::QueueClosed));
}
