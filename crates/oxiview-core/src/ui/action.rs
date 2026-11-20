use super::{Node, NodeId, UiCommands, UiError, UiTree, View};
use oxiview_reactive::ReactiveError;
use std::{cell::RefCell, rc::Rc, time::Duration};

/// An enumerable user command associated with a retained node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    /// Stable application-qualified name used by Keymaps and saved settings.
    pub id: String,
    /// Human-readable description for shortcut settings, menus and accessibility.
    pub description: String,
}

impl Action {
    /// Creates metadata for a node's Action registration.
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
        }
    }
}

/// How a node's Action was invoked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionSource {
    /// Primary press and release inside the same control.
    Pointer,
    /// Keymap or ordinary keyboard control activation.
    Keyboard,
    /// Explicit application invocation.
    Programmatic,
}

/// One Action delivery to its registered node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionEvent {
    /// Node that owns the selected handler.
    pub target: NodeId,
    /// Stable identifier of the invoked Action.
    pub action: String,
    /// Invocation origin.
    pub source: ActionSource,
    /// Capture time of the completing input, if any.
    pub timestamp: Option<Duration>,
}

pub(super) type ActionHandler = dyn FnMut(&ActionEvent, &UiTree, &mut UiCommands);

pub(super) struct RegisteredAction {
    pub(super) action: Action,
    handler: Rc<RefCell<Box<ActionHandler>>>,
}

impl Node {
    fn actions(&self) -> &[RegisteredAction] {
        self.interaction
            .as_ref()
            .map_or(&[], |interaction| &interaction.actions)
    }
}

impl View {
    /// Registers a named, node-owned Action and its handler.
    pub fn on_action(
        mut self,
        action: Action,
        handler: impl FnMut(&ActionEvent, &UiTree, &mut UiCommands) + 'static,
    ) -> Self {
        self.setup.push(Box::new(move |tree, id, _| {
            tree.register_action(id, action, handler)
        }));
        self
    }

    /// Marks which registered Action a click, Space or Enter activates.
    pub fn activate_with(mut self, action: impl Into<String>) -> Self {
        let action = action.into();
        self.setup.push(Box::new(move |tree, id, _| {
            tree.set_activation_action(id, Some(action))
        }));
        self
    }
}

impl UiTree {
    /// Installs or replaces the node's handler for this Action identifier.
    pub fn register_action(
        &mut self,
        id: NodeId,
        action: Action,
        handler: impl FnMut(&ActionEvent, &UiTree, &mut UiCommands) + 'static,
    ) -> Result<(), UiError> {
        self.node(id)?;
        if action.id.trim().is_empty() {
            return Err(UiError::InvalidAction);
        }
        let registration = RegisteredAction {
            action,
            handler: Rc::new(RefCell::new(Box::new(handler))),
        };
        let actions = &mut self.node_mut(id)?.interaction_mut().actions;
        if let Some(old) = actions
            .iter_mut()
            .find(|old| old.action.id == registration.action.id)
        {
            *old = registration;
            self.cancel_activation_for(id)?;
        } else {
            actions.push(registration);
        }
        self.sync_action_semantics(id)
    }

    /// Returns the node's Action descriptions in registration order.
    pub fn actions(&self, id: NodeId) -> Result<Vec<Action>, UiError> {
        Ok(self
            .node(id)?
            .actions()
            .iter()
            .map(|registration| registration.action.clone())
            .collect())
    }

    /// Assigns the Action used for normal control activation.
    pub fn set_activation_action(
        &mut self,
        id: NodeId,
        action: Option<String>,
    ) -> Result<(), UiError> {
        let node = self.node_mut(id)?;
        if action.as_ref().is_some_and(|name| name.trim().is_empty()) {
            return Err(UiError::InvalidAction);
        }
        if node
            .interaction
            .as_ref()
            .and_then(|interaction| interaction.activation.as_ref())
            != action.as_ref()
        {
            node.interaction_mut().activation = action;
            self.cancel_activation_for(id)?;
        }
        Ok(())
    }

    /// Invokes the Action on this exact live target and applies its queued work.
    /// Returns false if the target cannot currently perform the named Action.
    pub fn invoke_action(&mut self, target: NodeId, action: &str) -> Result<bool, UiError> {
        self.direct_interaction(|tree| {
            tree.perform_action(target, action, ActionSource::Programmatic, None)
        })
    }

    pub(super) fn sync_action_semantics(&mut self, id: NodeId) -> Result<(), UiError> {
        let node = self.node_mut(id)?;
        if !node.actions().is_empty() {
            let actions = node
                .actions()
                .iter()
                .map(|registration| registration.action.id.clone())
                .collect();
            let semantics = node.semantics.get_or_insert_with(Default::default);
            semantics.actions = actions;
        }
        Ok(())
    }

    pub(super) fn activation_action(&self, id: NodeId) -> Result<Option<&str>, UiError> {
        let Some(interaction) = &self.node(id)?.interaction else {
            return Ok(None);
        };
        Ok(interaction.activation.as_deref().filter(|action| {
            interaction
                .actions
                .iter()
                .any(|registered| registered.action.id == *action)
        }))
    }

    pub(super) fn perform_focused_action(
        &mut self,
        action: &str,
        timestamp: Duration,
    ) -> Result<(), UiError> {
        let mut current = Some(self.focused_node().unwrap_or(self.root));
        while let Some(target) = current {
            let node = self.node(target)?;
            if node
                .actions()
                .iter()
                .any(|registration| registration.action.id == action)
            {
                self.perform_action(target, action, ActionSource::Keyboard, Some(timestamp))?;
                break;
            }
            current = node.parent;
        }
        Ok(())
    }

    pub(super) fn perform_action(
        &mut self,
        target: NodeId,
        action: &str,
        source: ActionSource,
        timestamp: Option<Duration>,
    ) -> Result<bool, UiError> {
        let Some(registration) = self
            .node(target)?
            .actions()
            .iter()
            .find(|registration| registration.action.id == action)
        else {
            return Ok(false);
        };
        if !self.interaction_available(target)? {
            return Ok(false);
        }
        let callback = Rc::clone(&registration.handler);
        let mut callback = callback
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)?;
        let owner = self.lifetime_owner(target)?;
        let event = ActionEvent {
            target,
            action: action.to_owned(),
            source,
            timestamp,
        };
        let mut commands = UiCommands::new(self);
        {
            let _active = self.mailbox.activate(&commands)?;
            let _span =
                tracing::trace_span!("ui_action_handler", ?target, action, ?source).entered();
            match owner.run(|| {
                callback(&event, self, &mut commands);
                Ok(())
            }) {
                Ok(()) => {}
                Err(ReactiveError::Disposed) => return Ok(false),
                Err(error) => return Err(super::reactive_error(error)),
            }
        }
        self.enqueue(commands)?;
        Ok(true)
    }
}
