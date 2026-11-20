use super::{
    Keymap, Node, NodeId, UiError, UiTree, View,
    action::RegisteredAction,
    focus::{FocusHandler, FocusScope},
    keyboard::{HeldKey, KeyboardState},
};
use oxiview_reactive::{ReactiveError, Signal, batch, effect, signal};
use oxiview_style::{Display, InteractionState};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Default)]
pub(super) struct NodeInteraction {
    pub(super) focus_handler: Option<Rc<RefCell<Box<FocusHandler>>>>,
    pub(super) return_target: Option<NodeId>,
    pub(super) actions: Vec<RegisteredAction>,
    pub(super) activation: Option<String>,
    pub(super) keymap: Option<Box<Keymap>>,
}

impl Node {
    pub(super) fn interaction_mut(&mut self) -> &mut NodeInteraction {
        self.interaction.get_or_insert_with(Box::default)
    }
}

pub(super) struct Interaction {
    pub(super) focused: Option<NodeId>,
    pub(super) focused_signal: Cell<Option<Signal<Option<NodeId>>>>,
    pub(super) keyboard_visible: bool,
    pub(super) armed: Option<ArmedActivation>,
    pub(super) keyboard: KeyboardState,
    active: bool,
    visible: bool,
}

impl Default for Interaction {
    fn default() -> Self {
        Self {
            focused: None,
            focused_signal: Cell::default(),
            keyboard_visible: true,
            armed: None,
            keyboard: KeyboardState::default(),
            active: true,
            visible: true,
        }
    }
}

pub(super) struct ArmedActivation {
    pub(super) target: NodeId,
    pub(super) action: String,
    pub(super) input: ArmedInput,
}

pub(super) enum ArmedInput {
    Pointer { inside: bool },
    Keyboard(HeldKey),
}

impl View {
    /// Sets the disabled state inherited by this node's subtree.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.setup.push(Box::new(move |tree, id, _| {
            tree.change_disabled(id, disabled)
        }));
        self
    }

    /// Binds disabled state to the Signals read by this computation.
    pub fn bind_disabled(
        mut self,
        mut compute: impl FnMut() -> Result<bool, ReactiveError> + 'static,
    ) -> Self {
        self.bindings.push(Box::new(move |id, sender| {
            effect(move || {
                sender
                    .set_disabled(id, compute()?)
                    .map_err(|error| ReactiveError::User(error.to_string()))
            })
            .map_err(super::reactive_error)?;
            Ok(())
        }));
        self
    }
}

impl UiTree {
    /// Returns the effective states used by visual bindings and themes.
    /// Focused indicates keyboard-visible focus in an active, visible window.
    pub fn interaction_state(&self, id: NodeId) -> Result<InteractionState, UiError> {
        Ok(InteractionState {
            hovered: self.pointer.hover.contains(&id),
            pressed: self.interaction.armed.as_ref().is_some_and(|press| {
                press.target == id && !matches!(press.input, ArmedInput::Pointer { inside: false })
            }),
            focused: self.interaction.focused == Some(id)
                && self.interaction.keyboard_visible
                && self.accepts_input(),
            disabled: self.effectively_disabled(id)?,
        })
    }

    /// Returns a reactive snapshot that is disposed together with this node.
    pub fn interaction_signal(&self, id: NodeId) -> Result<Signal<InteractionState>, UiError> {
        let state = &self.node(id)?.interaction_signal;
        if let Some(existing) = state.get() {
            return Ok(**existing);
        }
        let initial = self.interaction_state(id)?;
        let created = self
            .lifetime_owner(id)?
            .run(|| signal(initial))
            .map_err(super::reactive_error)?;
        let _ = state.set(Box::new(created));
        Ok(created)
    }

    /// Sets whether this node and its descendants accept focus and Actions.
    pub fn set_disabled(&mut self, id: NodeId, disabled: bool) -> Result<(), UiError> {
        self.direct_interaction(|tree| tree.change_disabled(id, disabled))
    }

    pub(super) fn effectively_disabled(&self, id: NodeId) -> Result<bool, UiError> {
        for node in self.ancestors(id) {
            if node?.disabled {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn interaction_available(&self, id: NodeId) -> Result<bool, UiError> {
        for node in self.ancestors(id) {
            let node = node?;
            if node.hidden || node.style.layout.display == Display::None || node.disabled {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn change_disabled(&mut self, id: NodeId, disabled: bool) -> Result<(), UiError> {
        if self.node(id)?.disabled == disabled {
            return Ok(());
        }
        let scope = FocusScope::Subtree(id);
        let recovery = if disabled {
            self.plan_focus_loss(scope)?
        } else {
            None
        };
        self.node_mut(id)?.disabled = disabled;
        if disabled {
            self.apply_focus_loss(scope, recovery)?;
        }
        self.publish_subtree_interaction(id)
    }

    pub(super) fn publish_subtree_interaction(&mut self, subtree: NodeId) -> Result<(), UiError> {
        self.publish_interaction(self.postorder(subtree)?)
    }

    pub(super) fn publish_interaction(
        &mut self,
        changed: impl IntoIterator<Item = NodeId>,
    ) -> Result<(), UiError> {
        let mut updates = Vec::new();
        for id in changed {
            let node = self.node(id)?;
            let signal = node.interaction_signal.get().map(|signal| **signal);
            if node.theme.is_some() {
                self.mark_interaction_style(id)?;
            }
            if let Some(signal) = signal {
                updates.push((signal, self.interaction_state(id)?));
            }
        }
        let focused = self.interaction.focused_signal.get();
        if updates.is_empty() && focused.is_none() {
            return Ok(());
        }
        batch(|| {
            if let Some(signal) = focused {
                signal.set_if_changed(self.interaction.focused)?;
            }
            for (signal, state) in updates {
                signal.set_if_changed(state)?;
            }
            Ok(())
        })
        .map_err(super::reactive_error)
    }

    pub(super) fn accepts_input(&self) -> bool {
        self.interaction.active && self.interaction.visible
    }

    pub(super) fn update_window_input(
        &mut self,
        active: Option<bool>,
        visible: Option<bool>,
    ) -> Result<(), UiError> {
        let was_active = self.accepts_input();
        if let Some(active) = active {
            self.interaction.active = active;
        }
        if let Some(visible) = visible {
            self.interaction.visible = visible;
        }
        if was_active == self.accepts_input() {
            return Ok(());
        }
        let mut canceled = None;
        if !self.accepts_input() {
            canceled = self.interaction.armed.take().map(|press| press.target);
            self.interaction.keyboard.clear_sequence();
        }
        self.publish_interaction(self.interaction.focused.into_iter().chain(canceled))
    }

    pub(super) fn set_armed(&mut self, armed: Option<ArmedActivation>) -> Result<(), UiError> {
        let next_target = armed.as_ref().map(|press| press.target);
        let old = std::mem::replace(&mut self.interaction.armed, armed);
        self.publish_interaction(old.map(|press| press.target).into_iter().chain(next_target))
    }

    pub(super) fn cancel_activation_for(&mut self, id: NodeId) -> Result<(), UiError> {
        if self
            .interaction
            .armed
            .as_ref()
            .is_some_and(|press| press.target == id)
        {
            self.set_armed(None)?;
        }
        Ok(())
    }

    pub(super) fn take_arm(
        &mut self,
        matches: impl FnOnce(&ArmedInput) -> bool,
    ) -> Result<Option<ArmedActivation>, UiError> {
        if self
            .interaction
            .armed
            .as_ref()
            .is_some_and(|press| matches(&press.input))
        {
            let armed = self.interaction.armed.take();
            self.publish_interaction(armed.iter().map(|press| press.target))?;
            Ok(armed)
        } else {
            Ok(None)
        }
    }

    pub(super) fn pointer_press_inside(&mut self, path: &[NodeId]) -> Result<(), UiError> {
        if let Some(press) = &mut self.interaction.armed
            && let ArmedInput::Pointer { inside } = &mut press.input
            && *inside != path.contains(&press.target)
        {
            *inside = !*inside;
            let target = press.target;
            self.publish_interaction([target])?;
        }
        Ok(())
    }
}
