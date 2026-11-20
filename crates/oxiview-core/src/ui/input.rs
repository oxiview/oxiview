use super::{
    ActionSource, FocusPolicy, Keymap, NodeId, Reservation, UiError, UiTree,
    focus::FocusScope,
    interaction::{ArmedActivation, ArmedInput},
};
use oxiview_base::{Point, Size};
use oxiview_platform::{PointerButton, WindowEvent};
use oxiview_reactive::{Owner, ReactiveError};
use oxiview_scene::Primitive;
use oxiview_style::{Display, Style};
use oxiview_text::RichText;
use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration};

const COMMAND_BUDGET: usize = 4096;
const EVENT_BUDGET: usize = 16384;
pub(super) type Handler<'a> = dyn FnMut(&UiPointerEvent, &UiTree, &mut UiCommands) + 'a;
pub(super) type NodeHandler = dyn FnMut(&UiPointerEvent, &UiTree, &mut UiCommands);
pub(super) type ReportHandler = dyn FnMut(&UiBatchReport, &UiTree, &mut UiCommands);
pub(super) type Mutation =
    dyn FnOnce(&mut UiTree, &mut Handler<'_>, &mut Work) -> Result<(), UiError>;

/// Framework pointer event kind. Enter and leave are direct notifications.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerEventKind {
    /// Pointer position changed or was freshly established.
    Move,
    /// A mouse button was pressed.
    Down,
    /// A mouse button was released.
    Up,
    /// A node joined the effective hover path.
    Enter,
    /// A node left the effective hover path.
    Leave,
    /// The current captured interaction was interrupted.
    Cancel,
}

/// One delivery on a frozen target/ancestor route, in logical coordinates.
#[derive(Clone, Copy, Debug)]
pub struct UiPointerEvent {
    /// Event kind.
    pub kind: PointerEventKind,
    /// Original target, unchanged during bubbling.
    pub target: NodeId,
    /// Node currently receiving this delivery.
    pub current_target: NodeId,
    /// Position in logical window coordinates.
    pub position: Point,
    /// Position relative to the current target's border-box origin.
    pub local_position: Point,
    /// Pressed-button mask: primary 1, secondary 2, middle 4, other buttons 8.
    pub buttons: u8,
    /// The button that changed, for down/up events.
    pub button: Option<PointerButton>,
    /// Original platform input timestamp.
    pub timestamp: Duration,
}

/// Result of one deferred command, in submission order.
#[derive(Clone, Debug)]
pub struct UiCommandOutcome {
    /// Zero-based position in this batch.
    pub sequence: usize,
    /// Relevant node identity, including failed creation reservations.
    pub node: Option<NodeId>,
    /// Success or a structured rejection reason.
    pub result: Result<(), UiError>,
}
/// Results for a complete command batch. They do not imply frame presentation.
#[derive(Clone, Debug)]
pub struct UiBatchReport {
    /// Window-tree-local batch identity.
    pub batch: u64,
    /// All command outcomes in submission order.
    pub outcomes: Vec<UiCommandOutcome>,
    /// Automatic binding/reconciliation jobs canceled because their owner was
    /// disposed or the dispatch stopped accepting work. These are not successful
    /// mutations and do not conceal explicit rejections; sequences may skip them.
    pub canceled_bindings: usize,
}
/// Observable work produced by one call to dispatch.
#[derive(Default, Debug)]
pub struct UiDispatchResult {
    /// Retained visual work needs a frame. The application can coalesce invalidation.
    pub changed: bool,
    /// Number of pointer event deliveries, including bubbling and boundaries.
    pub events: usize,
    /// Framework input or the caller's recorder consumed this key transition.
    pub key_handled: bool,
    /// Completed and canceled command batches, in execution order.
    pub reports: Vec<UiBatchReport>,
}

pub(super) enum Command {
    Scoped(Owner, Box<Command>),
    CanceledBinding(NodeId),
    Create {
        reservation: Reservation,
        parent: NodeId,
        index: Option<usize>,
        style: Style,
    },
    Style(NodeId, Style),
    Content(NodeId, Vec<Primitive>),
    Text(NodeId, RichText),
    Handler(NodeId, Box<NodeHandler>),
    Mutation(NodeId, Box<Mutation>),
    Rejected(NodeId, UiError),
    Visible(NodeId, bool),
    Remove(NodeId),
    Clear(NodeId),
    Reparent(NodeId, NodeId, usize),
    Capture(NodeId),
    Release(NodeId),
    Focus(NodeId),
    ClearFocus(NodeId),
    FocusPolicy(NodeId, FocusPolicy),
    FocusReturn(NodeId, Option<NodeId>),
    Disabled(NodeId, bool),
    Keymap(NodeId, Keymap),
    InvokeAction(NodeId, String),
}
impl Command {
    pub(super) fn node(&self) -> NodeId {
        match self {
            Self::Scoped(_, command) => command.node(),
            Self::CanceledBinding(node) => *node,
            Self::Create { reservation, .. } => reservation.id,
            Self::Style(id, _)
            | Self::Content(id, _)
            | Self::Text(id, _)
            | Self::Handler(id, _)
            | Self::Mutation(id, _)
            | Self::Rejected(id, _)
            | Self::Visible(id, _)
            | Self::Remove(id)
            | Self::Clear(id)
            | Self::Reparent(id, _, _)
            | Self::Capture(id)
            | Self::Release(id)
            | Self::Focus(id)
            | Self::ClearFocus(id)
            | Self::FocusPolicy(id, _)
            | Self::FocusReturn(id, _)
            | Self::Disabled(id, _)
            | Self::Keymap(id, _)
            | Self::InvokeAction(id, _) => *id,
        }
    }
    fn canceled_binding(&self) -> Result<bool, UiError> {
        if matches!(self, Self::CanceledBinding(_)) {
            return Ok(true);
        }
        if let Self::Scoped(owner, _) = self {
            match owner.run(|| Ok(())) {
                Ok(()) => Ok(false),
                Err(ReactiveError::Disposed) => Ok(true),
                Err(error) => Err(UiError::Reactive(error.to_string())),
            }
        } else {
            Ok(false)
        }
    }
}
pub(super) struct Batch {
    number: u64,
    commands: Vec<Command>,
    notify: bool,
}

/// Mutations queued by a handler. Reads continue to observe pre-command state.
/// Creation reserves an ID immediately; dropping ID copies never cancels it.
pub struct UiCommands {
    root: NodeId,
    identities: Rc<RefCell<super::Identities>>,
    pub(super) buffer: Rc<super::mailbox::CommandBuffer>,
    notify: bool,
    stop: bool,
}
impl UiCommands {
    pub(super) fn new(tree: &UiTree) -> Self {
        Self {
            root: tree.root,
            identities: Rc::clone(&tree.identities),
            buffer: super::mailbox::CommandBuffer::new(!tree.closing),
            notify: true,
            stop: false,
        }
    }
    /// Queues an append and returns its reserved identity.
    pub fn append(&mut self, parent: NodeId, style: Style) -> Result<NodeId, UiError> {
        self.create(parent, None, style)
    }
    /// Queues creation at an insertion position evaluated when the command executes.
    pub fn insert(
        &mut self,
        parent: NodeId,
        index: usize,
        style: Style,
    ) -> Result<NodeId, UiError> {
        self.create(parent, Some(index), style)
    }
    fn create(
        &mut self,
        parent: NodeId,
        index: Option<usize>,
        style: Style,
    ) -> Result<NodeId, UiError> {
        if !self.buffer.accepting {
            return Err(UiError::QueueClosed);
        }
        let reservation = Reservation::new(&self.identities)?;
        let id = reservation.id;
        self.buffer.push(Command::Create {
            reservation,
            parent,
            index,
            style,
        })?;
        Ok(id)
    }
    /// Queues a validated style replacement.
    pub fn set_style(&mut self, id: NodeId, style: Style) {
        self.push(Command::Style(id, style));
    }
    /// Queues replacement of node-local painting content.
    pub fn set_content(&mut self, id: NodeId, content: Vec<Primitive>) {
        self.push(Command::Content(id, content));
    }
    /// Queues replacement of a text node's source string and invalidates measurement.
    pub fn set_text(&mut self, id: NodeId, text: impl Into<String>) {
        self.set_rich_text(id, RichText::plain(text.into()));
    }
    /// Queues a rich-text content replacement.
    pub fn set_rich_text(&mut self, id: NodeId, text: RichText) {
        self.push(Command::Text(id, text));
    }
    /// Queues replacement of the node-owned pointer handler.
    pub fn set_pointer_handler(
        &mut self,
        id: NodeId,
        handler: impl FnMut(&UiPointerEvent, &UiTree, &mut UiCommands) + 'static,
    ) {
        self.push(Command::Handler(id, Box::new(handler)));
    }
    /// Queues complete subtree removal.
    pub fn remove(&mut self, id: NodeId) {
        self.push(Command::Remove(id));
    }
    /// Queues descendant removal while retaining the specified parent.
    pub fn clear(&mut self, id: NodeId) {
        self.push(Command::Clear(id));
    }
    /// Queues a subtree move, using an insertion index after conceptual detachment.
    pub fn reparent(&mut self, id: NodeId, parent: NodeId, index: usize) {
        self.push(Command::Reparent(id, parent, index));
    }
    /// Queues capture acquisition. Another holder must release explicitly first.
    pub fn capture(&mut self, id: NodeId) {
        self.push(Command::Capture(id));
    }
    /// Queues release by this holder. Already released capture is a no-op.
    pub fn release_capture(&mut self, id: NodeId) {
        self.push(Command::Release(id));
    }
    /// Queues hiding or showing this subtree without removing its nodes.
    pub fn set_visible(&mut self, id: NodeId, visible: bool) {
        self.push(Command::Visible(id, visible));
    }
    /// Requests focus using eligibility at the command's turn.
    pub fn focus(&mut self, id: NodeId) {
        self.push(Command::Focus(id));
    }
    /// Queues clearing logical focus.
    pub fn clear_focus(&mut self) {
        self.push(Command::ClearFocus(self.root));
    }
    /// Queues a change of focus eligibility.
    pub fn set_focus_policy(&mut self, id: NodeId, policy: FocusPolicy) {
        self.push(Command::FocusPolicy(id, policy));
    }
    /// Queues a preferred return target for focus recovery in this subtree.
    pub fn set_focus_return_target(&mut self, id: NodeId, target: Option<NodeId>) {
        self.push(Command::FocusReturn(id, target));
    }
    /// Queues a change of the disabled state inherited by this subtree.
    pub fn set_disabled(&mut self, id: NodeId, disabled: bool) {
        self.push(Command::Disabled(id, disabled));
    }
    /// Queues replacement of the Keymap for this focus context.
    pub fn set_keymap(&mut self, id: NodeId, keymap: Keymap) {
        self.push(Command::Keymap(id, keymap));
    }
    /// Queues an invocation on the designated live node.
    pub fn invoke_action(&mut self, target: NodeId, action: impl Into<String>) {
        self.push(Command::InvokeAction(target, action.into()));
    }
    /// Stops remaining ancestors of the current event without discarding commands.
    pub fn stop_propagation(&mut self) {
        self.stop = true;
    }
    pub(super) fn push(&mut self, command: Command) {
        let command = if self.buffer.accepting {
            command
        } else {
            Command::Rejected(command.node(), UiError::QueueClosed)
        };
        if self.buffer.push(command).is_err() {
            self.buffer.failed.set(true);
        }
    }
}

#[derive(Default)]
pub(super) struct PointerState {
    pub(super) capture: Option<NodeId>,
    pub(super) hover: Vec<NodeId>,
    position: Point,
    pressed: Vec<PointerButton>,
    timestamp: Duration,
    inside: bool,
    active: bool,
}
impl PointerState {
    fn buttons(&self) -> u8 {
        self.pressed.iter().fold(0, |mask, button| {
            mask | match button {
                PointerButton::Primary => 1,
                PointerButton::Secondary => 2,
                PointerButton::Middle => 4,
                _ => 8,
            }
        })
    }
}
#[derive(Default)]
pub(super) struct Work {
    result: UiDispatchResult,
    commands: usize,
    deleted_options: HashMap<NodeId, bool>,
}

impl UiTree {
    /// Installs a node-owned callback; event delivery still follows the fixed route.
    pub fn set_pointer_handler(
        &mut self,
        id: NodeId,
        handler: impl FnMut(&UiPointerEvent, &UiTree, &mut UiCommands) + 'static,
    ) -> Result<(), UiError> {
        self.node_mut(id)?.handler = Some(Rc::new(RefCell::new(Box::new(handler))));
        Ok(())
    }
    /// Removes a handler without changing geometric hit participation.
    pub fn clear_pointer_handler(&mut self, id: NodeId) -> Result<(), UiError> {
        self.node_mut(id)?.handler = None;
        Ok(())
    }
    /// Receives each complete batch before stationary-hover reconciliation.
    /// Callback mutations join the queue tail and never alter the completed report.
    pub fn set_batch_report_handler(
        &mut self,
        handler: impl FnMut(&UiBatchReport, &UiTree, &mut UiCommands) + 'static,
    ) -> Result<(), UiError> {
        if self.closed {
            return Err(UiError::Closed);
        }
        self.report_handler = Some(Rc::new(RefCell::new(Box::new(handler))));
        Ok(())
    }
    /// Removes the custom report callback; rejected commands remain logged.
    pub fn clear_batch_report_handler(&mut self) {
        self.report_handler = None;
    }
    /// Hides or shows this subtree without removing its nodes.
    pub fn set_visible(&mut self, id: NodeId, visible: bool) -> Result<(), UiError> {
        self.direct_command(Command::Visible(id, visible))
    }

    pub(super) fn direct_command(&mut self, command: Command) -> Result<(), UiError> {
        let mut handler = |_: &UiPointerEvent, _: &UiTree, _: &mut UiCommands| {};
        let mut work = Work::default();
        self.apply(command, &mut handler, &mut work)?;
        if self.theme.has_pending_styles() {
            self.settle_interaction_styles(&mut handler, &mut work)?;
        }
        Ok(())
    }

    pub(super) fn direct_interaction<T>(
        &mut self,
        action: impl FnOnce(&mut Self) -> Result<T, UiError>,
    ) -> Result<T, UiError> {
        if self.closed {
            return Err(UiError::Closed);
        }
        let mut work = Work::default();
        let mut observer = |_: &UiPointerEvent, _: &UiTree, _: &mut UiCommands| {};
        self.finish_input(&mut observer, &mut work)?;
        let result = action(self)?;
        self.finish_input(&mut observer, &mut work)?;
        Ok(result)
    }

    /// Applies pending reactive or application-sent commands at a safe boundary.
    pub fn flush_updates(
        &mut self,
        viewport: Size,
        mut observer: impl FnMut(&UiPointerEvent, &UiTree, &mut UiCommands),
    ) -> Result<UiDispatchResult, UiError> {
        if self.closed {
            return Err(UiError::Closed);
        }
        let mut work = Work::default();
        self.finish_input(&mut observer, &mut work)?;
        self.layout_for_input(viewport, &mut observer, &mut work)?;
        work.result.changed = self.paint_dirty;
        Ok(work.result)
    }
    /// Routes a platform event and executes its deferred mutation batches.
    ///
    /// Hover boundaries precede move/down/up without flushing their mutations.
    /// The final up releases capture before handler mutations execute. Reports
    /// are delivered per batch and also returned after dispatch. Callbacks can
    /// inspect retained state but modify it only through deferred commands.
    pub fn dispatch(
        &mut self,
        event: &WindowEvent,
        viewport: Size,
        handler: impl FnMut(&UiPointerEvent, &UiTree, &mut UiCommands),
    ) -> Result<UiDispatchResult, UiError> {
        self.dispatch_input(event, viewport, false, handler)
    }

    /// Processes an event while consuming any key transition at the input boundary.
    /// Use this for recording a shortcut. Repeats and release of a consumed press
    /// remain consumed after returning to ordinary dispatch.
    pub fn dispatch_consumed_key(
        &mut self,
        event: &WindowEvent,
        viewport: Size,
        handler: impl FnMut(&UiPointerEvent, &UiTree, &mut UiCommands),
    ) -> Result<UiDispatchResult, UiError> {
        self.dispatch_input(event, viewport, true, handler)
    }

    fn dispatch_input(
        &mut self,
        event: &WindowEvent,
        viewport: Size,
        key_consumed: bool,
        mut handler: impl FnMut(&UiPointerEvent, &UiTree, &mut UiCommands),
    ) -> Result<UiDispatchResult, UiError> {
        if self.closed {
            return Err(UiError::Closed);
        }
        let _span = tracing::info_span!("ui_dispatch").entered();
        let mut work = Work::default();
        self.collect_mailbox()?;
        self.layout_for_input(viewport, &mut handler, &mut work)?;
        if !matches!(event, WindowEvent::Destroyed) {
            self.finish_input(&mut handler, &mut work)?;
        }
        match event {
            WindowEvent::PointerMoved {
                position,
                timestamp,
            } => {
                self.set_pointer(*position, *timestamp);
                self.positional(PointerEventKind::Move, None, &mut handler, &mut work)?;
            }
            WindowEvent::PointerButton {
                button,
                pressed,
                position,
                timestamp,
            } => {
                self.set_pointer(*position, *timestamp);
                if *pressed && self.pointer.active {
                    if !self.pointer.pressed.contains(button) {
                        self.pointer.pressed.push(*button);
                    }
                } else if !pressed {
                    self.pointer.pressed.retain(|old| old != button);
                }
                let activation = self.positional(
                    if *pressed {
                        PointerEventKind::Down
                    } else {
                        PointerEventKind::Up
                    },
                    Some(*button),
                    &mut handler,
                    &mut work,
                )?;
                if !pressed && self.pointer.pressed.is_empty() && self.pointer.capture.is_some() {
                    let next = self.geometric_path()?;
                    self.check_hover_budget(&next, &work)?;
                    self.pointer.capture = None;
                    self.reconcile(next, &mut handler, &mut work)?;
                }
                if let Some(activation) = activation {
                    self.perform_action(
                        activation.target,
                        &activation.action,
                        ActionSource::Pointer,
                        Some(*timestamp),
                    )?;
                }
            }
            WindowEvent::PointerLeft { timestamp } => {
                self.pointer.timestamp = *timestamp;
                self.pointer.inside = false;
                self.pointer_press_inside(&[])?;
                if self.pointer.capture.is_none() {
                    self.pointer.active = false;
                    self.reconcile(Vec::new(), &mut handler, &mut work)?;
                }
            }
            WindowEvent::ActivationChanged { active, timestamp } => {
                self.update_window_input(Some(*active), None)?;
                if !active {
                    self.pointer.timestamp = *timestamp;
                    self.interrupt(&mut handler, &mut work)?;
                }
            }
            WindowEvent::VisibilityChanged { visible, timestamp } => {
                self.update_window_input(None, Some(*visible))?;
                if !visible {
                    self.pointer.timestamp = *timestamp;
                    self.interrupt(&mut handler, &mut work)?;
                }
            }
            WindowEvent::PointerInterrupted { timestamp } => {
                self.pointer.timestamp = *timestamp;
                self.take_arm(|source| matches!(source, ArmedInput::Pointer { .. }))?;
                self.interrupt(&mut handler, &mut work)?;
            }
            WindowEvent::Key { .. } => {
                work.result.key_handled = self.route_key(event, key_consumed)?;
            }
            WindowEvent::PreferencesChanged { preferences } => {
                self.set_system_preferences(*preferences)?;
            }
            WindowEvent::Destroyed => {
                self.closing = true;
                self.mailbox.close()?;
                self.update_window_input(Some(false), Some(false))?;
                self.interrupt(&mut handler, &mut work)?;
                self.transition_focus(None, None)?;
                self.cancel_queue(UiError::QueueClosed, &mut handler, &mut work)?;
                let order = self.postorder(self.root)?;
                self.paint_cache = None;
                self.editors.clear();
                for id in order {
                    if let Some(slot) = self.nodes.get_mut(id.slot) {
                        drop(slot.take());
                    }
                }
                self.clear_theme_consumers();
                self.identities
                    .try_borrow_mut()
                    .map_err(|_| UiError::ReservationBusy)?
                    .closed = true;
                self.engine.clear_cache();
                self.layout_changes = HashMap::new();
                self.closed = true;
            }
            _ => {}
        }
        if !self.closed {
            self.finish_input(&mut handler, &mut work)?;
        }
        work.result.changed = self.paint_dirty && !self.closed;
        Ok(work.result)
    }

    pub(super) fn settle_interaction_styles(
        &mut self,
        handler: &mut Handler<'_>,
        work: &mut Work,
    ) -> Result<(), UiError> {
        // An effective style can hide its own node and cause another round of
        // interaction updates. Bound the work if the styles cycle.
        for _ in 0..COMMAND_BUDGET {
            if !self.sync_interaction_styles(handler, work)? {
                return Ok(());
            }
        }
        Err(UiError::BudgetExceeded)
    }

    fn set_pointer(&mut self, position: Point, timestamp: Duration) {
        self.pointer.position = position;
        self.pointer.timestamp = timestamp;
        self.pointer.inside = position.x() >= 0.0
            && position.y() >= 0.0
            && position.x() < self.viewport.width()
            && position.y() < self.viewport.height();
        self.pointer.active = self.accepts_input();
    }
    fn geometric_path(&self) -> Result<Vec<NodeId>, UiError> {
        if self.pointer.inside
            && self.pointer.active
            && let Some(target) = self.hit_test(self.pointer.position)?
        {
            return self.path(target);
        }
        Ok(Vec::new())
    }
    fn effective_path(&self) -> Result<Vec<NodeId>, UiError> {
        self.pointer
            .capture
            .map_or_else(|| self.geometric_path(), |holder| self.path(holder))
    }
    fn positional(
        &mut self,
        kind: PointerEventKind,
        button: Option<PointerButton>,
        handler: &mut Handler<'_>,
        work: &mut Work,
    ) -> Result<Option<ArmedActivation>, UiError> {
        self.layout_for_input(self.viewport, handler, work)?;
        let path = self.effective_path()?;
        let target = path.last().copied();
        let mut route = path.clone();
        route.reverse();
        self.check_budget(work, self.hover_cost(&path) + route.len())?;
        self.reconcile(path, handler, work)?;
        let activation = self.update_pointer_activation(kind, button, &route)?;
        if let Some(target) = target {
            self.emit(kind, target, &route, button, handler, work)?;
        }
        Ok(activation)
    }
    fn update_pointer_activation(
        &mut self,
        kind: PointerEventKind,
        button: Option<PointerButton>,
        route: &[NodeId],
    ) -> Result<Option<ArmedActivation>, UiError> {
        if !self.accepts_input() {
            return Ok(None);
        }
        let primary = button == Some(PointerButton::Primary);
        let pointer_armed = self
            .interaction
            .armed
            .as_ref()
            .is_some_and(|press| matches!(press.input, ArmedInput::Pointer { .. }));
        let geometric;
        let inside_path = if self.pointer.capture.is_some() && pointer_armed {
            geometric = self.geometric_path()?;
            geometric.as_slice()
        } else {
            route
        };
        if kind == PointerEventKind::Down && primary {
            let mut focus = None;
            let mut armed = None;
            for &candidate in route {
                if focus.is_none() && self.focus_eligible(candidate)? {
                    focus = Some(candidate);
                }
                if armed.is_none()
                    && let Some(action) = self.activation_action(candidate)?
                    && self.interaction_available(candidate)?
                {
                    let inside = if self.pointer.capture.is_some() && !pointer_armed {
                        self.geometric_path()?.contains(&candidate)
                    } else {
                        inside_path.contains(&candidate)
                    };
                    armed = Some(ArmedActivation {
                        target: candidate,
                        action: action.to_owned(),
                        input: ArmedInput::Pointer { inside },
                    });
                }
                if focus.is_some() && armed.is_some() {
                    break;
                }
            }
            self.transition_focus(focus.or(self.focused_node()), Some(false))?;
            self.set_armed(armed)?;
        } else if kind == PointerEventKind::Up && primary {
            let press = self.take_arm(|source| matches!(source, ArmedInput::Pointer { .. }))?;
            return Ok(press.filter(|press| {
                inside_path.contains(&press.target) && route.contains(&press.target)
            }));
        } else if pointer_armed {
            self.pointer_press_inside(inside_path)?;
        }
        Ok(None)
    }
    fn hover_cost(&self, next: &[NodeId]) -> usize {
        self.pointer
            .hover
            .iter()
            .filter(|id| !next.contains(id))
            .count()
            + next
                .iter()
                .filter(|id| !self.pointer.hover.contains(id))
                .count()
    }
    fn check_budget(&self, work: &Work, count: usize) -> Result<(), UiError> {
        if count > EVENT_BUDGET.saturating_sub(work.result.events) {
            return Err(UiError::BudgetExceeded);
        }
        Ok(())
    }
    fn check_hover_budget(&self, next: &[NodeId], work: &Work) -> Result<(), UiError> {
        self.check_budget(work, self.hover_cost(next))
    }
    fn reconcile(
        &mut self,
        next: Vec<NodeId>,
        handler: &mut Handler<'_>,
        work: &mut Work,
    ) -> Result<(), UiError> {
        self.check_hover_budget(&next, work)?;
        let old = std::mem::replace(&mut self.pointer.hover, next.clone());
        let mut commands = UiCommands::new(self);
        if old != next {
            let _active = self.mailbox.activate(&commands)?;
            self.publish_interaction(
                old.iter()
                    .filter(|id| !next.contains(id))
                    .chain(next.iter().filter(|id| !old.contains(id)))
                    .copied(),
            )?;
        }
        for id in old.iter().rev().filter(|id| !next.contains(id)) {
            self.deliver(
                PointerEventKind::Leave,
                *id,
                *id,
                None,
                &mut commands,
                handler,
                work,
            )?;
        }
        for id in next.iter().filter(|id| !old.contains(id)) {
            self.deliver(
                PointerEventKind::Enter,
                *id,
                *id,
                None,
                &mut commands,
                handler,
                work,
            )?;
        }
        self.enqueue(commands)
    }
    fn emit(
        &mut self,
        kind: PointerEventKind,
        target: NodeId,
        route: &[NodeId],
        button: Option<PointerButton>,
        handler: &mut Handler<'_>,
        work: &mut Work,
    ) -> Result<(), UiError> {
        self.check_budget(work, route.len())?;
        let mut commands = UiCommands::new(self);
        for current in route {
            self.deliver(kind, target, *current, button, &mut commands, handler, work)?;
            if commands.stop {
                break;
            }
        }
        self.enqueue(commands)
    }
    #[allow(clippy::too_many_arguments)]
    fn deliver(
        &mut self,
        kind: PointerEventKind,
        target: NodeId,
        current_target: NodeId,
        button: Option<PointerButton>,
        commands: &mut UiCommands,
        handler: &mut Handler<'_>,
        work: &mut Work,
    ) -> Result<(), UiError> {
        let origin = self.bounds(current_target)?.origin();
        let position = self.pointer.position;
        let event = UiPointerEvent {
            kind,
            target,
            current_target,
            position,
            local_position: Point::new(position.x() - origin.x(), position.y() - origin.y()),
            buttons: self.pointer.buttons(),
            button,
            timestamp: self.pointer.timestamp,
        };
        work.result.events += 1;
        let _active = self.mailbox.activate(commands)?;
        if let Some(callback) = self.node(current_target)?.handler.as_ref().cloned() {
            let mut callback = callback
                .try_borrow_mut()
                .map_err(|_| UiError::HandlerBusy)?;
            let owner = self
                .node(current_target)?
                .scopes
                .first()
                .map(|scope| scope.owner());
            let mut invoke = || {
                let _span =
                    tracing::trace_span!("ui_pointer_handler", ?kind, ?target, ?current_target)
                        .entered();
                self.diagnostics.dispatched_handlers =
                    self.diagnostics.dispatched_handlers.saturating_add(1);
                callback(&event, self, commands);
            };
            if let Some(owner) = owner {
                match owner.run(|| {
                    invoke();
                    Ok(())
                }) {
                    Ok(()) | Err(ReactiveError::Disposed) => {}
                    Err(error) => return Err(UiError::Reactive(error.to_string())),
                }
            } else {
                invoke();
            }
        }
        handler(&event, self, commands);
        Ok(())
    }
    pub(super) fn enqueue(&mut self, commands: UiCommands) -> Result<(), UiError> {
        if commands.buffer.failed.get() {
            return Err(UiError::CommandBufferBusy);
        }
        self.collect_mailbox()?;
        let submitted = std::mem::take(
            &mut *commands
                .buffer
                .commands
                .try_borrow_mut()
                .map_err(|_| UiError::CommandBufferBusy)?,
        );
        self.push_batch(submitted, commands.notify)
    }
    fn collect_mailbox(&mut self) -> Result<(), UiError> {
        let pending = self.mailbox.take_pending()?;
        self.push_batch(pending, true)
    }
    fn push_batch(&mut self, commands: Vec<Command>, notify: bool) -> Result<(), UiError> {
        if commands.is_empty() {
            return Ok(());
        }
        let next = self
            .next_batch
            .checked_add(1)
            .ok_or(UiError::IdentityExhausted)?;
        self.queue.push_back(Batch {
            number: self.next_batch,
            commands,
            notify,
        });
        self.next_batch = next;
        Ok(())
    }
    fn interrupt(&mut self, handler: &mut Handler<'_>, work: &mut Work) -> Result<(), UiError> {
        let route = if let Some(holder) = self.pointer.capture {
            let mut path = self.path(holder)?;
            path.reverse();
            path
        } else {
            Vec::new()
        };
        self.check_budget(work, route.len() + self.pointer.hover.len())?;
        if let Some(holder) = self.pointer.capture {
            self.emit(
                PointerEventKind::Cancel,
                holder,
                &route,
                None,
                handler,
                work,
            )?;
        }
        self.pointer.capture = None;
        self.pointer.pressed.clear();
        self.pointer.active = false;
        self.reconcile(Vec::new(), handler, work)
    }
    fn cleanup(
        &mut self,
        subtree: NodeId,
        removed: bool,
        handler: &mut Handler<'_>,
        work: &mut Work,
    ) -> Result<(), UiError> {
        if self.pointer.hover.is_empty() && self.pointer.capture.is_none() {
            return Ok(());
        }
        let old = self.pointer.hover.clone();
        let split = old
            .iter()
            .position(|id| self.descendant(*id, subtree).unwrap_or(false));
        let mut next = split.map_or_else(
            || old.clone(),
            |index| old.iter().take(index).copied().collect(),
        );
        let cancel = self
            .pointer
            .capture
            .filter(|holder| self.descendant(*holder, subtree).unwrap_or(false));
        let route = if let Some(holder) = cancel {
            let mut route = self.path(holder)?;
            route.reverse();
            route
        } else {
            Vec::new()
        };
        if cancel.is_some() && !self.pointer.inside {
            next.clear();
        }
        self.check_budget(work, route.len() + self.hover_cost(&next))?;
        if removed {
            for id in &old {
                if self.descendant(*id, subtree)? {
                    work.deleted_options
                        .insert(*id, self.node(*id)?.style.stationary_hover_updates);
                }
            }
        }
        if let Some(holder) = cancel {
            self.emit(
                PointerEventKind::Cancel,
                holder,
                &route,
                None,
                handler,
                work,
            )?;
            self.pointer.capture = None;
        }
        self.reconcile(next, handler, work)
    }
    pub(super) fn prepare_display_suppression(
        &mut self,
        id: NodeId,
        display: Display,
        handler: &mut Handler<'_>,
        work: &mut Work,
    ) -> Result<(), UiError> {
        if display != Display::None || self.node(id)?.style.layout.display == Display::None {
            return Ok(());
        }
        self.cleanup(id, false, handler, work)?;
        self.lose_interaction(FocusScope::Subtree(id))
    }
    fn finish_input(&mut self, handler: &mut Handler<'_>, work: &mut Work) -> Result<(), UiError> {
        self.collect_mailbox()?;
        if self.theme.has_pending_styles() {
            self.settle_interaction_styles(handler, work)?;
            self.collect_mailbox()?;
        }
        while let Some(batch) = self.queue.pop_front() {
            let old_hover = self.pointer.hover.clone();
            let mut report = UiBatchReport {
                batch: batch.number,
                outcomes: Vec::new(),
                canceled_bindings: 0,
            };
            let mut exceeded = false;
            for (sequence, command) in batch.commands.into_iter().enumerate() {
                if command.canceled_binding()? {
                    report.canceled_bindings += 1;
                    continue;
                }
                let id = command.node();
                let mut result = if exceeded
                    || work.commands >= COMMAND_BUDGET
                    || work.result.events >= EVENT_BUDGET
                {
                    exceeded = true;
                    Err(UiError::BudgetExceeded)
                } else {
                    work.commands += 1;
                    self.apply(command, handler, work)
                };
                self.collect_mailbox()?;
                if result != Err(UiError::BudgetExceeded) && self.theme.has_pending_styles() {
                    // Later commands validate against the effective state of
                    // earlier commands, including any visibility changes.
                    result = result.and(self.settle_interaction_styles(handler, work));
                    self.collect_mailbox()?;
                }
                if result == Err(UiError::BudgetExceeded) {
                    exceeded = true;
                }
                report.outcomes.push(UiCommandOutcome {
                    sequence,
                    node: Some(id),
                    result,
                });
            }
            self.publish_report(report, batch.notify, !exceeded && !self.closing, work)?;
            if exceeded {
                self.cancel_queue(UiError::BudgetExceeded, handler, work)?;
                break;
            }
            let result = self.stationary(&old_hover, handler, work).and_then(|()| {
                if self.theme.has_pending_styles() {
                    self.collect_mailbox()?;
                    self.settle_interaction_styles(handler, work)?;
                    self.collect_mailbox()?;
                }
                Ok(())
            });
            if let Err(error) = result {
                if error == UiError::BudgetExceeded {
                    self.cancel_queue(error, handler, work)?;
                    break;
                }
                return Err(error);
            }
            work.deleted_options.clear();
        }
        Ok(())
    }
    fn stationary(
        &mut self,
        old: &[NodeId],
        handler: &mut Handler<'_>,
        work: &mut Work,
    ) -> Result<(), UiError> {
        if self.pointer.capture.is_some() || !self.pointer.active || !self.pointer.inside {
            return Ok(());
        }
        let some_enabled = self
            .nodes
            .iter()
            .filter_map(Option::as_ref)
            .any(|node| node.style.stationary_hover_updates)
            || work.deleted_options.values().any(|enabled| *enabled);
        if !some_enabled {
            return Ok(());
        }
        self.layout_for_input(self.viewport, handler, work)?;
        let next = self.geometric_path()?;
        let qualifies = old.iter().chain(next.iter()).any(|id| {
            self.node(*id).map_or_else(
                |_| work.deleted_options.get(id).copied().unwrap_or(false),
                |node| node.style.stationary_hover_updates,
            )
        });
        if qualifies {
            self.pointer_press_inside(&next)?;
            self.reconcile(next, handler, work)?;
        }
        Ok(())
    }
    pub(super) fn apply(
        &mut self,
        command: Command,
        handler: &mut Handler<'_>,
        work: &mut Work,
    ) -> Result<(), UiError> {
        match command {
            Command::Scoped(_, command) => self.apply(*command, handler, work),
            Command::CanceledBinding(_) => Err(UiError::QueueClosed),
            Command::Create {
                reservation,
                parent,
                index,
                style,
            } => {
                let index = index.unwrap_or(self.node(parent)?.children.len());
                self.insert_reserved(reservation.id, parent, index, style)
            }
            Command::Style(id, style) => {
                self.set_local_style(id, style, handler, work)?;
                Ok(())
            }
            Command::Content(id, content) => self.set_content(id, content),
            Command::Text(id, text) => self.set_rich_text(id, text),
            Command::Handler(id, callback) => {
                self.node_mut(id)?.handler = Some(Rc::new(RefCell::new(callback)));
                Ok(())
            }
            Command::Mutation(id, mutation) => {
                self.node(id)?;
                mutation(self, handler, work)
            }
            Command::Rejected(_, reason) => Err(reason),
            Command::Visible(id, visible) => {
                self.node(id)?;
                if self.node(id)?.hidden == visible {
                    if !visible {
                        self.cleanup(id, false, handler, work)?;
                        self.lose_interaction(FocusScope::Subtree(id))?;
                    }
                    self.node_mut(id)?.hidden = !visible;
                    self.dirty_layout(id, true, false)?;
                }
                Ok(())
            }
            Command::Remove(id) => {
                self.validate_remove(id)?;
                self.cleanup(id, true, handler, work)?;
                self.remove_raw(id)
            }
            Command::Clear(id) => {
                let children = self.node(id)?.children.clone();
                let capture_cost = self
                    .pointer
                    .capture
                    .map_or(Ok(0), |holder| self.path(holder).map(|path| path.len()))?;
                self.check_budget(work, self.pointer.hover.len() + capture_cost)?;
                self.lose_interaction(FocusScope::Descendants(id))?;
                for child in children {
                    self.cleanup(child, true, handler, work)?;
                    self.remove_raw(child)?;
                }
                Ok(())
            }
            Command::Reparent(id, parent, index) => {
                let (old_parent, old_index) = self.validate_reparent(id, parent, index)?;
                if old_parent == parent && old_index == index {
                    return Ok(());
                }
                let parent_suppressed = self.suppressed(parent)?;
                let parent_disabled = self.effectively_disabled(parent)?;
                let inherited_disabled_changed = parent != old_parent
                    && parent_disabled != self.effectively_disabled(old_parent)?;
                let captured = self
                    .pointer
                    .capture
                    .is_some_and(|holder| self.descendant(holder, id).unwrap_or(false));
                if captured
                    && !parent_suppressed
                    && let Some(holder) = self.pointer.capture
                {
                    let old_path = self.path(holder)?;
                    let suffix = old_path
                        .iter()
                        .position(|ancestor| *ancestor == id)
                        .ok_or(UiError::StaleNode)?;
                    let mut next = self.path(parent)?;
                    next.extend(old_path.into_iter().skip(suffix));
                    self.check_hover_budget(&next, work)?;
                }
                if parent_suppressed {
                    self.cleanup(id, false, handler, work)?;
                }
                if parent_suppressed || parent_disabled {
                    self.lose_interaction(FocusScope::Subtree(id))?;
                } else if old_parent != parent
                    && let Some(focused) = self.focused_node()
                    && self.descendant(focused, id)?
                {
                    self.interaction.keyboard.clear_sequence();
                }
                self.reparent_raw(id, parent, index, old_parent, old_index)?;
                if inherited_disabled_changed {
                    self.publish_subtree_interaction(id)?;
                }
                if captured && self.pointer.capture.is_some() {
                    self.layout_for_input(self.viewport, handler, work)?;
                    self.reconcile(self.effective_path()?, handler, work)?;
                }
                Ok(())
            }
            Command::Capture(id) => {
                self.node(id)?;
                if self.pointer.pressed.is_empty() || !self.pointer.active || self.suppressed(id)? {
                    return Err(UiError::CaptureUnavailable);
                }
                if let Some(holder) = self.pointer.capture {
                    return if holder == id {
                        Ok(())
                    } else {
                        Err(UiError::CaptureOwned)
                    };
                }
                self.layout_for_input(self.viewport, handler, work)?;
                let next = self.path(id)?;
                self.check_hover_budget(&next, work)?;
                if let Some(press) = &self.interaction.armed
                    && matches!(press.input, ArmedInput::Pointer { .. })
                    && !next.contains(&press.target)
                {
                    self.take_arm(|source| matches!(source, ArmedInput::Pointer { .. }))?;
                }
                self.pointer.capture = Some(id);
                self.reconcile(next, handler, work)
            }
            Command::Release(id) => {
                self.node(id)?;
                if self.pointer.capture.is_none() {
                    return Ok(());
                }
                if self.pointer.capture != Some(id) {
                    return Err(UiError::CaptureOwned);
                }
                self.layout_for_input(self.viewport, handler, work)?;
                let next = self.geometric_path()?;
                self.check_hover_budget(&next, work)?;
                self.pointer.capture = None;
                self.reconcile(next, handler, work)
            }
            Command::Focus(id) => self.request_focus(id),
            Command::ClearFocus(_) => self.transition_focus(None, None),
            Command::FocusPolicy(id, policy) => self.change_focus_policy(id, policy),
            Command::FocusReturn(id, target) => self.set_focus_return_target(id, target),
            Command::Disabled(id, disabled) => self.change_disabled(id, disabled),
            Command::Keymap(id, keymap) => self.set_keymap(id, keymap),
            Command::InvokeAction(id, action) => {
                if self.perform_action(id, &action, ActionSource::Programmatic, None)? {
                    Ok(())
                } else {
                    Err(UiError::ActionUnavailable(action))
                }
            }
        }
    }
    fn publish_report(
        &mut self,
        report: UiBatchReport,
        notify: bool,
        accepting: bool,
        work: &mut Work,
    ) -> Result<(), UiError> {
        if notify && let Some(callback) = self.report_handler.as_ref().cloned() {
            let mut commands = UiCommands::new(self);
            commands.buffer = super::mailbox::CommandBuffer::new(accepting);
            // Rejections generated by terminal result callbacks are recorded once
            // without recursively notifying the same callback again.
            commands.notify = accepting;
            {
                let _active = self.mailbox.activate(&commands)?;
                let mut callback = callback
                    .try_borrow_mut()
                    .map_err(|_| UiError::HandlerBusy)?;
                callback(&report, self, &mut commands);
            }
            self.enqueue(commands)?;
        } else if self.report_handler.is_none() {
            for outcome in &report.outcomes {
                if let Err(error) = &outcome.result {
                    tracing::warn!(batch=report.batch,sequence=outcome.sequence,node=?outcome.node,%error,"UI command rejected");
                }
            }
        }
        work.result.reports.push(report);
        Ok(())
    }
    fn cancel_queue(
        &mut self,
        reason: UiError,
        _handler: &mut Handler<'_>,
        work: &mut Work,
    ) -> Result<(), UiError> {
        self.collect_mailbox()?;
        while let Some(batch) = self.queue.pop_front() {
            let mut outcomes = Vec::new();
            let mut canceled_bindings = 0;
            for (sequence, command) in batch.commands.into_iter().enumerate() {
                if command.canceled_binding()? {
                    canceled_bindings += 1;
                    continue;
                }
                outcomes.push(UiCommandOutcome {
                    sequence,
                    node: Some(command.node()),
                    result: Err(reason.clone()),
                });
            }
            self.publish_report(
                UiBatchReport {
                    batch: batch.number,
                    outcomes,
                    canceled_bindings,
                },
                batch.notify,
                false,
                work,
            )?;
        }
        Ok(())
    }
}
