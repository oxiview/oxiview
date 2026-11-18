//! Public-API input traces spanning layout, stacking, commands, and lifecycle cleanup.

use oxiview_base::{Point, Size};
use oxiview_core::{NodeId, PointerEventKind, UiError, UiTree};
use oxiview_platform::{PointerButton, WindowEvent};
use oxiview_style::{Display, Edges, LayoutStyle, Length, Position, Style};
use std::time::Duration;

fn viewport() -> Size {
    Size::new(200.0, 120.0)
}

fn positioned(x: f64, y: f64, width: f64, height: f64, z: i32) -> Style {
    Style {
        layout: LayoutStyle {
            position: Position::Absolute,
            width: Length::Points(width),
            height: Length::Points(height),
            inset: Edges {
                left: Length::Points(x),
                top: Length::Points(y),
                ..Edges::all(Length::Auto)
            },
            ..LayoutStyle::default()
        },
        z_index: Some(z),
        ..Style::default()
    }
}

fn tree() -> Result<UiTree, UiError> {
    let mut tree = UiTree::new()?;
    tree.set_style(
        tree.root(),
        Style {
            layout: LayoutStyle {
                display: Display::Block,
                position: Position::Relative,
                width: Length::Points(viewport().width()),
                height: Length::Points(viewport().height()),
                ..LayoutStyle::default()
            },
            ..Style::default()
        },
    )?;
    Ok(tree)
}

fn moved(x: f64, y: f64, time: u64) -> WindowEvent {
    WindowEvent::PointerMoved {
        position: Point::new(x, y),
        timestamp: Duration::from_millis(time),
    }
}

fn button(x: f64, y: f64, button: PointerButton, pressed: bool, time: u64) -> WindowEvent {
    WindowEvent::PointerButton {
        button,
        pressed,
        position: Point::new(x, y),
        timestamp: Duration::from_millis(time),
    }
}

fn layered_subtree(tree: &mut UiTree) -> Result<(NodeId, NodeId, NodeId), UiError> {
    let underlying = tree.append(tree.root(), positioned(0.0, 0.0, 180.0, 100.0, 1))?;
    let parent = tree.append(tree.root(), positioned(10.0, 10.0, 80.0, 80.0, 2))?;
    let child = tree.append(
        parent,
        Style {
            layout: LayoutStyle {
                width: Length::Points(80.0),
                height: Length::Points(80.0),
                ..LayoutStyle::default()
            },
            ..Style::default()
        },
    )?;
    Ok((underlying, parent, child))
}

#[test]
fn enter_deletion_keeps_the_down_target_and_bubble_route_alive() -> Result<(), UiError> {
    let mut tree = tree()?;
    let root = tree.root();
    let underlying = tree.append(root, positioned(0.0, 0.0, 180.0, 100.0, 1))?;
    let target = tree.append(root, positioned(10.0, 10.0, 80.0, 80.0, 2))?;
    let mut trace = Vec::new();
    let result = tree.dispatch(
        &button(25.0, 25.0, PointerButton::Primary, true, 10),
        viewport(),
        |event, view, commands| {
            if event.current_target == target && event.kind == PointerEventKind::Enter {
                commands.remove(target);
                assert!(view.contains(target));
                assert_eq!(view.hover_path().last(), Some(&target));
                trace.push((target, PointerEventKind::Enter));
            }
            if event.kind == PointerEventKind::Down {
                assert_eq!(event.target, target);
                assert_ne!(event.current_target, underlying);
                assert_eq!(event.timestamp, Duration::from_millis(10));
                assert!(view.contains(target));
                trace.push((event.current_target, PointerEventKind::Down));
            }
            if event.current_target == target && event.kind == PointerEventKind::Leave {
                assert!(view.contains(target));
                assert!(!view.hover_path().contains(&target));
                trace.push((target, PointerEventKind::Leave));
            }
        },
    )?;
    assert_eq!(
        trace,
        vec![
            (target, PointerEventKind::Enter),
            (target, PointerEventKind::Down),
            (root, PointerEventKind::Down),
            (target, PointerEventKind::Leave),
        ]
    );
    assert!(!tree.contains(target));
    assert!(tree.contains(underlying));
    assert!(
        result
            .reports
            .iter()
            .flat_map(|report| &report.outcomes)
            .all(|outcome| outcome.result.is_ok())
    );
    Ok(())
}

#[test]
fn only_the_final_button_up_releases_before_self_deletion() -> Result<(), UiError> {
    let mut tree = tree()?;
    let underlying = tree.append(tree.root(), positioned(0.0, 0.0, 180.0, 100.0, 1))?;
    let holder = tree.append(tree.root(), positioned(10.0, 10.0, 70.0, 70.0, 2))?;
    tree.dispatch(
        &button(25.0, 25.0, PointerButton::Primary, true, 1),
        viewport(),
        |event, _, commands| {
            if event.current_target == holder && event.kind == PointerEventKind::Down {
                commands.capture(holder);
            }
        },
    )?;
    tree.dispatch(
        &button(150.0, 30.0, PointerButton::Secondary, true, 2),
        viewport(),
        |_, _, _| {},
    )?;
    tree.dispatch(
        &button(150.0, 30.0, PointerButton::Primary, false, 3),
        viewport(),
        |_, _, _| {},
    )?;
    assert_eq!(tree.capture_holder(), Some(holder));
    assert_eq!(tree.hover_path().last(), Some(&holder));

    let mut trace = Vec::new();
    tree.dispatch(
        &button(150.0, 30.0, PointerButton::Secondary, false, 4),
        viewport(),
        |event, view, commands| {
            assert_ne!(event.kind, PointerEventKind::Cancel);
            if event.current_target == holder && event.kind == PointerEventKind::Up {
                assert_eq!(view.capture_holder(), Some(holder));
                commands.remove(holder);
                trace.push((holder, PointerEventKind::Up));
            }
            if event.current_target == holder && event.kind == PointerEventKind::Leave {
                assert_eq!(view.capture_holder(), None);
                assert!(view.contains(holder));
                trace.push((holder, PointerEventKind::Leave));
            }
            if event.current_target == underlying && event.kind == PointerEventKind::Enter {
                assert_eq!(view.capture_holder(), None);
                assert!(view.contains(holder));
                trace.push((underlying, PointerEventKind::Enter));
            }
        },
    )?;
    assert_eq!(
        trace,
        vec![
            (holder, PointerEventKind::Up),
            (holder, PointerEventKind::Leave),
            (underlying, PointerEventKind::Enter)
        ]
    );
    assert!(!tree.contains(holder));
    assert_eq!(tree.capture_holder(), None);
    assert_eq!(tree.hover_path().last(), Some(&underlying));
    Ok(())
}

#[test]
fn deleting_hovered_ancestry_leaves_inner_first_without_entering_exposed_node()
-> Result<(), UiError> {
    let mut tree = tree()?;
    let (underlying, parent, child) = layered_subtree(&mut tree)?;
    tree.dispatch(&moved(25.0, 25.0, 1), viewport(), |_, _, _| {})?;
    assert_eq!(tree.hover_path().last(), Some(&child));
    let mut departed = Vec::new();
    tree.dispatch(
        &moved(25.0, 25.0, 2),
        viewport(),
        |event, view, commands| {
            if event.current_target == child && event.kind == PointerEventKind::Move {
                commands.remove(parent);
            }
            if event.kind == PointerEventKind::Leave {
                assert!(view.contains(child));
                assert!(view.contains(parent));
                assert!(!view.hover_path().contains(&child));
                assert!(!view.hover_path().contains(&parent));
                departed.push(event.current_target);
            }
            assert!(!(event.current_target == underlying && event.kind == PointerEventKind::Enter));
        },
    )?;
    assert_eq!(departed, vec![child, parent]);
    assert_eq!(tree.hover_path(), &[tree.root()]);
    assert!(!tree.contains(child));
    assert!(!tree.contains(parent));
    assert!(tree.contains(underlying));
    Ok(())
}

#[test]
fn hiding_captured_ancestry_publishes_cleanup_before_suppression() -> Result<(), UiError> {
    let mut tree = tree()?;
    let (_, parent, child) = layered_subtree(&mut tree)?;
    let mut hidden = tree.style(parent)?.clone();
    hidden.layout.display = Display::None;
    tree.dispatch(
        &button(25.0, 25.0, PointerButton::Primary, true, 1),
        viewport(),
        |event, _, commands| {
            if event.current_target == child && event.kind == PointerEventKind::Down {
                commands.capture(child);
            }
        },
    )?;
    let mut trace = Vec::new();
    tree.dispatch(
        &moved(25.0, 25.0, 2),
        viewport(),
        |event, view, commands| {
            if event.current_target == child && event.kind == PointerEventKind::Move {
                commands.set_style(parent, hidden.clone());
            }
            if event.current_target == child && event.kind == PointerEventKind::Cancel {
                assert_eq!(view.capture_holder(), Some(child));
                assert_eq!(view.hover_path().last(), Some(&child));
                assert!(
                    matches!(view.style(parent), Ok(style) if style.layout.display != Display::None)
                );
                trace.push((child, PointerEventKind::Cancel));
            }
            if event.kind == PointerEventKind::Leave {
                assert_eq!(view.capture_holder(), None);
                assert_eq!(view.hover_path(), &[view.root()]);
                assert!(
                    matches!(view.style(parent), Ok(style) if style.layout.display != Display::None)
                );
                trace.push((event.current_target, PointerEventKind::Leave));
            }
        },
    )?;
    assert_eq!(
        trace,
        vec![
            (child, PointerEventKind::Cancel),
            (child, PointerEventKind::Leave),
            (parent, PointerEventKind::Leave)
        ]
    );
    assert!(tree.contains(parent));
    assert!(tree.contains(child));
    assert_eq!(tree.style(parent)?.layout.display, Display::None);
    assert_eq!(tree.capture_holder(), None);
    Ok(())
}
