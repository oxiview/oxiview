use super::{NodeId, UiCommands, UiError, UiTree, View, interaction::ArmedInput};
use oxiview_reactive::{ReactiveError, Signal, signal};
use oxiview_style::Display;
use std::{cell::RefCell, collections::HashSet, rc::Rc};

/// Whether a node can receive logical focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FocusPolicy {
    /// This node is skipped by all focus requests.
    #[default]
    None,
    /// An explicit request or a primary press can focus this node.
    Programmatic,
    /// The node also participates in forward and backward Tab traversal.
    Tab,
}

/// A node gained or lost logical focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusEventKind {
    /// Logical focus moved away from this node.
    Blur,
    /// Logical focus moved to this node.
    Focus,
}

/// Direct notification of one side of a logical focus transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusEvent {
    /// The retained node receiving the notification.
    pub target: NodeId,
    /// Whether the node lost or gained focus.
    pub kind: FocusEventKind,
    /// The node at the other end of the transition, if present.
    pub related_target: Option<NodeId>,
}

pub(super) type FocusHandler = dyn FnMut(&FocusEvent, &UiTree, &mut UiCommands);

#[derive(Clone, Copy)]
pub(super) enum FocusScope<'a> {
    Node(NodeId),
    Subtree(NodeId),
    Descendants(NodeId),
    Subtrees(&'a HashSet<NodeId>),
}

impl FocusScope<'_> {
    fn includes(self, tree: &UiTree, id: NodeId) -> Result<bool, UiError> {
        match self {
            Self::Node(target) => Ok(target == id),
            Self::Subtree(root) => tree.descendant(id, root),
            Self::Descendants(root) => Ok(id != root && tree.descendant(id, root)?),
            Self::Subtrees(roots) => {
                if roots.is_empty() {
                    return Ok(false);
                }
                for node in tree.ancestors(id) {
                    if roots.contains(&node?.id) {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }
}

pub(super) struct FocusRecovery(Option<NodeId>);

impl View {
    /// Opts this node into focus and optionally Tab traversal.
    pub fn focusable(mut self, policy: FocusPolicy) -> Self {
        self.setup.push(Box::new(move |tree, id, _| {
            tree.change_focus_policy(id, policy)
        }));
        self
    }

    /// Registers a callback for logical focus gained or lost by this node.
    pub fn on_focus(
        mut self,
        handler: impl FnMut(&FocusEvent, &UiTree, &mut UiCommands) + 'static,
    ) -> Self {
        self.setup.push(Box::new(move |tree, id, _| {
            tree.set_focus_handler(id, handler)
        }));
        self
    }

    /// Prefers this return target when the mounted subtree loses focus.
    pub fn focus_return_target(mut self, target: NodeId) -> Self {
        self.setup.push(Box::new(move |tree, id, _| {
            tree.set_focus_return_target(id, Some(target))
        }));
        self
    }
}

impl UiTree {
    /// Returns the focused identity, including while the window is inactive.
    pub fn focused_node(&self) -> Option<NodeId> {
        self.interaction.focused
    }

    /// Returns a window-owned reactive Signal for changes of logical identity.
    pub fn focused_signal(&self) -> Result<Signal<Option<NodeId>>, UiError> {
        if let Some(existing) = self.interaction.focused_signal.get() {
            self.node(self.root)?;
            return Ok(existing);
        }
        let created = self
            .lifetime_owner(self.root)?
            .run(|| signal(self.interaction.focused))
            .map_err(super::reactive_error)?;
        self.interaction.focused_signal.set(Some(created));
        Ok(created)
    }

    /// Requests focus for an eligible node. Handler requests use UiCommands.
    pub fn focus(&mut self, id: NodeId) -> Result<(), UiError> {
        self.direct_interaction(|tree| tree.request_focus(id))
    }

    /// Clears logical focus and any keyboard press armed on that focus.
    pub fn clear_focus(&mut self) -> Result<(), UiError> {
        self.direct_interaction(|tree| tree.transition_focus(None, None))
    }

    /// Sets focus eligibility for this node.
    pub fn set_focus_policy(&mut self, id: NodeId, policy: FocusPolicy) -> Result<(), UiError> {
        self.direct_interaction(|tree| tree.change_focus_policy(id, policy))
    }

    /// Sets a preferred focus target when an operation suppresses or removes
    /// this node or a subtree containing it. The target is rechecked at recovery.
    pub fn set_focus_return_target(
        &mut self,
        id: NodeId,
        target: Option<NodeId>,
    ) -> Result<(), UiError> {
        self.node(id)?;
        if let Some(target) = target {
            self.node(target)?;
        }
        self.node_mut(id)?.interaction_mut().return_target = target;
        Ok(())
    }

    /// Installs the node's direct callback for focus and blur notifications.
    pub fn set_focus_handler(
        &mut self,
        id: NodeId,
        handler: impl FnMut(&FocusEvent, &UiTree, &mut UiCommands) + 'static,
    ) -> Result<(), UiError> {
        self.node_mut(id)?.interaction_mut().focus_handler =
            Some(Rc::new(RefCell::new(Box::new(handler))));
        Ok(())
    }

    pub(super) fn request_focus(&mut self, id: NodeId) -> Result<(), UiError> {
        if !self.focus_eligible(id)? {
            return Err(UiError::FocusUnavailable);
        }
        self.transition_focus(Some(id), None)
    }

    pub(super) fn focus_eligible(&self, id: NodeId) -> Result<bool, UiError> {
        Ok(self.node(id)?.focus_policy != FocusPolicy::None && self.interaction_available(id)?)
    }

    pub(super) fn change_focus_policy(
        &mut self,
        id: NodeId,
        policy: FocusPolicy,
    ) -> Result<(), UiError> {
        let old = self.node(id)?.focus_policy;
        if old == policy {
            return Ok(());
        }
        if policy == FocusPolicy::None && self.focused_node() == Some(id) {
            self.lose_interaction(FocusScope::Node(id))?;
        }
        self.node_mut(id)?.focus_policy = policy;
        Ok(())
    }

    pub(super) fn tab_focus(&mut self, backwards: bool) -> Result<(), UiError> {
        let order = self.focus_order()?;
        let first_tab = |id: &&NodeId| {
            self.node(**id)
                .is_ok_and(|node| node.focus_policy == FocusPolicy::Tab)
        };
        let index = order
            .iter()
            .position(|id| Some(*id) == self.interaction.focused);
        let next = match (backwards, index) {
            (false, Some(index)) => order
                .iter()
                .skip(index + 1)
                .chain(order.iter().take(index + 1))
                .find(first_tab),
            (true, Some(index)) => order
                .iter()
                .take(index)
                .rev()
                .chain(order.iter().skip(index).rev())
                .find(first_tab),
            (false, None) => order.iter().find(first_tab),
            (true, None) => order.iter().rev().find(first_tab),
        };
        let next = next.copied().or(self.focused_node());
        if next == self.focused_node() {
            self.take_arm(|source| matches!(source, ArmedInput::Keyboard(_)))?;
        }
        self.transition_focus(next, Some(true))
    }

    // Keep a programmatic-only focused node as a position marker in preorder.
    fn focus_order(&self) -> Result<Vec<NodeId>, UiError> {
        let mut order = Vec::new();
        let mut pending = vec![self.root];
        while let Some(id) = pending.pop() {
            let node = self.node(id)?;
            if node.hidden || node.style.layout.display == Display::None || node.disabled {
                continue;
            }
            if node.focus_policy == FocusPolicy::Tab || Some(id) == self.interaction.focused {
                order.push(id);
            }
            pending.extend(node.children.iter().rev().copied());
        }
        Ok(order)
    }

    pub(super) fn plan_focus_loss(
        &self,
        scope: FocusScope<'_>,
    ) -> Result<Option<FocusRecovery>, UiError> {
        let Some(old) = self.interaction.focused else {
            return Ok(None);
        };
        if !scope.includes(self, old)? {
            return Ok(None);
        }
        for id in self.path(old)?.into_iter().rev() {
            if let Some(target) = self
                .node(id)?
                .interaction
                .as_ref()
                .and_then(|interaction| interaction.return_target)
                && scope.includes(self, id)?
                && self.focus_eligible(target).unwrap_or(false)
                && !scope.includes(self, target)?
            {
                return Ok(Some(FocusRecovery(Some(target))));
            }
        }
        let order = self.focus_order()?;
        if let Some(index) = order.iter().position(|id| *id == old) {
            for &next in order
                .iter()
                .skip(index + 1)
                .chain(order.iter().take(index).rev())
            {
                if !scope.includes(self, next)? {
                    return Ok(Some(FocusRecovery(Some(next))));
                }
            }
        }
        Ok(Some(FocusRecovery(None)))
    }

    pub(super) fn lose_interaction(&mut self, scope: FocusScope<'_>) -> Result<(), UiError> {
        let recovery = self.plan_focus_loss(scope)?;
        self.apply_focus_loss(scope, recovery)
    }

    pub(super) fn apply_focus_loss(
        &mut self,
        scope: FocusScope<'_>,
        recovery: Option<FocusRecovery>,
    ) -> Result<(), UiError> {
        if let Some(context) = self.interaction.keyboard.pending_context()
            && scope.includes(self, context)?
        {
            self.interaction.keyboard.clear_sequence();
        }
        let canceled = if let Some(press) = &self.interaction.armed
            && scope.includes(self, press.target)?
        {
            self.interaction.armed.take().map(|press| press.target)
        } else {
            None
        };
        let old = self.interaction.focused;
        if let Some(FocusRecovery(next)) = recovery {
            self.transition_focus(next, None)?;
        }
        if let Some(target) = canceled
            && (Some(target) != old || self.interaction.focused == old)
        {
            self.publish_interaction([target])?;
        }
        Ok(())
    }

    pub(super) fn transition_focus(
        &mut self,
        next: Option<NodeId>,
        keyboard_visible: Option<bool>,
    ) -> Result<(), UiError> {
        let old = self.interaction.focused;
        let visible = keyboard_visible.unwrap_or(self.interaction.keyboard_visible);
        if old == next && self.interaction.keyboard_visible == visible {
            return Ok(());
        }
        self.interaction.keyboard_visible = visible;
        self.interaction.focused = next;
        let mut canceled = None;
        if old != next {
            self.interaction.keyboard.clear_sequence();
            if self
                .interaction
                .armed
                .as_ref()
                .is_some_and(|press| matches!(press.input, ArmedInput::Keyboard(_)))
            {
                canceled = self.interaction.armed.take().map(|press| press.target);
            }
        }
        let mut commands = UiCommands::new(self);
        {
            let _active = self.mailbox.activate(&commands)?;
            self.publish_interaction(old.into_iter().chain(next).chain(canceled))?;
            if old != next {
                if let Some(target) = old {
                    self.deliver_focus(target, FocusEventKind::Blur, next, &mut commands)?;
                }
                if let Some(target) = next {
                    self.deliver_focus(target, FocusEventKind::Focus, old, &mut commands)?;
                }
            }
        }
        self.enqueue(commands)
    }

    fn deliver_focus(
        &self,
        target: NodeId,
        kind: FocusEventKind,
        related_target: Option<NodeId>,
        commands: &mut UiCommands,
    ) -> Result<(), UiError> {
        let Some(callback) = self
            .node(target)?
            .interaction
            .as_ref()
            .and_then(|interaction| interaction.focus_handler.as_ref())
            .cloned()
        else {
            return Ok(());
        };
        let mut callback = callback
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)?;
        let _span = tracing::trace_span!("ui_focus_handler", ?kind, ?target).entered();
        match self.lifetime_owner(target)?.run(|| {
            callback(
                &FocusEvent {
                    target,
                    kind,
                    related_target,
                },
                self,
                commands,
            );
            Ok(())
        }) {
            Ok(()) | Err(ReactiveError::Disposed) => Ok(()),
            Err(error) => Err(super::reactive_error(error)),
        }
    }
}
