use super::{
    Identities, Keymap, NodeId, Reservation, UiError, UiTree,
    input::{Command, Mutation, UiCommands},
};
use oxiview_reactive::Owner;
use oxiview_scene::Primitive;
use oxiview_style::Style;
use oxiview_text::RichText;
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

pub(super) struct CommandBuffer {
    pub(super) commands: RefCell<Vec<Command>>,
    pub(super) failed: Cell<bool>,
    pub(super) accepting: bool,
}
impl CommandBuffer {
    pub(super) fn new(accepting: bool) -> Rc<Self> {
        Rc::new(Self {
            commands: RefCell::new(Vec::new()),
            failed: Cell::new(false),
            accepting,
        })
    }
    pub(super) fn push(&self, command: Command) -> Result<(), UiError> {
        self.commands
            .try_borrow_mut()
            .map_err(|_| UiError::CommandBufferBusy)?
            .push(command);
        Ok(())
    }
}
#[derive(Default)]
struct State {
    pending: Vec<Command>,
    active: Option<Weak<CommandBuffer>>,
    closed: bool,
}

/// A thread-local sender for deferred UI changes and reactive property bindings.
/// Cloning it does not keep a UI tree or a node alive.
#[derive(Clone)]
pub struct UiCommandSender {
    state: Rc<RefCell<State>>,
    identities: Rc<RefCell<Identities>>,
    owner: Option<Owner>,
}
impl UiCommandSender {
    pub(super) fn new(identities: Rc<RefCell<Identities>>) -> Self {
        Self {
            state: Rc::new(RefCell::new(State::default())),
            identities,
            owner: None,
        }
    }
    pub(super) fn post(&self, command: Command) -> Result<(), UiError> {
        let mut state = self
            .state
            .try_borrow_mut()
            .map_err(|_| UiError::CommandBufferBusy)?;
        let active = state.active.as_ref().and_then(Weak::upgrade);
        let accepting = !state.closed && active.as_ref().is_none_or(|active| active.accepting);
        let command = if !accepting {
            if self.owner.is_none() {
                return Err(UiError::QueueClosed);
            }
            Command::CanceledBinding(command.node())
        } else {
            match self.owner {
                Some(owner) => Command::Scoped(owner, Box::new(command)),
                None => command,
            }
        };
        if let Some(active) = active {
            drop(state);
            active.push(command)
        } else {
            state.pending.push(command);
            Ok(())
        }
    }
    /// Internal bindings follow their node's scope. Explicit application senders
    /// remain unscoped so invalid manual commands keep their rejection outcomes.
    pub(super) fn scoped(&self, owner: Owner) -> Self {
        Self {
            owner: Some(owner),
            ..self.clone()
        }
    }
    /// Queues a style replacement, joining the active event batch when called in a handler.
    pub fn set_style(&self, node: NodeId, style: Style) -> Result<(), UiError> {
        self.post(Command::Style(node, style))
    }
    /// Queues a change to the node's inherited disabled state.
    pub fn set_disabled(&self, node: NodeId, disabled: bool) -> Result<(), UiError> {
        self.post(Command::Disabled(node, disabled))
    }
    /// Queues an application or binding request for logical focus.
    pub fn focus(&self, node: NodeId) -> Result<(), UiError> {
        self.post(Command::Focus(node))
    }
    /// Queues a validated Keymap replacement.
    pub fn set_keymap(&self, node: NodeId, keymap: Keymap) -> Result<(), UiError> {
        self.post(Command::Keymap(node, keymap))
    }
    /// Queues custom painting content; the framework retains the supplied primitives.
    pub fn set_content(&self, node: NodeId, content: Vec<Primitive>) -> Result<(), UiError> {
        self.post(Command::Content(node, content))
    }
    /// Queues replacement of a real text node's source string.
    pub fn set_text(&self, node: NodeId, text: impl Into<String>) -> Result<(), UiError> {
        self.set_rich_text(node, RichText::plain(text.into()))
    }
    /// Queues a rich-text content replacement in the active or next batch.
    pub fn set_rich_text(&self, node: NodeId, text: RichText) -> Result<(), UiError> {
        self.post(Command::Text(node, text))
    }
    /// Queues subtree removal with the normal lifecycle cleanup.
    pub fn remove(&self, node: NodeId) -> Result<(), UiError> {
        self.post(Command::Remove(node))
    }
    /// Queues a subtree move without changing identities or private scope state.
    pub fn reparent(&self, node: NodeId, parent: NodeId, index: usize) -> Result<(), UiError> {
        self.post(Command::Reparent(node, parent, index))
    }
    /// Reserves an identity and queues an attached node creation.
    pub fn append(&self, parent: NodeId, style: Style) -> Result<NodeId, UiError> {
        let reservation = Reservation::new(&self.identities)?;
        let id = reservation.id;
        self.post(Command::Create {
            reservation,
            parent,
            index: None,
            style,
        })?;
        Ok(id)
    }
    pub(super) fn mutate(&self, node: NodeId, mutation: Box<Mutation>) -> Result<(), UiError> {
        self.post(Command::Mutation(node, mutation))
    }
    pub(super) fn activate(&self, commands: &UiCommands) -> Result<ActiveBatch, UiError> {
        let mut state = self
            .state
            .try_borrow_mut()
            .map_err(|_| UiError::CommandBufferBusy)?;
        let previous = state.active.replace(Rc::downgrade(&commands.buffer));
        Ok(ActiveBatch {
            sender: self.clone(),
            previous,
        })
    }
    pub(super) fn take_pending(&self) -> Result<Vec<Command>, UiError> {
        let mut state = self
            .state
            .try_borrow_mut()
            .map_err(|_| UiError::CommandBufferBusy)?;
        Ok(std::mem::take(&mut state.pending))
    }
    pub(super) fn close(&self) -> Result<(), UiError> {
        self.state
            .try_borrow_mut()
            .map_err(|_| UiError::CommandBufferBusy)?
            .closed = true;
        Ok(())
    }
}
pub(super) struct ActiveBatch {
    sender: UiCommandSender,
    previous: Option<Weak<CommandBuffer>>,
}
impl Drop for ActiveBatch {
    fn drop(&mut self) {
        if let Ok(mut state) = self.sender.state.try_borrow_mut() {
            state.active = self.previous.take();
        } else {
            tracing::error!("UI active batch could not restore its command context");
        }
    }
}

impl UiTree {
    /// Returns a sender whose work is applied at the next dispatch or flush boundary.
    pub fn command_sender(&self) -> UiCommandSender {
        self.mailbox.clone()
    }
}
