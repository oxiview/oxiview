//! M2 S6 traces at the public platform-event and retained UI boundary.

use std::time::Duration;

use oxiview_base::{Color, Point, Size};
use oxiview_core::{NodeId, PointerEventKind, UiError, UiTree};
use oxiview_platform::{PointerButton, WindowEvent};
use oxiview_style::{Edges, LayoutStyle, Length, Position, Style};

const VIEWPORT: Size = Size::new(200.0, 120.0);
const HOLDER_ORIGIN: Point = Point::new(20.0, 15.0);
const PRESS: Point = Point::new(30.0, 25.0);
const SIBLING: Point = Point::new(130.0, 80.0);
const EDGES: [(&str, Point, Point); 4] = [
    ("left", Point::new(-0.25, 34.5), Point::new(-9.5, 37.0)),
    ("right", Point::new(200.0, 34.5), Point::new(209.5, 37.0)),
    ("top", Point::new(36.5, -0.25), Point::new(39.0, -9.5)),
    ("bottom", Point::new(36.5, 120.0), Point::new(39.0, 129.5)),
];

fn positioned(origin: Point) -> Style {
    Style {
        layout: LayoutStyle {
            position: Position::Absolute,
            width: Length::Points(40.0),
            height: Length::Points(35.0),
            inset: Edges {
                left: Length::Points(origin.x()),
                top: Length::Points(origin.y()),
                ..Edges::all(Length::Auto)
            },
            ..LayoutStyle::default()
        },
        ..Style::default()
    }
}

fn fixture() -> Result<(UiTree, NodeId, NodeId), UiError> {
    let mut tree = UiTree::new()?;
    let holder = tree.append(tree.root(), positioned(HOLDER_ORIGIN))?;
    let sibling = tree.append(tree.root(), positioned(Point::new(105.0, 64.0)))?;
    tree.paint(VIEWPORT, 1.25)?;
    Ok((tree, holder, sibling))
}

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn moved(position: Point, time: u64) -> WindowEvent {
    WindowEvent::PointerMoved {
        position,
        timestamp: ms(time),
    }
}

fn primary(position: Point, pressed: bool, time: u64) -> WindowEvent {
    WindowEvent::PointerButton {
        button: PointerButton::Primary,
        pressed,
        position,
        timestamp: ms(time),
    }
}

fn start_drag(tree: &mut UiTree, holder: NodeId) -> Result<(), UiError> {
    let result = tree.dispatch(&primary(PRESS, true, 1), VIEWPORT, |event, _, commands| {
        if event.kind == PointerEventKind::Down && event.current_target == holder {
            commands.capture(holder);
        }
    })?;
    assert_eq!(tree.capture_holder(), Some(holder));
    assert_eq!(tree.hover_path(), &[tree.root(), holder]);
    assert!(!result.changed);
    Ok(())
}

fn cross_edge(outside: Point, release: Point, returns: bool) -> Result<(), UiError> {
    use PointerEventKind::{Enter, Leave, Move, Up};

    let (mut tree, holder, sibling) = fixture()?;
    let root = tree.root();
    start_drag(&mut tree, holder)?;

    let left = tree.dispatch(
        &WindowEvent::PointerLeft { timestamp: ms(2) },
        VIEWPORT,
        |_, _, _| {},
    )?;
    assert_eq!(left.events, 0);
    assert!(!left.changed);
    assert_eq!(tree.capture_holder(), Some(holder));
    assert_eq!(tree.hover_path(), &[root, holder]);

    for (position, timestamp, hit) in [(outside, 3, None), (SIBLING, 4, Some(sibling))]
        .into_iter()
        .take(if returns { 2 } else { 1 })
    {
        let mut trace = Vec::new();
        let motion = tree.dispatch(&moved(position, timestamp), VIEWPORT, |event, view, _| {
            trace.push((event.kind, event.current_target));
            assert_eq!(event.target, holder);
            assert_eq!(event.position, position);
            assert_eq!(event.timestamp, ms(timestamp));
            assert_eq!(event.buttons, 1);
            assert_eq!(event.button, None);
            let local = if event.current_target == holder {
                Point::new(position.x() - 20.0, position.y() - 15.0)
            } else {
                position
            };
            assert_eq!(event.local_position, local);
            assert_eq!(view.hit_test(position), Ok(hit));
            assert_eq!(view.capture_holder(), Some(holder));
            assert_eq!(view.hover_path(), &[root, holder]);
        })?;
        assert_eq!(trace, [(Move, holder), (Move, root)]);
        assert!(!motion.changed);
    }

    let release = if returns {
        Point::new(131.5, 81.25)
    } else {
        release
    };
    let mut trace = Vec::new();
    let up = tree.dispatch(&primary(release, false, 5), VIEWPORT, |event, view, _| {
        trace.push((event.kind, event.current_target));
        assert_eq!(event.position, release);
        assert_eq!(event.timestamp, ms(5));
        assert_eq!(event.buttons, 0);
        if event.kind == Up {
            assert_eq!(event.target, holder);
            assert_eq!(event.button, Some(PointerButton::Primary));
            assert_eq!(view.capture_holder(), Some(holder));
            assert_eq!(view.hover_path(), &[root, holder]);
        } else {
            assert_eq!(event.target, event.current_target);
            assert_eq!(view.capture_holder(), None);
            assert_eq!(view.hover_target(), returns.then_some(sibling));
        }
    })?;
    let last = if returns {
        (Enter, sibling)
    } else {
        (Leave, root)
    };
    assert_eq!(trace, [(Up, holder), (Up, root), (Leave, holder), last]);
    assert_eq!(tree.capture_holder(), None);
    if returns {
        assert_eq!(tree.hover_path(), &[root, sibling]);
    } else {
        assert!(tree.hover_path().is_empty());
    }
    assert!(!up.changed);
    Ok(())
}

#[test]
fn captured_motion_and_release_outside_each_edge() {
    for (edge, outside, release) in EDGES {
        eprintln!("Outside release across the {edge} edge");
        cross_edge(outside, release, false).unwrap();
    }
}

#[test]
fn captured_motion_returns_across_each_edge_before_release() {
    for (edge, outside, release) in EDGES {
        eprintln!("Return across the {edge} edge");
        cross_edge(outside, release, true).unwrap();
    }
}

#[test]
fn lifecycle_cleans_up_once_even_when_cancel_bubbling_stops() -> Result<(), UiError> {
    use PointerEventKind::{Cancel, Enter, Leave, Move};

    let deactivate = WindowEvent::ActivationChanged {
        active: false,
        timestamp: ms(4),
    };
    let minimize = WindowEvent::VisibilityChanged {
        visible: false,
        timestamp: ms(4),
    };
    let interrupt = WindowEvent::PointerInterrupted { timestamp: ms(4) };
    for first in [&deactivate, &minimize, &interrupt, &WindowEvent::Destroyed] {
        let (mut tree, holder, sibling) = fixture()?;
        let root = tree.root();
        start_drag(&mut tree, holder)?;
        tree.dispatch(
            &WindowEvent::PointerLeft { timestamp: ms(2) },
            VIEWPORT,
            |_, _, _| {},
        )?;
        let outside = EDGES[0].1;
        tree.dispatch(&moved(outside, 3), VIEWPORT, |_, _, _| {})?;

        let destroyed = matches!(first, WindowEvent::Destroyed);
        let mut trace = Vec::new();
        tree.dispatch(first, VIEWPORT, |event, view, commands| {
            trace.push((event.kind, event.current_target));
            assert_eq!(event.position, outside);
            assert!(view.contains(root));
            assert!(view.contains(holder));
            assert_eq!(event.timestamp, ms(if destroyed { 3 } else { 4 }));
            match event.kind {
                Cancel => {
                    assert_eq!(view.capture_holder(), Some(holder));
                    assert_eq!(view.hover_path(), &[root, holder]);
                    commands.stop_propagation();
                }
                Leave => {
                    assert_eq!(view.capture_holder(), None);
                    assert!(view.hover_path().is_empty());
                    assert_eq!(event.buttons, 0);
                }
                kind => panic!("unexpected {kind:?} during {first:?}"),
            }
        })?;
        assert_eq!(
            trace,
            [(Cancel, holder), (Leave, holder), (Leave, root)],
            "{first:?}"
        );
        assert!(tree.hover_path().is_empty());
        assert_eq!(tree.capture_holder(), None);
        if !destroyed {
            for followup in [
                first.clone(),
                deactivate.clone(),
                minimize.clone(),
                interrupt.clone(),
                WindowEvent::VisibilityChanged {
                    visible: true,
                    timestamp: ms(5),
                },
                WindowEvent::ActivationChanged {
                    active: true,
                    timestamp: ms(6),
                },
            ] {
                let result = tree.dispatch(&followup, VIEWPORT, |event, _, _| {
                    panic!("{followup:?} unexpectedly delivered {event:?}");
                })?;
                assert!(!result.changed);
                assert!(tree.hover_path().is_empty());
                assert_eq!(tree.capture_holder(), None);
            }
            let mut recovered = Vec::new();
            tree.dispatch(&moved(SIBLING, 7), VIEWPORT, |event, _, _| {
                assert_eq!(event.buttons, 0);
                recovered.push((event.kind, event.current_target));
            })?;
            assert_eq!(
                recovered,
                [
                    (Enter, root),
                    (Enter, sibling),
                    (Move, sibling),
                    (Move, root)
                ]
            );
            assert_eq!(tree.hover_path(), &[root, sibling]);
            tree.dispatch(&WindowEvent::Destroyed, VIEWPORT, |event, _, _| {
                assert_eq!(event.kind, Leave);
            })?;
        }
        assert!(!tree.contains(holder));
        assert!(!tree.contains(sibling));
        assert!(!tree.contains(root));
        for late in [WindowEvent::Destroyed, moved(SIBLING, 8)] {
            assert!(matches!(
                tree.dispatch(&late, VIEWPORT, |event, _, _| {
                    panic!("closed tree delivered {event:?}");
                }),
                Err(UiError::Closed)
            ));
        }
    }
    Ok(())
}

#[test]
fn stale_capture_and_no_op_style_do_not_request_redraw() -> Result<(), UiError> {
    use PointerEventKind::{Leave, Move};

    let (mut tree, holder, _) = fixture()?;
    let root = tree.root();
    start_drag(&mut tree, holder)?;
    let outside = EDGES[0].1;
    let mut root_left = false;
    tree.dispatch(&moved(outside, 2), VIEWPORT, |event, view, commands| {
        if event.kind == Move && event.current_target == holder {
            commands.remove(holder);
        }
        if event.kind == Leave && event.current_target == root {
            assert!(view.contains(holder));
            assert!(view.hover_path().is_empty());
            root_left = true;
        }
    })?;
    assert!(root_left);
    assert!(!tree.contains(holder));
    assert_eq!(tree.capture_holder(), None);
    let replacement = tree.append(root, positioned(HOLDER_ORIGIN))?;
    assert_ne!(holder, replacement);
    tree.paint(VIEWPORT, 1.25)?;

    let mut moves = Vec::new();
    let no_op = tree.dispatch(&moved(PRESS, 3), VIEWPORT, |event, view, commands| {
        assert_ne!(event.current_target, holder);
        if event.kind == Move {
            moves.push(event.current_target);
            assert_eq!(event.target, replacement);
            assert_eq!(event.buttons, 1);
            if event.current_target == replacement {
                commands.capture(holder);
                commands.set_style(replacement, view.style(replacement).unwrap().clone());
            }
        }
    })?;
    assert_eq!(moves, [replacement, root]);
    let [report] = no_op.reports.as_slice() else {
        panic!("expected one command batch")
    };
    let [capture, style] = report.outcomes.as_slice() else {
        panic!("expected two command outcomes")
    };
    assert_eq!(capture.node, Some(holder));
    assert!(matches!(capture.result, Err(UiError::StaleNode)));
    assert_eq!(style.node, Some(replacement));
    assert!(style.result.is_ok());
    assert_eq!(tree.capture_holder(), None);
    assert_eq!(tree.hover_path(), &[root, replacement]);
    assert!(!no_op.changed);

    let mut painted = tree.style(replacement)?.clone();
    painted.background = Color::from_rgba8(40, 90, 230, 255);
    let visible_change = tree.dispatch(&moved(PRESS, 4), VIEWPORT, |event, _, commands| {
        if event.kind == Move && event.current_target == replacement {
            commands.set_style(replacement, painted.clone());
        }
    })?;
    assert!(visible_change.changed);
    Ok(())
}
