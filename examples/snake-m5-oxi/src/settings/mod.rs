//! Mounted inline settings, application commands and their window-local Signals.

pub(crate) mod storage;
mod view;
use oxiview_core::NodeId;
use oxiview_reactive::{ReactiveError, Signal, signal};
use std::{cell::Cell, rc::Rc, sync::mpsc::Sender};
pub(crate) use view::{opener, panel};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Command {
    Open,
    Close,
    Appearance(bool),
    SelectAction(bool),
    Record,
    Retry,
}

#[derive(Default)]
pub(crate) struct Controls {
    pub(crate) opener: Cell<Option<NodeId>>,
    pub(crate) panel: Cell<Option<NodeId>>,
    pub(crate) appearance: Cell<Option<NodeId>>,
    pub(crate) selection: Cell<Option<NodeId>>,
    pub(crate) record: Cell<Option<NodeId>>,
    pub(crate) close: Cell<Option<NodeId>>,
}

#[derive(Clone)]
pub(crate) struct Settings {
    pub(crate) open: Signal<bool>,
    pub(crate) palette_label: Signal<String>,
    pub(crate) action_label: Signal<String>,
    pub(crate) shortcut_label: Signal<String>,
    pub(crate) footer: Signal<String>,
    pub(crate) recording: Signal<bool>,
    pub(crate) status: Signal<String>,
    pub(crate) theme_error: Signal<Option<String>>,
    pub(crate) controls: Rc<Controls>,
    pub(crate) send: Sender<Command>,
    pub(crate) builder_panel: bool,
}

impl Settings {
    pub(crate) fn new(send: Sender<Command>, builder_panel: bool) -> Result<Self, ReactiveError> {
        Ok(Self {
            open: signal(false)?,
            palette_label: signal(String::new())?,
            action_label: signal(String::new())?,
            shortcut_label: signal(String::new())?,
            footer: signal(String::new())?,
            recording: signal(false)?,
            status: signal("Left and Right change an option. Tab moves between controls.".into())?,
            theme_error: signal(None)?,
            controls: Rc::new(Controls::default()),
            send,
            builder_panel,
        })
    }
}
