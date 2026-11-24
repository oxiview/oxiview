//! Focus and control interaction through the retained tree's public input path.
#![allow(clippy::unwrap_used)]

use oxiview_base::{Point, Size};
use oxiview_core::{
    Action, ActionSource, FocusEventKind, FocusPolicy, NodeId, PointerEventKind, UiError, UiTree,
    View, keyed_for, node, show,
};
use oxiview_platform::{KeyCode, KeyModifiers, PointerButton, WindowEvent};
use oxiview_reactive::{ReactiveError, Signal, batch, signal};
use oxiview_style::{Display, LayoutPatch, Length, Position, StateVariant, Style, StylePatch};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

const VIEWPORT: Size = Size::new(240.0, 120.0);

fn key(code: KeyCode, shift: bool, pressed: bool, repeat: bool) -> WindowEvent {
    WindowEvent::Key {
        code,
        logical: code.us_logical_key(),
        modifiers: KeyModifiers {
            shift,
            ..KeyModifiers::default()
        },
        pressed,
        repeat,
        timestamp: Duration::from_millis(10),
    }
}

fn dispatch(tree: &mut UiTree, event: WindowEvent) {
    tree.dispatch(&event, VIEWPORT, |_, _, _| {}).unwrap();
}

fn tab(tree: &mut UiTree, backwards: bool) {
    dispatch(tree, key(KeyCode::Tab, backwards, true, false));
}

fn tab_stop(tree: &mut UiTree, parent: NodeId) -> NodeId {
    tree.mount(parent, node().focusable(FocusPolicy::Tab))
        .unwrap()
}

fn box_at(x: f64) -> Style {
    let mut style = Style::default();
    style.layout.position = Position::Absolute;
    style.layout.inset.left = Length::Points(x);
    style.layout.width = Length::Points(50.0);
    style.layout.height = Length::Points(50.0);
    style
}

fn primary(x: f64, pressed: bool) -> WindowEvent {
    WindowEvent::PointerButton {
        position: Point::new(x, 10.0),
        pressed,
        button: PointerButton::Primary,
        timestamp: Duration::from_millis(12),
    }
}

#[test]
fn tab_uses_preorder_and_wraps_in_both_directions() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let a = tree
        .mount(
            root,
            node()
                .style(Style {
                    hit_test: false,
                    z_index: Some(20),
                    ..box_at(120.0)
                })
                .focusable(FocusPolicy::Tab),
        )
        .unwrap();
    let b = tab_stop(&mut tree, a);
    let programmatic = tree
        .mount(root, node().focusable(FocusPolicy::Programmatic))
        .unwrap();
    let c = tab_stop(&mut tree, root);
    let hidden = tab_stop(&mut tree, root);
    tree.set_visible(hidden, false).unwrap();
    let disabled = tab_stop(&mut tree, root);
    tree.set_disabled(disabled, true).unwrap();

    for expected in [a, b, c, a] {
        tab(&mut tree, false);
        assert_eq!(tree.focused_node(), Some(expected));
    }
    for expected in [c, b, a, c] {
        tab(&mut tree, true);
        assert_eq!(tree.focused_node(), Some(expected));
    }
    tree.clear_focus().unwrap();
    tab(&mut tree, true);
    assert_eq!(tree.focused_node(), Some(c));
    tree.focus(programmatic).unwrap();
    tab(&mut tree, true);
    assert_eq!(tree.focused_node(), Some(b));
    tree.focus(programmatic).unwrap();
    tab(&mut tree, false);
    assert_eq!(tree.focused_node(), Some(c));
    assert_eq!(tree.focus(disabled), Err(UiError::FocusUnavailable));
    assert_eq!(tree.focus(hidden), Err(UiError::FocusUnavailable));
    assert_eq!(tree.focus(root), Err(UiError::FocusUnavailable));
    let foreign = UiTree::new().unwrap();
    assert_eq!(tree.focus(foreign.root()), Err(UiError::ForeignNode));
}

#[test]
fn tab_order_follows_policy_changes_and_subtree_moves() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let first = tab_stop(&mut tree, root);
    let branch = tree.mount(root, node()).unwrap();
    let middle = tab_stop(&mut tree, branch);
    let last = tab_stop(&mut tree, root);
    tree.mount(root, node().focusable(FocusPolicy::Programmatic))
        .unwrap();

    tab(&mut tree, false);
    assert_eq!(tree.focused_node(), Some(first));
    tree.set_focus_policy(middle, FocusPolicy::Programmatic)
        .unwrap();
    tab(&mut tree, false);
    assert_eq!(tree.focused_node(), Some(last));

    tree.focus(middle).unwrap();
    tree.reparent(branch, root, 3).unwrap();
    assert_eq!(tree.focused_node(), Some(middle));
    tab(&mut tree, true);
    assert_eq!(tree.focused_node(), Some(last));
    tree.focus(middle).unwrap();
    tab(&mut tree, false);
    assert_eq!(tree.focused_node(), Some(first));

    tree.set_focus_policy(middle, FocusPolicy::Tab).unwrap();
    tab(&mut tree, true);
    assert_eq!(tree.focused_node(), Some(middle));
    tree.set_focus_policy(middle, FocusPolicy::None).unwrap();
    assert_eq!(tree.focused_node(), Some(last));
    let replacement = tab_stop(&mut tree, branch);
    tab(&mut tree, false);
    assert_eq!(tree.focused_node(), Some(replacement));
    tree.clear(branch).unwrap();
    assert_eq!(tree.focused_node(), Some(last));
    tab(&mut tree, false);
    assert_eq!(tree.focused_node(), Some(first));
}

#[test]
fn tab_order_follows_interaction_availability_in_ancestor_subtrees() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let first = tab_stop(&mut tree, root);
    let branch = tree.mount(root, node()).unwrap();
    let child = tab_stop(&mut tree, branch);
    let last = tab_stop(&mut tree, root);

    tab(&mut tree, false);
    tab(&mut tree, false);
    assert_eq!(tree.focused_node(), Some(child));

    tree.set_visible(branch, false).unwrap();
    assert_eq!(tree.focused_node(), Some(last));
    tab(&mut tree, true);
    assert_eq!(tree.focused_node(), Some(first));
    tree.set_visible(branch, true).unwrap();
    tab(&mut tree, false);
    assert_eq!(tree.focused_node(), Some(child));

    tree.set_disabled(branch, true).unwrap();
    assert_eq!(tree.focused_node(), Some(last));
    tab(&mut tree, true);
    assert_eq!(tree.focused_node(), Some(first));
    tree.set_disabled(branch, false).unwrap();
    tab(&mut tree, false);
    assert_eq!(tree.focused_node(), Some(child));

    let visible = tree.style(branch).unwrap().clone();
    let mut suppressed = visible.clone();
    suppressed.layout.display = Display::None;
    tree.set_style(branch, suppressed).unwrap();
    assert_eq!(tree.focused_node(), Some(last));
    tab(&mut tree, true);
    assert_eq!(tree.focused_node(), Some(first));
    tree.set_style(branch, visible).unwrap();
    tab(&mut tree, false);
    assert_eq!(tree.focused_node(), Some(child));
}

#[test]
fn focus_recovery_is_observable_before_disposal_and_follows_original_neighbors() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let focused_signal = tree.focused_signal().unwrap();
    let a = tab_stop(&mut tree, root);
    let b = tab_stop(&mut tree, root);
    let c = tab_stop(&mut tree, root);
    let trace = Rc::new(RefCell::new(Vec::new()));
    let local = Rc::new(Cell::new(None::<Signal<u32>>));
    let b_log = Rc::clone(&trace);
    let b_local = Rc::clone(&local);
    tree.set_focus_handler(b, move |event, view, commands| {
        assert!(view.contains(b));
        if event.kind == FocusEventKind::Blur {
            assert_eq!(focused_signal.get().unwrap(), Some(c));
            assert_eq!(view.focused_node(), Some(c));
            assert!(view.interaction_state(c).unwrap().focused);
            assert!(!view.interaction_state(b).unwrap().focused);
            b_log.borrow_mut().push(*event);
            b_local.set(Some(signal(17).unwrap()));
            commands.focus(b);
        }
    })
    .unwrap();
    let c_log = Rc::clone(&trace);
    tree.set_focus_handler(c, move |event, view, _| {
        if event.kind == FocusEventKind::Focus {
            assert!(view.contains(b));
            c_log.borrow_mut().push(*event);
        }
    })
    .unwrap();
    tree.focus(b).unwrap();
    tree.command_sender().remove(b).unwrap();
    let applied = tree.flush_updates(VIEWPORT, |_, _, _| {}).unwrap();
    assert_eq!(tree.focused_node(), Some(c));
    assert_eq!(focused_signal.get().unwrap(), Some(c));
    let trace = trace.borrow();
    assert_eq!(trace.len(), 2);
    assert_eq!(
        (trace[0].kind, trace[0].target, trace[0].related_target),
        (FocusEventKind::Blur, b, Some(c))
    );
    assert_eq!(
        (trace[1].kind, trace[1].target, trace[1].related_target),
        (FocusEventKind::Focus, c, Some(b))
    );
    assert_eq!(local.get().unwrap().get(), Err(ReactiveError::Disposed));
    assert_eq!(applied.reports.len(), 2);
    assert_eq!(applied.reports[0].outcomes[0].result, Ok(()));
    assert_eq!(
        applied.reports[1].outcomes[0].result,
        Err(UiError::StaleNode)
    );

    tree.remove(c).unwrap();
    assert_eq!(tree.focused_node(), Some(a));
    tree.remove(a).unwrap();
    assert_eq!(tree.focused_node(), None);
    let replacement = tab_stop(&mut tree, root);
    assert_ne!(replacement, a);
    assert_eq!(tree.focus(a), Err(UiError::StaleNode));
    assert_eq!(tree.focused_node(), None);
}

#[test]
fn hidden_and_disabled_ancestry_and_ineligible_reparent_recover_focus() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let first = tab_stop(&mut tree, root);
    let disabled = signal(false).unwrap();
    let parent = tree
        .mount(root, node().bind_disabled(move || disabled.get()))
        .unwrap();
    let middle = tab_stop(&mut tree, parent);
    let last = tab_stop(&mut tree, root);
    let middle_signal = tree.interaction_signal(middle).unwrap();
    tree.focus(middle).unwrap();
    disabled.set(true).unwrap();
    tree.flush_updates(VIEWPORT, |_, _, _| {}).unwrap();
    assert_eq!(tree.focused_node(), Some(last));
    assert!(middle_signal.get().unwrap().disabled);
    assert!(!middle_signal.get().unwrap().focused);
    assert_eq!(tree.focus(middle), Err(UiError::FocusUnavailable));
    disabled.set(false).unwrap();
    tree.flush_updates(VIEWPORT, |_, _, _| {}).unwrap();

    tree.focus(middle).unwrap();
    let mut hidden = tree.style(parent).unwrap().clone();
    hidden.layout.display = Display::None;
    tree.set_style(parent, hidden).unwrap();
    assert_eq!(tree.focused_node(), Some(last));
    assert_eq!(tree.focus(middle), Err(UiError::FocusUnavailable));
    tree.set_style(parent, Style::default()).unwrap();

    tree.reparent(middle, root, 2).unwrap();
    tree.focus(middle).unwrap();
    tree.set_disabled(parent, true).unwrap();
    tree.reparent(middle, parent, 0).unwrap();
    assert_eq!(tree.focused_node(), Some(last));
    assert!(middle_signal.get().unwrap().disabled);
    tree.set_focus_policy(last, FocusPolicy::None).unwrap();
    assert_eq!(tree.focused_node(), Some(first));
    tree.set_visible(root, false).unwrap();
    assert_eq!(tree.focused_node(), None);
}

#[test]
fn show_restores_to_the_eligible_opener_only_when_its_branch_loses_focus() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let opener = tree
        .mount(root, node().focusable(FocusPolicy::Programmatic))
        .unwrap();
    let show_setting = signal(true).unwrap();
    let settings = tree
        .mount(
            root,
            show(show_setting, move || {
                Ok(node()
                    .focus_return_target(opener)
                    .child(node().focusable(FocusPolicy::Tab)))
            }),
        )
        .unwrap();
    let outside = tab_stop(&mut tree, root);
    let setting = |tree: &UiTree| {
        let branch = tree.children(settings).unwrap()[0];
        tree.children(branch).unwrap()[0]
    };
    let initial = setting(&tree);
    tree.focus(initial).unwrap();
    show_setting.set(false).unwrap();
    tree.flush_updates(VIEWPORT, |_, _, _| {}).unwrap();
    assert_eq!(tree.focused_node(), Some(opener));
    assert!(!tree.contains(initial));
    show_setting.set(true).unwrap();
    tree.flush_updates(VIEWPORT, |_, _, _| {}).unwrap();
    tree.focus(outside).unwrap();
    show_setting.set(false).unwrap();
    tree.flush_updates(VIEWPORT, |_, _, _| {}).unwrap();
    assert_eq!(tree.focused_node(), Some(outside));
    show_setting.set(true).unwrap();
    tree.flush_updates(VIEWPORT, |_, _, _| {}).unwrap();
    tree.focus(setting(&tree)).unwrap();
    tree.set_disabled(opener, true).unwrap();
    show_setting.set(false).unwrap();
    tree.flush_updates(VIEWPORT, |_, _, _| {}).unwrap();
    assert_eq!(tree.focused_node(), Some(outside));
}

#[test]
fn keyed_reorder_retains_identity_and_recovery_skips_new_or_removed_rows() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let rows = signal(vec![1, 2, 3, 4]).unwrap();
    let container = tree
        .mount(
            root,
            keyed_for(rows, |row| *row, |_| Ok(node().focusable(FocusPolicy::Tab))),
        )
        .unwrap();
    let ids = tree.children(container).unwrap().to_vec();
    let (one, two, three, four) = (ids[0], ids[1], ids[2], ids[3]);
    tree.focus(two).unwrap();
    rows.set(vec![3, 2, 1, 4]).unwrap();
    tree.flush_updates(VIEWPORT, |_, _, _| {}).unwrap();
    assert_eq!(tree.children(container).unwrap(), &[three, two, one, four]);
    assert_eq!(tree.focused_node(), Some(two));
    tab(&mut tree, false);
    assert_eq!(tree.focused_node(), Some(one));
    rows.set(vec![3, 5]).unwrap();
    tree.flush_updates(VIEWPORT, |_, _, _| {}).unwrap();
    assert_eq!(tree.focused_node(), Some(three));
    rows.set(vec![6]).unwrap();
    tree.flush_updates(VIEWPORT, |_, _, _| {}).unwrap();
    assert_eq!(tree.focused_node(), None);
    tab(&mut tree, false);
    assert_eq!(
        tree.focused_node(),
        Some(tree.children(container).unwrap()[0])
    );
}

fn control(x: f64, sources: Rc<RefCell<Vec<ActionSource>>>) -> View {
    node()
        .style(box_at(x))
        .focusable(FocusPolicy::Tab)
        .activate_with("test.activate")
        .on_action(
            Action::new("test.activate", "Activate"),
            move |event, _, _| {
                sources.borrow_mut().push(event.source);
            },
        )
}

#[test]
fn wrapping_back_to_the_only_tab_stop_cancels_an_armed_key() {
    let mut tree = UiTree::new().unwrap();
    let sources = Rc::new(RefCell::new(Vec::new()));
    let stop = tree
        .mount(tree.root(), control(0.0, Rc::clone(&sources)))
        .unwrap();

    tab(&mut tree, false);
    dispatch(&mut tree, key(KeyCode::Space, false, true, false));
    assert!(tree.interaction_state(stop).unwrap().pressed);
    tab(&mut tree, true);
    assert_eq!(tree.focused_node(), Some(stop));
    assert!(!tree.interaction_state(stop).unwrap().pressed);
    dispatch(&mut tree, key(KeyCode::Space, false, false, false));
    assert!(sources.borrow().is_empty());
}

#[test]
fn press_state_and_keyboard_focus_follow_input_and_window_lifecycle() {
    let mut tree = UiTree::new().unwrap();
    let focused = tree.focused_signal().unwrap();
    let sources = Rc::new(RefCell::new(Vec::new()));
    let id = tree
        .mount(tree.root(), control(0.0, Rc::clone(&sources)))
        .unwrap();
    let watched = tree.interaction_signal(id).unwrap();
    dispatch(&mut tree, primary(10.0, true));
    assert_eq!(tree.focused_node(), Some(id));
    assert!(watched.get().unwrap().pressed && watched.get().unwrap().hovered);
    assert!(!watched.get().unwrap().focused);
    dispatch(&mut tree, primary(100.0, false));
    assert!(sources.borrow().is_empty());
    assert!(!watched.get().unwrap().pressed);

    dispatch(&mut tree, key(KeyCode::Space, false, true, false));
    assert!(watched.get().unwrap().focused && watched.get().unwrap().pressed);
    dispatch(
        &mut tree,
        WindowEvent::ActivationChanged {
            active: false,
            timestamp: Duration::ZERO,
        },
    );
    assert_eq!(tree.focused_node(), Some(id));
    assert!(!watched.get().unwrap().focused && !watched.get().unwrap().pressed);
    dispatch(
        &mut tree,
        WindowEvent::VisibilityChanged {
            visible: false,
            timestamp: Duration::ZERO,
        },
    );
    dispatch(
        &mut tree,
        WindowEvent::ActivationChanged {
            active: true,
            timestamp: Duration::ZERO,
        },
    );
    assert!(!watched.get().unwrap().focused);
    dispatch(
        &mut tree,
        WindowEvent::VisibilityChanged {
            visible: true,
            timestamp: Duration::ZERO,
        },
    );
    assert!(watched.get().unwrap().focused);
    dispatch(&mut tree, key(KeyCode::Space, false, false, false));
    assert!(sources.borrow().is_empty());
    dispatch(&mut tree, key(KeyCode::Space, false, true, false));
    dispatch(&mut tree, key(KeyCode::Space, false, true, true));
    assert!(sources.borrow().is_empty());
    dispatch(&mut tree, key(KeyCode::Space, false, false, false));
    dispatch(&mut tree, key(KeyCode::Enter, false, true, false));
    dispatch(&mut tree, key(KeyCode::Enter, false, true, true));
    dispatch(&mut tree, key(KeyCode::Enter, false, false, false));
    assert_eq!(sources.borrow().as_slice(), &[ActionSource::Keyboard; 2]);

    dispatch(&mut tree, primary(10.0, true));
    dispatch(
        &mut tree,
        WindowEvent::PointerInterrupted {
            timestamp: Duration::ZERO,
        },
    );
    dispatch(&mut tree, primary(10.0, false));
    assert_eq!(sources.borrow().len(), 2);
    dispatch(&mut tree, primary(10.0, true));
    dispatch(&mut tree, primary(10.0, false));
    assert_eq!(sources.borrow().last(), Some(&ActionSource::Pointer));
    dispatch(&mut tree, WindowEvent::Destroyed);
    assert_eq!(tree.focused_node(), None);
    assert_eq!(focused.get(), Err(ReactiveError::Disposed));
    assert_eq!(watched.get(), Err(ReactiveError::Disposed));
}

#[test]
fn pointer_route_survives_focus_requests_and_normal_release_precedes_activation() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let second = tree
        .mount(root, node().style(box_at(80.0)).focusable(FocusPolicy::Tab))
        .unwrap();
    let trace = Rc::new(RefCell::new(Vec::new()));
    let action_trace = Rc::clone(&trace);
    let pointer_trace = Rc::clone(&trace);
    let first = tree
        .mount(
            root,
            node()
                .style(box_at(0.0))
                .focusable(FocusPolicy::Tab)
                .on_pointer(move |event, view, commands| match event.kind {
                    PointerEventKind::Enter => {
                        commands.focus(second);
                        pointer_trace.borrow_mut().push("enter");
                    }
                    PointerEventKind::Down => {
                        assert_eq!(view.focused_node(), Some(event.current_target));
                        assert_eq!(event.target, event.current_target);
                        commands.capture(event.current_target);
                        pointer_trace.borrow_mut().push("down");
                    }
                    PointerEventKind::Up => {
                        assert_eq!(view.capture_holder(), Some(event.current_target));
                        commands.remove(event.current_target);
                        pointer_trace.borrow_mut().push("up");
                    }
                    PointerEventKind::Cancel => pointer_trace.borrow_mut().push("cancel"),
                    _ => {}
                })
                .on_action(
                    Action::new("test.activate", "Activate"),
                    move |event, view, _| {
                        assert!(view.contains(event.target));
                        assert_eq!(view.capture_holder(), None);
                        action_trace.borrow_mut().push("action");
                    },
                )
                .activate_with("test.activate"),
        )
        .unwrap();
    dispatch(&mut tree, primary(10.0, true));
    assert_eq!(tree.focused_node(), Some(second));
    dispatch(&mut tree, primary(10.0, false));
    assert!(!tree.contains(first));
    assert_eq!(trace.borrow().as_slice(), ["enter", "down", "up", "action"]);
}

#[test]
fn queued_suppression_and_clear_disarm_space_before_focus_recovers() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let trace = Rc::new(RefCell::new(Vec::new()));
    let container = tree.mount(root, node()).unwrap();
    let a = tree
        .mount(container, control(0.0, Rc::clone(&trace)))
        .unwrap();
    let b = tree
        .mount(container, control(60.0, Rc::clone(&trace)))
        .unwrap();
    let c = tree.mount(root, control(120.0, Rc::clone(&trace))).unwrap();
    tree.focus(a).unwrap();
    dispatch(&mut tree, key(KeyCode::Space, false, true, false));
    let mut suppressed = tree.style(a).unwrap().clone();
    suppressed.layout.display = Display::None;
    tree.command_sender().set_style(a, suppressed).unwrap();
    dispatch(&mut tree, key(KeyCode::Space, false, false, false));
    assert_eq!(tree.focused_node(), Some(b));
    assert!(trace.borrow().is_empty());
    assert!(!tree.interaction_state(a).unwrap().pressed);
    dispatch(&mut tree, key(KeyCode::Space, false, true, false));
    tree.clear(container).unwrap();
    assert_eq!(tree.focused_node(), Some(c));
    dispatch(&mut tree, key(KeyCode::Space, false, false, false));
    assert!(trace.borrow().is_empty());
}

#[test]
fn moving_an_armed_keyboard_control_retains_or_cancels_it_by_eligibility() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let trace = Rc::new(RefCell::new(Vec::new()));
    let branch = tree.mount(root, node()).unwrap();
    let focused = tree.mount(branch, control(0.0, Rc::clone(&trace))).unwrap();
    let next = tree.mount(root, control(60.0, Rc::clone(&trace))).unwrap();
    let eligible_parent = tree.mount(root, node()).unwrap();
    let disabled_parent = tree.mount(root, node().disabled(true)).unwrap();
    tree.focus(focused).unwrap();

    let down = key(KeyCode::Space, false, true, false);
    let up = key(KeyCode::Space, false, false, false);
    dispatch(&mut tree, down.clone());
    tree.reparent(branch, eligible_parent, 0).unwrap();
    assert_eq!(tree.focused_node(), Some(focused));
    assert!(tree.interaction_state(focused).unwrap().pressed);
    dispatch(&mut tree, up.clone());
    assert_eq!(trace.borrow().as_slice(), [ActionSource::Keyboard]);

    dispatch(&mut tree, down);
    tree.command_sender()
        .reparent(branch, disabled_parent, 0)
        .unwrap();
    dispatch(&mut tree, up);
    assert_eq!(tree.focused_node(), Some(next));
    assert!(!tree.interaction_state(focused).unwrap().pressed);
    assert_eq!(trace.borrow().as_slice(), [ActionSource::Keyboard]);
}

#[test]
fn effective_state_style_suppression_recovers_focus_before_the_next_input() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let hide_on_focus = || {
        node().focusable(FocusPolicy::Tab).style_variant(
            StateVariant::Focused,
            StylePatch {
                layout: LayoutPatch {
                    display: Some(Display::None),
                    ..LayoutPatch::default()
                },
                ..StylePatch::default()
            },
        )
    };
    let first = tree.mount(root, hide_on_focus()).unwrap();
    let second = tree.mount(root, hide_on_focus()).unwrap();
    let next = tab_stop(&mut tree, root);
    batch(|| {
        tree.focus(first).unwrap();
        assert_eq!(tree.focused_node(), Some(next));
        Ok(())
    })
    .unwrap();
    for id in [first, second] {
        assert!(!tree.interaction_state(id).unwrap().focused);
        assert_ne!(tree.style(id).unwrap().layout.display, Display::None);
    }
}

#[test]
fn queued_focus_uses_styles_from_the_preceding_command() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let parent = tree.mount(root, node()).unwrap();
    let child = tree
        .mount(
            parent,
            node().focusable(FocusPolicy::Tab).style_variant(
                StateVariant::Disabled,
                StylePatch {
                    layout: LayoutPatch {
                        display: Some(Display::None),
                        ..LayoutPatch::default()
                    },
                    ..StylePatch::default()
                },
            ),
        )
        .unwrap();
    tree.set_disabled(parent, true).unwrap();
    assert_eq!(tree.style(child).unwrap().layout.display, Display::None);

    let sender = tree.command_sender();
    sender.set_disabled(parent, false).unwrap();
    sender.focus(child).unwrap();
    let result = tree.flush_updates(VIEWPORT, |_, _, _| {}).unwrap();
    assert_eq!(result.reports.len(), 1);
    assert_eq!(result.reports[0].outcomes.len(), 2);
    assert_eq!(result.reports[0].outcomes[1].result, Ok(()));
    assert_eq!(tree.focused_node(), Some(child));
}

#[test]
fn state_style_suppression_reports_capture_cleanup_to_the_dispatch_observer() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let captured = tree
        .mount(
            root,
            node()
                .style(box_at(0.0))
                .focusable(FocusPolicy::Tab)
                .style_variant(
                    StateVariant::Focused,
                    StylePatch {
                        layout: LayoutPatch {
                            display: Some(Display::None),
                            ..LayoutPatch::default()
                        },
                        ..StylePatch::default()
                    },
                )
                .on_pointer(|event, _, commands| {
                    if event.kind == PointerEventKind::Down {
                        commands.capture(event.target);
                    }
                }),
        )
        .unwrap();
    let next = tab_stop(&mut tree, root);
    dispatch(&mut tree, primary(10.0, true));
    assert_eq!(tree.capture_holder(), Some(captured));

    let mut observed = Vec::new();
    let result = tree
        .dispatch(
            &key(KeyCode::F6, false, true, false),
            VIEWPORT,
            |event, tree, commands| {
                observed.push((event.current_target, event.kind));
                assert!(tree.contains(captured));
                assert_ne!(tree.style(captured).unwrap().layout.display, Display::None);
                match event.kind {
                    PointerEventKind::Cancel => {
                        assert_eq!(tree.capture_holder(), Some(captured));
                        assert_eq!(tree.hover_target(), Some(captured));
                    }
                    PointerEventKind::Leave => {
                        assert_eq!(tree.capture_holder(), None);
                        assert_ne!(tree.hover_target(), Some(captured));
                        commands.remove(captured);
                    }
                    _ => {}
                }
            },
        )
        .unwrap();
    assert_eq!(
        observed,
        [
            (captured, PointerEventKind::Cancel),
            (root, PointerEventKind::Cancel),
            (captured, PointerEventKind::Leave),
        ]
    );
    assert_eq!(result.events, observed.len());
    assert_eq!(result.reports[0].outcomes[0].result, Ok(()));
    assert!(!tree.contains(captured));
    assert_eq!(tree.focused_node(), Some(next));
}

#[test]
fn queued_style_preserves_capture_when_an_active_variant_keeps_the_node_visible() {
    let mut tree = UiTree::new().unwrap();
    let trace = Rc::new(RefCell::new(Vec::new()));
    let id = tree
        .mount(
            tree.root(),
            control(0.0, Rc::clone(&trace)).style_variant(
                StateVariant::Hovered,
                StylePatch {
                    layout: LayoutPatch {
                        display: Some(Display::Block),
                        ..LayoutPatch::default()
                    },
                    ..StylePatch::default()
                },
            ),
        )
        .unwrap();
    let mut observed = Vec::new();
    tree.dispatch(&primary(10.0, true), VIEWPORT, |event, view, commands| {
        if event.current_target == id {
            observed.push(event.kind);
            if event.kind == PointerEventKind::Down {
                commands.capture(id);
                let mut style = view.style(id).unwrap().clone();
                style.layout.display = Display::None;
                commands.set_style(id, style);
            }
        }
    })
    .unwrap();
    assert_eq!(observed, [PointerEventKind::Enter, PointerEventKind::Down]);
    assert_eq!(tree.capture_holder(), Some(id));
    assert_eq!(tree.focused_node(), Some(id));
    assert_eq!(tree.style(id).unwrap().layout.display, Display::Block);
    dispatch(&mut tree, primary(10.0, false));
    assert_eq!(trace.borrow().as_slice(), [ActionSource::Pointer]);
}

#[test]
fn capture_keeps_hover_but_pointer_activation_tracks_geometric_reentry() {
    let mut tree = UiTree::new().unwrap();
    let trace = Rc::new(RefCell::new(Vec::new()));
    let id = tree
        .mount(
            tree.root(),
            control(0.0, Rc::clone(&trace)).on_pointer(|event, _, commands| {
                if event.kind == PointerEventKind::Down {
                    commands.capture(event.current_target);
                }
            }),
        )
        .unwrap();
    let moved = |x| WindowEvent::PointerMoved {
        position: Point::new(x, 10.0),
        timestamp: Duration::from_millis(20),
    };
    dispatch(&mut tree, primary(10.0, true));
    dispatch(&mut tree, moved(100.0));
    let state = tree.interaction_state(id).unwrap();
    assert!(state.hovered);
    assert!(!state.pressed);
    dispatch(&mut tree, primary(100.0, false));
    assert!(trace.borrow().is_empty());
    dispatch(&mut tree, primary(10.0, true));
    dispatch(&mut tree, moved(100.0));
    dispatch(&mut tree, moved(10.0));
    assert!(tree.interaction_state(id).unwrap().pressed);
    dispatch(&mut tree, primary(10.0, false));
    assert_eq!(trace.borrow().as_slice(), [ActionSource::Pointer]);
}
