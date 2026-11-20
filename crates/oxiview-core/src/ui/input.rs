use super::{NodeId, Reservation, UiError, UiTree};
use oxiview_base::{Point, Size};
use oxiview_platform::{PointerButton, WindowEvent};
use oxiview_reactive::{Owner, ReactiveError};
use oxiview_scene::Primitive;
use oxiview_style::{Display, Style};
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
    Text(NodeId, String),
    Handler(NodeId, Box<NodeHandler>),
    Mutation(NodeId, Box<Mutation>),
    Rejected(NodeId, UiError),
    Visible(NodeId, bool),
    Remove(NodeId),
    Clear(NodeId),
    Reparent(NodeId, NodeId, usize),
    Capture(NodeId),
    Release(NodeId),
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
            | Self::Release(id) => *id,
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
    identities: Rc<RefCell<super::Identities>>,
    pub(super) buffer: Rc<super::mailbox::CommandBuffer>,
    notify: bool,
    stop: bool,
}
impl UiCommands {
    fn new(tree: &UiTree) -> Self {
        Self {
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
        self.push(Command::Text(id, text.into()));
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
    /// Stops remaining ancestors of the current event without discarding commands.
    pub fn stop_propagation(&mut self) {
        self.stop = true;
    }
    fn push(&mut self, command: Command) {
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
        self.collect_mailbox()?;
        self.drain(&mut observer, &mut work)?;
        self.layout(viewport)?;
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
        mut handler: impl FnMut(&UiPointerEvent, &UiTree, &mut UiCommands),
    ) -> Result<UiDispatchResult, UiError> {
        if self.closed {
            return Err(UiError::Closed);
        }
        let _span = tracing::info_span!("ui_dispatch").entered();
        let mut work = Work::default();
        self.collect_mailbox()?;
        self.layout(viewport)?;
        if !matches!(event, WindowEvent::Destroyed) {
            self.drain(&mut handler, &mut work)?;
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
                if *pressed {
                    if !self.pointer.pressed.contains(button) {
                        self.pointer.pressed.push(*button);
                    }
                } else {
                    self.pointer.pressed.retain(|old| old != button);
                }
                self.positional(
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
            }
            WindowEvent::PointerLeft { timestamp } => {
                self.pointer.timestamp = *timestamp;
                self.pointer.inside = false;
                if self.pointer.capture.is_none() {
                    self.pointer.active = false;
                    self.reconcile(Vec::new(), &mut handler, &mut work)?;
                }
            }
            WindowEvent::ActivationChanged {
                active: false,
                timestamp,
            }
            | WindowEvent::VisibilityChanged {
                visible: false,
                timestamp,
            }
            | WindowEvent::PointerInterrupted { timestamp } => {
                self.pointer.timestamp = *timestamp;
                self.interrupt(&mut handler, &mut work)?;
            }
            WindowEvent::Destroyed => {
                self.closing = true;
                self.mailbox.close()?;
                self.interrupt(&mut handler, &mut work)?;
                self.cancel_queue(UiError::QueueClosed, &mut handler, &mut work)?;
                let order = self.postorder(self.root)?;
                self.paint_cache = None;
                for id in order {
                    if let Some(slot) = self.nodes.get_mut(id.slot) {
                        drop(slot.take());
                    }
                }
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
            self.drain(&mut handler, &mut work)?;
        }
        work.result.changed = self.paint_dirty && !self.closed;
        Ok(work.result)
    }

    fn set_pointer(&mut self, position: Point, timestamp: Duration) {
        self.pointer.position = position;
        self.pointer.timestamp = timestamp;
        self.pointer.inside = position.x() >= 0.0
            && position.y() >= 0.0
            && position.x() < self.viewport.width()
            && position.y() < self.viewport.height();
        self.pointer.active = true;
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
    ) -> Result<(), UiError> {
        self.layout(self.viewport)?;
        let path = self.effective_path()?;
        let target = path.last().copied();
        let mut route = path.clone();
        route.reverse();
        self.check_budget(work, self.hover_cost(&path) + route.len())?;
        self.reconcile(path, handler, work)?;
        if let Some(target) = target {
            self.emit(kind, target, &route, button, handler, work)?;
        }
        Ok(())
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
    fn enqueue(&mut self, commands: UiCommands) -> Result<(), UiError> {
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
    fn drain(&mut self, handler: &mut Handler<'_>, work: &mut Work) -> Result<(), UiError> {
        self.collect_mailbox()?;
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
                let result = if exceeded
                    || work.commands >= COMMAND_BUDGET
                    || work.result.events >= EVENT_BUDGET
                {
                    exceeded = true;
                    Err(UiError::BudgetExceeded)
                } else {
                    work.commands += 1;
                    self.apply(command, handler, work)
                };
                if result == Err(UiError::BudgetExceeded) {
                    exceeded = true;
                }
                self.collect_mailbox()?;
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
            if let Err(error) = self.stationary(&old_hover, handler, work) {
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
        self.layout(self.viewport)?;
        let next = self.geometric_path()?;
        let qualifies = old.iter().chain(next.iter()).any(|id| {
            self.node(*id).map_or_else(
                |_| work.deleted_options.get(id).copied().unwrap_or(false),
                |node| node.style.stationary_hover_updates,
            )
        });
        if qualifies {
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
                self.validate_style(&style)?;
                let suppresses = self.node(id)?.style.layout.display != Display::None
                    && style.layout.display == Display::None;
                if suppresses {
                    self.cleanup(id, false, handler, work)?;
                }
                self.set_style(id, style)?;
                Ok(())
            }
            Command::Content(id, content) => self.set_content(id, content),
            Command::Text(id, text) => self.set_text(id, text),
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
                for child in children {
                    self.cleanup(child, true, handler, work)?;
                    self.remove_raw(child)?;
                }
                Ok(())
            }
            Command::Reparent(id, parent, index) => {
                self.validate_reparent(id, parent, index)?;
                let captured = self
                    .pointer
                    .capture
                    .is_some_and(|holder| self.descendant(holder, id).unwrap_or(false));
                if captured
                    && !self.suppressed(parent)?
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
                if self.suppressed(parent)? {
                    self.cleanup(id, false, handler, work)?;
                }
                self.reparent(id, parent, index)?;
                if captured && self.pointer.capture.is_some() {
                    self.layout(self.viewport)?;
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
                self.layout(self.viewport)?;
                let next = self.path(id)?;
                self.check_hover_budget(&next, work)?;
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
                self.layout(self.viewport)?;
                let next = self.geometric_path()?;
                self.check_hover_budget(&next, work)?;
                self.pointer.capture = None;
                self.reconcile(next, handler, work)
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
