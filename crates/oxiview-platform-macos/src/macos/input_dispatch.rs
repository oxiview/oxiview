//! Nonblocking ordering between AppKit and the window thread. A raw input
//! turn can move focus or change the selection. Subsequent native keys wait
//! in capture order until that turn has published the editor's new snapshot.

use super::{Native, RESIZE_DEPTH, keyboard, native_time, text_input::Session};
use objc2::{Message, rc::Retained};
use objc2_app_kit::{NSEvent, NSTextInputContext};
use objc2_foundation::NSArray;
use oxiview_platform::{
    KeyCode, PlatformCommand, TextInputSession, TextInputSnapshot, WindowEvent,
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
};

enum Capture {
    Event(WindowEvent),
    Key(Retained<NSEvent>, bool),
}

#[derive(Clone, Copy)]
enum Stroke {
    Raw,
    Text(TextInputSession),
}

#[derive(Default)]
pub(super) struct Dispatch {
    enabled: Cell<bool>,
    draining: Cell<bool>,
    flushing: Cell<bool>,
    pending: RefCell<VecDeque<Capture>>,
    completed: Cell<u64>,
    session: RefCell<Option<Session>>,
    retired: Cell<Option<TextInputSession>>,
    strokes: RefCell<BTreeMap<u16, Stroke>>,
}

impl Dispatch {
    pub(super) fn session(&self) -> Option<Session> {
        self.session.borrow().as_ref().cloned()
    }

    pub(super) fn context(&self) -> Option<Retained<NSTextInputContext>> {
        self.session().map(|session| session.context)
    }

    fn retire(&self, report_cancel: bool) {
        let old = self.session.borrow_mut().take();
        if let Some(old) = old {
            self.retired.set(Some(old.id));
            old.retire(report_cancel);
        }
    }

    pub(super) fn close(&self) {
        self.retire(false);
        self.pending.borrow_mut().clear();
        self.strokes.borrow_mut().clear();
    }

    pub(super) fn has_pending(&self) -> bool {
        !self.pending.borrow().is_empty()
    }
}

impl Native {
    pub(super) fn route_input(&self, event: WindowEvent) {
        if self.closed.get() {
            return;
        }
        if self.input.enabled.get() {
            self.input
                .pending
                .borrow_mut()
                .push_back(Capture::Event(event));
            self.drain_input();
        } else {
            self.shared.emit(event);
        }
    }

    pub(super) fn route_key(&self, event: &NSEvent, pressed: bool) {
        if self.closed.get() {
            return;
        }
        if self.input.enabled.get() {
            self.input
                .pending
                .borrow_mut()
                .push_back(Capture::Key(event.retain(), pressed));
            self.drain_input();
        } else {
            let transition = keyboard::transition(event, pressed);
            let escape = matches!(
                transition,
                WindowEvent::Key {
                    code: KeyCode::Escape,
                    pressed: true,
                    repeat: false,
                    ..
                }
            );
            self.emit_key(transition, event, escape);
        }
    }

    pub(super) fn input_activation(&self, active: bool) {
        if !active {
            // A buffered key never migrates into the next key window or field.
            // Any text callbacks already captured retain their preceding order.
            self.input
                .pending
                .borrow_mut()
                .retain(|capture| !matches!(capture, Capture::Key(_, _)));
            self.input.retire(true);
        }
        self.route_input(WindowEvent::ActivationChanged {
            active,
            timestamp: native_time(),
        });
        if active && let Some(context) = self.input.context() {
            context.activate();
        }
    }

    pub(super) fn invalidate_text_coordinates(&self) {
        if let Some(context) = self.input.context() {
            context.invalidateCharacterCoordinates();
        }
    }

    pub(super) fn flush_editor_commands(&self) {
        if self.input.flushing.replace(true) {
            return;
        }
        let flushes = match self.shared.lock() {
            Ok(mut queue) => std::mem::take(&mut queue.platform_flushes),
            Err(_) => std::process::abort(),
        };
        for flush in flushes {
            for command in flush.commands {
                match command {
                    PlatformCommand::TextInput(snapshot) => self.publish_text(snapshot),
                    PlatformCommand::Clipboard(request) => {
                        let reply = super::desktop_services::clipboard(request);
                        self.route_input(reply);
                    }
                    PlatformCommand::FilePanel(request) => self.panels.show(self, request),
                    PlatformCommand::Close => {
                        let _ = self.shared.close();
                    }
                    _ => tracing::warn!("unknown platform command"),
                }
            }
            if let Some(completed) = flush.completed_input {
                self.input.completed.set(completed);
            }
        }
        self.input.flushing.set(false);
        self.drain_input();
        if self
            .shared
            .lock()
            .is_ok_and(|queue| !queue.platform_flushes.is_empty())
        {
            self.shared.signal();
        }
    }

    fn publish_text(&self, snapshot: Option<TextInputSnapshot>) {
        self.input.enabled.set(true);
        let Some(snapshot) = snapshot else {
            self.input.retire(false);
            return;
        };
        if let Some(session) = self.input.session()
            && session.id == snapshot.session
        {
            session.publish(snapshot);
            return;
        }
        if self.input.retired.get() == Some(snapshot.session) {
            // An activation-loss callback may beat a window-thread flush for
            // the outgoing editor. Reactivation requires a new focus lifetime.
            return;
        }
        self.input.retire(false);
        let session = Session::new(self, snapshot);
        self.input.session.replace(Some(session.clone()));
        if self.window.isKeyWindow() {
            session.context.activate();
        }
        session.context.invalidateCharacterCoordinates();
    }

    fn drain_input(&self) {
        if self.closed.get()
            || self.input.flushing.get()
            || RESIZE_DEPTH.get() != 0
            || self.input.draining.replace(true)
        {
            return;
        }
        while self.shared.lock().is_ok_and(|queue| {
            queue.alive && !queue.close && queue.input_sequence == self.input.completed.get()
        }) {
            let capture = self.input.pending.borrow_mut().pop_front();
            match capture {
                Some(Capture::Event(event)) => self.shared.emit_input([event]),
                Some(Capture::Key(event, pressed)) => self.dispatch_key(&event, pressed),
                None => break,
            }
        }
        self.input.draining.set(false);
    }

    fn dispatch_key(&self, native: &NSEvent, pressed: bool) {
        // AppKit may route native key equivalents before the responder's keyDown:.
        let transition = keyboard::transition(native, pressed);
        let WindowEvent::Key {
            code,
            modifiers,
            repeat,
            ..
        } = &transition
        else {
            return;
        };
        let physical = native.keyCode();
        if !pressed {
            if !matches!(
                self.input.strokes.borrow_mut().remove(&physical),
                Some(Stroke::Text(_))
            ) {
                self.emit_key(transition, native, false);
            }
            return;
        }
        let held = self.input.strokes.borrow().get(&physical).copied();
        let session = self.input.session();
        if *repeat {
            match held {
                Some(Stroke::Raw) => self.emit_key(transition, native, false),
                Some(Stroke::Text(id)) => {
                    if let Some(session) = session.filter(|session| session.id == id) {
                        session.key(native, &transition);
                    }
                }
                None => self.emit_key(transition, native, false),
            }
            return;
        }

        let text = session.filter(|session| !modifiers.meta && session.key(native, &transition));
        let route = text
            .as_ref()
            .map_or(Stroke::Raw, |session| Stroke::Text(session.id));
        self.input.strokes.borrow_mut().insert(physical, route);
        if text.is_none() {
            let escape = *code == KeyCode::Escape;
            self.emit_key(transition, native, escape);
        }
    }

    fn emit_key(&self, transition: WindowEvent, native: &NSEvent, escape_press: bool) {
        let interrupt_drag = escape_press && NSEvent::pressedMouseButtons() != 0;
        if self.input.enabled.get() {
            let interruption = interrupt_drag.then(|| WindowEvent::PointerInterrupted {
                timestamp: native_time(),
            });
            self.shared
                .emit_input(std::iter::once(transition).chain(interruption));
        } else {
            self.shared.emit(transition);
            // Preserve the original responder path for M5 Escape during a drag.
            if interrupt_drag {
                self.view
                    .interpretKeyEvents(&NSArray::from_slice(&[native]));
            }
        }
    }
}
