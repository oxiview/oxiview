//! Mounted and imperative node ownership survives checked subtree reparenting.
// Test-only assertions unwrap deterministic fixtures and callbacks.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use oxiview_base::{Point, Size};
use oxiview_core::{PointerEventKind, UiError, UiTree, component, node};
use oxiview_platform::{PointerButton, WindowEvent};
use oxiview_reactive::{ReactiveError, Scope, Signal, effect, signal};
use oxiview_style::{Display, Length, Position, Style};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

fn viewport() -> Size {
    Size::new(400.0, 200.0)
}
fn box_style(left: f64, width: f64) -> Style {
    let mut style = Style::default();
    style.layout.position = Position::Absolute;
    style.layout.inset.left = Length::Points(left);
    style.layout.width = Length::Points(width);
    style.layout.height = Length::Points(80.0);
    style
}
fn ui(error: ReactiveError) -> UiError {
    UiError::Reactive(error.to_string())
}
struct Resource(Rc<Cell<usize>>);
impl Resource {
    fn touch(&self) {
        let _ = self.0.get();
    }
}
impl Drop for Resource {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn moved_component_keeps_private_state_handlers_and_effects_without_adopting_external_signals() {
    let private = Rc::new(Cell::new(None::<Signal<i32>>));
    let external = Rc::new(Cell::new(None::<Signal<i32>>));
    let child = Rc::new(Cell::new(None));
    let builds = Rc::new(Cell::new(0));
    let effects = Rc::new(Cell::new(0));
    let releases = Rc::new(Cell::new(0));
    let external_disposed_in_handler = Rc::new(Cell::new(false));
    let saved_private = Rc::clone(&private);
    let saved_external = Rc::clone(&external);
    let saved_child = Rc::clone(&child);
    let built = Rc::clone(&builds);
    let ran = Rc::clone(&effects);
    let dropped = Rc::clone(&releases);
    let observed_external = Rc::clone(&external_disposed_in_handler);
    let a_view = component(move || {
        let external = signal(99).map_err(ui)?;
        saved_external.set(Some(external));
        Ok(node()
            .style(box_style(0.0, 100.0))
            .child(component(move || {
                built.set(built.get() + 1);
                let local = signal(7).map_err(ui)?;
                saved_private.set(Some(local));
                let resource = Resource(dropped);
                effect(move || {
                    local.get()?;
                    resource.touch();
                    ran.set(ran.get() + 1);
                    Ok(())
                })
                .map_err(ui)?;
                Ok(node()
                    .bind_style(move || Ok(box_style(0.0, 40.0 + f64::from(local.get()?))))
                    .on_mount(move |id| saved_child.set(Some(id)))
                    .on_pointer(move |event, _, _| {
                        if event.kind == PointerEventKind::Down {
                            local.update(|value| *value += 1).unwrap();
                            observed_external.set(external.get() == Err(ReactiveError::Disposed));
                        }
                    }))
            })))
    });
    let mut tree = UiTree::new().unwrap();
    let a = tree.mount(tree.root(), a_view).unwrap();
    let b = tree
        .mount(tree.root(), node().style(box_style(180.0, 100.0)))
        .unwrap();
    let p = child.get().unwrap();
    let local = private.get().unwrap();
    assert_eq!(effects.get(), 1);
    tree.reparent(p, b, 0).unwrap();
    tree.remove(a).unwrap();
    assert!(tree.contains(p));
    assert_eq!(tree.parent(p).unwrap(), Some(b));
    assert_eq!(local.get().unwrap(), 7);
    assert_eq!(external.get().unwrap().get(), Err(ReactiveError::Disposed));
    assert_eq!(releases.get(), 0);
    local.set(9).unwrap();
    tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    assert_eq!(effects.get(), 2);
    assert_eq!(tree.style(p).unwrap().layout.width, Length::Points(49.0));
    tree.dispatch(
        &WindowEvent::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
            position: Point::new(190.0, 10.0),
            timestamp: Duration::ZERO,
        },
        viewport(),
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(local.get().unwrap(), 10);
    assert!(external_disposed_in_handler.get());
    assert_eq!(builds.get(), 1);
    assert_eq!(effects.get(), 3);
    tree.remove(b).unwrap();
    assert_eq!(local.get(), Err(ReactiveError::Disposed));
    assert_eq!(releases.get(), 1);
    drop(tree);
    assert_eq!(releases.get(), 1);
}

#[test]
fn scope_adoption_failure_is_checked_before_links_or_capture_cleanup_change() {
    let active = Rc::new(RefCell::new(None::<Scope>));
    let saved = Rc::clone(&active);
    let mut tree = UiTree::new().unwrap();
    let a = tree.append(tree.root(), box_style(0.0, 100.0)).unwrap();
    let mut hidden = box_style(180.0, 100.0);
    hidden.layout.display = Display::None;
    let b = tree.append(tree.root(), hidden).unwrap();
    let p = tree
        .mount(
            a,
            component(move || {
                *saved.borrow_mut() = Some(Scope::new().map_err(ui)?);
                Ok(node()
                    .style(box_style(0.0, 60.0))
                    .on_pointer(|event, _, commands| {
                        if event.kind == PointerEventKind::Down {
                            commands.capture(event.current_target);
                        }
                    }))
            }),
        )
        .unwrap();
    tree.dispatch(
        &WindowEvent::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
            position: Point::new(10.0, 10.0),
            timestamp: Duration::ZERO,
        },
        viewport(),
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(tree.capture_holder(), Some(p));
    let owner = active.borrow().as_ref().unwrap().owner();
    let mut canceled = false;
    let result = owner
        .run(|| {
            Ok(tree.dispatch(
                &WindowEvent::PointerMoved {
                    position: Point::new(11.0, 10.0),
                    timestamp: Duration::ZERO,
                },
                viewport(),
                |event, _, commands| {
                    if event.current_target == p && event.kind == PointerEventKind::Move {
                        commands.reparent(p, b, 0);
                    }
                    if event.kind == PointerEventKind::Cancel {
                        canceled = true;
                    }
                },
            ))
        })
        .unwrap()
        .unwrap();
    assert!(
        result
            .reports
            .iter()
            .flat_map(|report| &report.outcomes)
            .any(|outcome| outcome.result.is_err())
    );
    assert!(!canceled);
    assert_eq!(tree.capture_holder(), Some(p));
    assert_eq!(tree.parent(p).unwrap(), Some(a));
    assert_eq!(tree.children(a).unwrap(), &[p]);
    assert!(tree.children(b).unwrap().is_empty());
    owner.run(|| Ok(tree.reparent(p, a, 0))).unwrap().unwrap();
    let foreign = UiTree::new().unwrap();
    assert_eq!(
        tree.reparent(p, foreign.root(), 0),
        Err(UiError::ForeignNode)
    );
    assert_eq!(tree.reparent(a, p, 0), Err(UiError::Cycle));
    assert_eq!(tree.parent(p).unwrap(), Some(a));
}

#[test]
fn imperative_node_handlers_also_own_their_reactive_resources() {
    let local = Rc::new(Cell::new(None::<Signal<i32>>));
    let captured = Rc::clone(&local);
    let mut tree = UiTree::new().unwrap();
    let id = tree.append(tree.root(), box_style(0.0, 80.0)).unwrap();
    tree.set_pointer_handler(id, move |event, _, _| {
        if event.kind == PointerEventKind::Down {
            captured.set(Some(signal(5).unwrap()));
        }
    })
    .unwrap();
    tree.dispatch(
        &WindowEvent::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
            position: Point::new(10.0, 10.0),
            timestamp: Duration::ZERO,
        },
        viewport(),
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(local.get().unwrap().get().unwrap(), 5);
    tree.remove(id).unwrap();
    assert_eq!(local.get().unwrap().get(), Err(ReactiveError::Disposed));
}
