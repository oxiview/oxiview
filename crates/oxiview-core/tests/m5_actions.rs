//! Keymap resolution and Action delivery through UiTree dispatch.
#![allow(clippy::unwrap_used)]

use oxiview_base::Size;
use oxiview_core::{
    Action, ActionEvent, ActionSource, FocusPolicy, KEY_SEQUENCE_TIMEOUT, KeyBinding, KeyChord,
    Keymap, KeymapError, NodeId, NodeSemantics, UiDispatchResult, UiError, UiTree, node,
};
use oxiview_platform::{KeyCode, KeyModifiers, LogicalKey, WindowEvent};
use std::{cell::RefCell, rc::Rc, time::Duration};

const VIEWPORT: Size = Size::new(240.0, 120.0);
const PLAIN: KeyModifiers = KeyModifiers {
    shift: false,
    alt: false,
    meta: false,
    control: false,
};
const COMMAND: KeyModifiers = KeyModifiers {
    meta: true,
    ..PLAIN
};
const CONTROL: KeyModifiers = KeyModifiers {
    control: true,
    ..PLAIN
};

fn logical(code: KeyCode, modifiers: KeyModifiers) -> KeyChord {
    KeyChord::logical(code.us_logical_key(), modifiers)
}

fn binding(id: &str, keys: impl IntoIterator<Item = KeyCode>) -> KeyBinding {
    KeyBinding::new(id, keys.into_iter().map(|code| logical(code, PLAIN)))
}

fn transition(code: KeyCode, pressed: bool, repeat: bool, ms: u64) -> WindowEvent {
    WindowEvent::Key {
        code,
        logical: code.us_logical_key(),
        modifiers: PLAIN,
        pressed,
        repeat,
        timestamp: Duration::from_millis(ms),
    }
}

fn dispatch(tree: &mut UiTree, event: WindowEvent) -> UiDispatchResult {
    tree.dispatch(&event, VIEWPORT, |_, _, _| {}).unwrap()
}

fn press(tree: &mut UiTree, key: KeyCode, ms: u64) -> UiDispatchResult {
    dispatch(tree, transition(key, true, false, ms))
}

fn map(tree: &mut UiTree, node: NodeId, bindings: impl IntoIterator<Item = KeyBinding>) {
    tree.set_keymap(node, Keymap::new(bindings).unwrap())
        .unwrap();
}

fn record(tree: &mut UiTree, target: NodeId, id: &str, trace: &Rc<RefCell<Vec<ActionEvent>>>) {
    let trace = Rc::clone(trace);
    tree.register_action(target, Action::new(id, id), move |event, _, _| {
        trace.borrow_mut().push(event.clone());
    })
    .unwrap();
}

fn action_names(trace: &Rc<RefCell<Vec<ActionEvent>>>) -> Vec<String> {
    trace
        .borrow()
        .iter()
        .map(|event| event.action.clone())
        .collect()
}

#[test]
fn nested_contexts_shadow_outer_bindings_and_route_one_named_handler() {
    use KeyCode::{ArrowLeft, ArrowRight, KeyM, KeyQ};

    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let board = tree
        .mount(root, node().focusable(FocusPolicy::Programmatic))
        .unwrap();
    let settings = tree
        .mount(board, node().focusable(FocusPolicy::Tab))
        .unwrap();
    let trace = Rc::new(RefCell::new(Vec::new()));
    record(&mut tree, root, "test.menu", &trace);
    record(&mut tree, root, "test.outer", &trace);
    record(&mut tree, board, "test.right", &trace);
    record(&mut tree, settings, "test.right", &trace);
    map(
        &mut tree,
        root,
        [
            KeyBinding::new("test.menu", [logical(KeyM, COMMAND)]),
            KeyBinding::new("test.menu", [logical(KeyM, CONTROL)]),
            binding("test.outer", [ArrowLeft]),
        ],
    );
    map(&mut tree, board, [binding("test.right", [ArrowRight])]);
    map(
        &mut tree,
        settings,
        [
            binding("test.right", [ArrowRight]),
            binding("test.missing", [ArrowLeft]),
        ],
    );
    tree.focus(settings).unwrap();
    assert!(press(&mut tree, ArrowRight, 5).key_handled);
    assert!(press(&mut tree, ArrowLeft, 6).key_handled);
    for modifiers in [COMMAND, CONTROL] {
        assert!(
            dispatch(
                &mut tree,
                WindowEvent::Key {
                    code: KeyQ,
                    logical: LogicalKey::Character("m".into()),
                    modifiers,
                    pressed: true,
                    repeat: false,
                    timestamp: Duration::from_millis(9),
                }
            )
            .key_handled
        );
    }
    assert!(!press(&mut tree, KeyM, 10).key_handled);
    tree.focus(board).unwrap();
    assert!(press(&mut tree, ArrowRight, 11).key_handled);
    assert!(press(&mut tree, ArrowLeft, 12).key_handled);
    tree.clear_focus().unwrap();
    assert!(press(&mut tree, ArrowLeft, 13).key_handled);
    assert_eq!(
        action_names(&trace),
        [
            "test.right",
            "test.menu",
            "test.menu",
            "test.right",
            "test.outer",
            "test.outer"
        ]
    );
    assert_eq!(
        trace
            .borrow()
            .iter()
            .map(|event| event.target)
            .collect::<Vec<_>>(),
        [settings, root, root, board, root, root]
    );
}

#[test]
fn override_validation_is_atomic_and_checks_complete_prefixes_in_both_orders() {
    use KeyCode::{KeyK, KeyP, KeyQ, KeyT, KeyX};

    let short = binding("test.short", [KeyK]);
    let long = binding("test.long", [KeyK, KeyT]);
    for (first, second) in [(&short, &long), (&long, &short)] {
        assert_eq!(
            Keymap::new([first.clone(), second.clone()]),
            Err(KeymapError::Conflict {
                first: Box::new(first.clone()),
                second: Box::new(second.clone()),
            }),
        );
        let original = Keymap::new([first.clone()]).unwrap();
        let mut edited = original.clone();
        assert!(matches!(
            edited.set_overrides([second.clone()]),
            Err(KeymapError::Conflict { .. })
        ));
        assert_eq!(edited, original);
    }
    let shared = binding("test.other", [KeyK, KeyP]);
    assert!(Keymap::new([long, shared.clone()]).is_ok());
    assert!(Keymap::new([shared.repeatable()]).is_err());
    assert!(Keymap::new([KeyBinding::new("test.empty", [])]).is_err());
    assert!(Keymap::new([binding("test.traversal", [KeyK, KeyCode::Tab])]).is_err());
    assert!(
        Keymap::new([KeyBinding::new(
            "test.modified-tab",
            [logical(KeyCode::Tab, CONTROL)],
        )])
        .is_ok()
    );
    assert!(
        Keymap::new([KeyBinding::new(
            "test.unknown",
            [KeyChord::logical(LogicalKey::Unidentified, PLAIN)]
        )])
        .is_err()
    );

    let mut map = Keymap::new([short.clone(), binding("test.other", [KeyQ])]).unwrap();
    // Both old bindings are removed before validating the swapped chords.
    let swap = [
        binding("test.short", [KeyQ]),
        binding("test.other", [KeyK, KeyT]),
    ];
    map.set_overrides(swap.clone()).unwrap();
    assert!(map.effective_bindings().eq(&swap));
    assert!(
        map.set_overrides([binding("test.other", [KeyK, KeyX])])
            .is_err()
    );
    assert!(map.effective_bindings().eq(&swap));
    map.set_overrides([]).unwrap();
    assert_eq!(map.effective_bindings().next(), Some(&short));
}

#[test]
fn an_override_wins_when_a_layout_matches_both_logical_and_physical_chords() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let trace = Rc::new(RefCell::new(Vec::new()));
    record(&mut tree, root, "test.position", &trace);
    record(&mut tree, root, "test.character", &trace);
    let mut map = Keymap::new([
        KeyBinding::new("test.position", [KeyChord::physical(KeyCode::KeyA, PLAIN)]),
        binding("test.character", [KeyCode::KeyD]),
    ])
    .unwrap();
    map.set_overrides([binding("test.character", [KeyCode::KeyQ])])
        .unwrap();
    tree.set_keymap(root, map).unwrap();
    for logical in [LogicalKey::Character("q".into()), LogicalKey::Unidentified] {
        assert!(
            dispatch(
                &mut tree,
                WindowEvent::Key {
                    code: KeyCode::KeyA,
                    logical,
                    modifiers: PLAIN,
                    pressed: true,
                    repeat: false,
                    timestamp: Duration::ZERO,
                }
            )
            .key_handled
        );
    }
    assert!(!press(&mut tree, KeyCode::KeyD, 0).key_handled);
    assert_eq!(action_names(&trace), ["test.character", "test.position"]);
}

#[test]
fn partial_sequences_are_idle_and_timeout_or_mismatch_retries_the_stroke() {
    use KeyCode::{KeyK, KeyP, KeyQ, KeyT};

    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let inner = tree
        .mount(root, node().focusable(FocusPolicy::Tab))
        .unwrap();
    let trace = Rc::new(RefCell::new(Vec::new()));
    for action in ["test.p", "test.t"] {
        record(&mut tree, inner, action, &trace);
    }
    for action in ["test.outer", "test.latest", "test.repeat"] {
        record(&mut tree, root, action, &trace);
    }
    map(
        &mut tree,
        inner,
        [
            binding("test.p", [KeyK, KeyP]),
            binding("test.t", [KeyK, KeyT]),
        ],
    );
    map(
        &mut tree,
        root,
        [
            binding("test.outer", [KeyK]),
            binding("test.latest", [KeyQ]).repeatable(),
            binding("test.repeat", [KeyP]).repeatable(),
        ],
    );
    tree.focus(inner).unwrap();
    tree.paint(VIEWPORT, 1.0).unwrap();
    let before = tree.diagnostics();
    let prefix = press(&mut tree, KeyK, 10);
    assert!(prefix.key_handled && !prefix.changed);
    assert_eq!(tree.diagnostics(), before);
    assert!(dispatch(&mut tree, transition(KeyK, true, true, 200)).key_handled);
    assert!(trace.borrow().is_empty());
    assert!(press(&mut tree, KeyT, 201).key_handled);
    assert!(press(&mut tree, KeyK, 300).key_handled);
    assert!(press(&mut tree, KeyQ, 310).key_handled);
    assert!(dispatch(&mut tree, transition(KeyQ, true, true, 311)).key_handled);
    assert!(press(&mut tree, KeyK, 320).key_handled);
    assert!(press(&mut tree, KeyP, 321).key_handled);
    assert!(dispatch(&mut tree, transition(KeyP, true, true, 322)).key_handled);
    assert!(dispatch(&mut tree, transition(KeyP, false, false, 323)).key_handled);
    assert!(press(&mut tree, KeyK, 400).key_handled);
    let expired_at = 400 + u64::try_from(KEY_SEQUENCE_TIMEOUT.as_millis()).unwrap();
    assert!(!press(&mut tree, KeyT, expired_at).key_handled);
    assert_eq!(
        action_names(&trace),
        ["test.t", "test.latest", "test.latest", "test.p"]
    );
    assert_eq!(
        trace.borrow()[0].timestamp,
        Some(Duration::from_millis(201))
    );
}

#[test]
fn focus_route_keymap_and_window_changes_clear_partial_sequences() {
    use KeyCode::{KeyK, KeyT};

    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let other = tree
        .mount(root, node().focusable(FocusPolicy::Tab))
        .unwrap();
    let container = tree.mount(root, node()).unwrap();
    let focused = tree
        .mount(container, node().focusable(FocusPolicy::Tab))
        .unwrap();
    let trace = Rc::new(RefCell::new(Vec::new()));
    record(&mut tree, focused, "test.sequence", &trace);
    map(&mut tree, focused, [binding("test.sequence", [KeyK, KeyT])]);
    tree.focus(focused).unwrap();
    assert!(press(&mut tree, KeyK, 1).key_handled);
    tree.focus(other).unwrap();
    tree.focus(focused).unwrap();
    assert!(!press(&mut tree, KeyT, 2).key_handled);
    assert!(press(&mut tree, KeyK, 3).key_handled);
    tree.reparent(focused, root, 0).unwrap();
    assert_eq!(tree.focused_node(), Some(focused));
    assert!(!press(&mut tree, KeyT, 4).key_handled);
    assert!(press(&mut tree, KeyK, 5).key_handled);
    map(
        &mut tree,
        focused,
        [binding("test.different", [KeyK, KeyT])],
    );
    assert!(!press(&mut tree, KeyT, 6).key_handled);
    assert!(press(&mut tree, KeyK, 7).key_handled);
    for active in [false, true] {
        dispatch(
            &mut tree,
            WindowEvent::ActivationChanged {
                active,
                timestamp: Duration::from_millis(8),
            },
        );
    }
    assert_eq!(tree.focused_node(), Some(focused));
    assert!(!press(&mut tree, KeyT, 9).key_handled);
    assert!(trace.borrow().is_empty());
}

#[test]
fn consuming_a_recorded_stroke_prevents_repeats_and_releases_from_activating_it() {
    use KeyCode::{KeyK, KeyT};

    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let trace = Rc::new(RefCell::new(Vec::new()));
    record(&mut tree, root, "test.sequence", &trace);
    record(&mut tree, root, "test.rebound", &trace);
    let mut keys = Keymap::new([
        binding("test.sequence", [KeyK, KeyT]),
        binding("test.rebound", [KeyCode::KeyR]).repeatable(),
    ])
    .unwrap();
    tree.set_keymap(root, keys.clone()).unwrap();
    assert!(press(&mut tree, KeyK, 0).key_handled);
    let consumed = tree
        .dispatch_consumed_key(&transition(KeyT, true, false, 1), VIEWPORT, |_, _, _| {})
        .unwrap();
    assert!(consumed.key_handled && trace.borrow().is_empty());
    keys.set_overrides([binding("test.rebound", [KeyT]).repeatable()])
        .unwrap();
    tree.set_keymap(root, keys).unwrap();
    for active in [false, true] {
        dispatch(
            &mut tree,
            WindowEvent::ActivationChanged {
                active,
                timestamp: Duration::from_millis(1),
            },
        );
    }
    assert!(dispatch(&mut tree, transition(KeyT, true, true, 2)).key_handled);
    assert!(dispatch(&mut tree, transition(KeyT, false, false, 3)).key_handled);
    assert!(trace.borrow().is_empty());
    assert!(press(&mut tree, KeyT, 4).key_handled);
    assert_eq!(action_names(&trace), ["test.rebound"]);
    assert!(!press(&mut tree, KeyCode::KeyR, 5).key_handled);
}

#[test]
fn actions_are_enumerable_and_programmatic_dispatch_respects_node_lifetime() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let trace = Rc::new(RefCell::new(Vec::new()));
    let after = Rc::clone(&trace);
    let id = tree
        .mount(
            root,
            node()
                .semantics(NodeSemantics {
                    role: "button".into(),
                    actions: vec!["old.action".into()],
                    ..NodeSemantics::default()
                })
                .on_action(
                    Action::new("test.first", "First operation"),
                    |event, view, commands| {
                        assert!(view.contains(event.target));
                        commands.invoke_action(event.target, "test.second");
                    },
                )
                .on_action(
                    Action::new("test.second", "Second operation"),
                    move |event, _, _| {
                        after.borrow_mut().push(event.clone());
                    },
                ),
        )
        .unwrap();
    assert_eq!(
        tree.actions(id).unwrap(),
        [
            Action::new("test.first", "First operation"),
            Action::new("test.second", "Second operation"),
        ]
    );
    let semantics = tree.semantics(id).unwrap().unwrap();
    assert_eq!(semantics.role, "button");
    assert_eq!(semantics.actions, ["test.first", "test.second"]);
    assert!(tree.invoke_action(id, "test.first").unwrap());
    assert_eq!(action_names(&trace), ["test.second"]);
    assert_eq!(trace.borrow()[0].source, ActionSource::Programmatic);
    assert_eq!(trace.borrow()[0].timestamp, None);
    assert!(!tree.invoke_action(id, "old.action").unwrap());
    tree.set_disabled(id, true).unwrap();
    assert!(!tree.invoke_action(id, "test.first").unwrap());
    tree.set_disabled(id, false).unwrap();
    tree.set_visible(id, false).unwrap();
    assert!(!tree.invoke_action(id, "test.first").unwrap());
    tree.remove(id).unwrap();
    assert_eq!(
        tree.invoke_action(id, "test.first"),
        Err(UiError::StaleNode)
    );
    assert_eq!(trace.borrow().len(), 1);
}
