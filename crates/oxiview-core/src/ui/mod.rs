//! Retained UI ownership, layout projection, painting, and pointer routing.

use oxiview_base::{Point, Rect, Size};
use oxiview_layout::{
    AvailableSpace, Dimensions, IncrementalTaffyLayout, LayoutBox, LayoutChange, LayoutEngine,
    LayoutInput, LayoutStats, LayoutTree,
};
use oxiview_scene::Primitive;
use oxiview_style::{Display, Style};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

mod diagnostics;
mod input;
mod mailbox;
mod paint;
mod text;
mod view;
pub use diagnostics::UiDiagnostics;
pub use input::{
    PointerEventKind, UiBatchReport, UiCommandOutcome, UiCommands, UiDispatchResult, UiPointerEvent,
};
pub use mailbox::UiCommandSender;
pub use text::text;
pub use view::{View, component, for_each, keyed_for, node, show};

const MAX_DEPTH: usize = oxiview_layout::MAX_LAYOUT_DEPTH;
static NEXT_TREE: AtomicU64 = AtomicU64::new(1);

fn allocate_tree_owner(counter: &AtomicU64) -> Result<u64, UiError> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| UiError::IdentityExhausted)
}

/// Opaque identity of a UI node, distinct from a painting Scene identity.
///
/// ```compile_fail
/// use oxiview_core::NodeId;
///
/// let _ = NodeId { owner: 1, slot: 0, generation: 1 };
/// ```
///
/// ```compile_fail
/// use oxiview_core::{UiError, UiTree};
///
/// fn add_retained_child(tree: &mut UiTree, parent: oxiview_scene::NodeId) -> Result<(), UiError> {
///     tree.append(parent, oxiview_style::Style::default()).map(|_| ())
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId {
    owner: u64,
    slot: usize,
    generation: u64,
}

/// Rejected UI operations; no platform or layout-engine types escape this boundary.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UiError {
    /// The identity belongs to another UI tree.
    #[error("node belongs to another UI tree")]
    ForeignNode,
    /// The identity was removed, canceled, or never allocated.
    #[error("node identity is no longer valid")]
    StaleNode,
    /// A creation command reserved the identity but has not created its node yet.
    #[error("node has not been created yet")]
    NotYetCreated,
    /// This tree has been closed.
    #[error("UI tree is closed")]
    Closed,
    /// A unique owner, slot generation, or batch number could not be allocated.
    #[error("UI identities exhausted")]
    IdentityExhausted,
    /// The root cannot be removed or reparented.
    #[error("the UI root cannot be removed or reparented")]
    RootOperation,
    /// Child insertion indices are evaluated after conceptual removal for moves.
    #[error("child index {index} exceeds insertion length {len}")]
    ChildIndex {
        /// Requested insertion index.
        index: usize,
        /// Inclusive maximum insertion index.
        len: usize,
    },
    /// A reparenting operation would introduce a cycle.
    #[error("UI edit would create a cycle")]
    Cycle,
    /// Retained tree depth and component construction nesting are bounded.
    #[error("UI tree exceeds its supported depth")]
    DepthLimit,
    /// The viewport must have finite, nonnegative dimensions.
    #[error("invalid UI viewport or scale")]
    InvalidViewport,
    /// Style validation rejected an edit.
    #[error("invalid style: {0}")]
    Style(String),
    /// The owned layout port rejected preparation.
    #[error("layout failed: {0}")]
    Layout(String),
    /// The owned scene port rejected painting content.
    #[error("scene preparation failed: {0}")]
    Scene(#[from] oxiview_scene::Error),
    /// Capture requires a live, unsuppressed node and at least one pressed button.
    #[error("pointer capture is unavailable for this node")]
    CaptureUnavailable,
    /// Existing capture must be explicitly released before another node acquires it.
    #[error("pointer capture is already owned by another node")]
    CaptureOwned,
    /// A bounded input processing budget was exhausted.
    #[error("UI command or event processing budget exceeded")]
    BudgetExceeded,
    /// A queued command was canceled when its tree closed.
    #[error("UI command canceled because its queue closed")]
    QueueClosed,
    /// An internal reservation is already borrowed by another operation.
    #[error("UI identity reservation is already in use")]
    ReservationBusy,
    /// A deferred command buffer is already borrowed.
    #[error("UI command buffer is already in use")]
    CommandBufferBusy,
    /// A node callback attempted reentrant use of the same mutable callback.
    #[error("UI handler is already in use")]
    HandlerBusy,
    /// A reactive binding or scope operation failed.
    #[error("reactive UI operation failed: {0}")]
    Reactive(String),
    /// A keyed list contains duplicate keys.
    #[error("keyed children require unique keys")]
    DuplicateKey,
    /// Component construction returned an invalid view.
    #[error("view construction failed: {0}")]
    View(String),
    /// Text shaping, font selection, or glyph generation failed.
    #[error("text preparation failed: {0}")]
    Text(String),
    /// Intrinsic measurement cache versions cannot advance without wrapping.
    #[error("UI measurement revision exhausted")]
    RevisionExhausted,
}

/// Optional retained semantics; platform accessibility and action dispatch are later milestones.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeSemantics {
    /// Application-defined semantic role.
    pub role: String,
    /// Human-readable label.
    pub label: String,
    /// Current semantic value.
    pub value: String,
    /// Identifiers of supported actions.
    pub actions: Vec<String>,
}

/// Intrinsic leaf measurement using only owned layout-port values.
pub trait NodeMeasure {
    /// Returns intrinsic content dimensions under the supplied constraints.
    fn measure(
        &self,
        known: Dimensions<Option<f64>>,
        available: Dimensions<AvailableSpace>,
    ) -> Dimensions<f64>;
}
impl<F> NodeMeasure for F
where
    F: Fn(Dimensions<Option<f64>>, Dimensions<AvailableSpace>) -> Dimensions<f64>,
{
    fn measure(
        &self,
        known: Dimensions<Option<f64>>,
        available: Dimensions<AvailableSpace>,
    ) -> Dimensions<f64> {
        self(known, available)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IdentityState {
    Reserved,
    Live,
    Free,
    Retired,
}
struct IdentitySlot {
    generation: u64,
    state: IdentityState,
}
struct Identities {
    owner: u64,
    slots: Vec<IdentitySlot>,
    free: Vec<usize>,
    closed: bool,
}

impl Identities {
    fn reserve(&mut self) -> Result<NodeId, UiError> {
        if self.closed {
            return Err(UiError::Closed);
        }
        let slot = if let Some(slot) = self.free.pop() {
            slot
        } else {
            let slot = self.slots.len();
            self.slots.push(IdentitySlot {
                generation: 1,
                state: IdentityState::Free,
            });
            slot
        };
        let entry = self.slots.get_mut(slot).ok_or(UiError::StaleNode)?;
        entry.state = IdentityState::Reserved;
        Ok(NodeId {
            owner: self.owner,
            slot,
            generation: entry.generation,
        })
    }
    fn state(&self, id: NodeId) -> Result<IdentityState, UiError> {
        if id.owner != self.owner {
            return Err(UiError::ForeignNode);
        }
        if self.closed {
            return Err(UiError::Closed);
        }
        let entry = self.slots.get(id.slot).ok_or(UiError::StaleNode)?;
        if entry.generation != id.generation
            || matches!(entry.state, IdentityState::Free | IdentityState::Retired)
        {
            return Err(UiError::StaleNode);
        }
        Ok(entry.state)
    }
    fn commit(&mut self, id: NodeId) -> Result<(), UiError> {
        if self.state(id)? != IdentityState::Reserved {
            return Err(UiError::StaleNode);
        }
        self.slots.get_mut(id.slot).ok_or(UiError::StaleNode)?.state = IdentityState::Live;
        Ok(())
    }
    fn release(&mut self, id: NodeId, reserved_only: bool) {
        if id.owner != self.owner {
            return;
        }
        let Some(entry) = self.slots.get_mut(id.slot) else {
            return;
        };
        if entry.generation != id.generation
            || (reserved_only && entry.state != IdentityState::Reserved)
            || matches!(entry.state, IdentityState::Free | IdentityState::Retired)
        {
            return;
        }
        if let Some(generation) = entry.generation.checked_add(1) {
            entry.generation = generation;
            entry.state = IdentityState::Free;
            self.free.push(id.slot);
        } else {
            entry.state = IdentityState::Retired;
        }
    }
}

struct Reservation {
    id: NodeId,
    identities: Rc<RefCell<Identities>>,
}
impl Reservation {
    fn new(identities: &Rc<RefCell<Identities>>) -> Result<Self, UiError> {
        let id = identities
            .try_borrow_mut()
            .map_err(|_| UiError::ReservationBusy)?
            .reserve()?;
        Ok(Self {
            id,
            identities: Rc::clone(identities),
        })
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        match self.identities.try_borrow_mut() {
            Ok(mut identities) => identities.release(self.id, true),
            Err(_) => {
                tracing::error!(?self.id, "reservation cleanup encountered an active identity borrow")
            }
        }
    }
}

struct Node {
    id: NodeId,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    style: Style,
    layout: LayoutBox,
    intrinsic: Size,
    content_revision: u64,
    measure: Option<Box<dyn NodeMeasure>>,
    content: Vec<Primitive>,
    semantics: Option<NodeSemantics>,
    handler: Option<Rc<RefCell<Box<input::NodeHandler>>>>,
    scopes: Vec<oxiview_reactive::Scope>,
    hidden: bool,
    text: Option<Rc<text::TextNode>>,
    painting: Option<Box<paint::NodePainting>>,
}

/// One window's retained UI tree. All methods run on its window thread.
///
/// Direct edits support initial construction. During interaction, use the
/// command buffer supplied by [`UiTree::dispatch`] so lifecycle notifications
/// and mutation ordering remain observable by the same callback.
pub struct UiTree {
    owner: u64,
    root: NodeId,
    identities: Rc<RefCell<Identities>>,
    nodes: Vec<Option<Node>>,
    engine: IncrementalTaffyLayout<NodeId>,
    viewport: Size,
    layout_dirty: bool,
    layout_changes: HashMap<NodeId, LayoutChange<NodeId>>,
    paint_dirty: bool,
    paint_cache: Option<paint::PaintCache>,
    diagnostics: UiDiagnostics,
    pointer: input::PointerState,
    queue: VecDeque<input::Batch>,
    mailbox: UiCommandSender,
    report_handler: Option<Rc<RefCell<Box<input::ReportHandler>>>>,
    next_batch: u64,
    closing: bool,
    closed: bool,
}

impl UiTree {
    /// Creates a tree and its persistent root.
    pub fn new() -> Result<Self, UiError> {
        let owner = allocate_tree_owner(&NEXT_TREE)?;
        let identities = Rc::new(RefCell::new(Identities {
            owner,
            slots: Vec::new(),
            free: Vec::new(),
            closed: false,
        }));
        let reservation = Reservation::new(&identities)?;
        let root = reservation.id;
        let scope = oxiview_reactive::Scope::new().map_err(reactive_error)?;
        identities
            .try_borrow_mut()
            .map_err(|_| UiError::ReservationBusy)?
            .commit(root)?;
        let node = Node {
            id: root,
            parent: None,
            children: Vec::new(),
            style: Style::default(),
            layout: LayoutBox::default(),
            intrinsic: Size::default(),
            content_revision: 1,
            measure: None,
            content: Vec::new(),
            semantics: None,
            handler: None,
            scopes: vec![scope],
            hidden: false,
            text: None,
            painting: None,
        };
        let mailbox = UiCommandSender::new(Rc::clone(&identities));
        Ok(Self {
            owner,
            root,
            identities,
            nodes: vec![Some(node)],
            engine: IncrementalTaffyLayout::default(),
            viewport: Size::default(),
            layout_dirty: true,
            layout_changes: HashMap::new(),
            paint_dirty: true,
            paint_cache: None,
            diagnostics: UiDiagnostics::default(),
            pointer: input::PointerState::default(),
            queue: VecDeque::new(),
            mailbox,
            report_handler: None,
            next_batch: 1,
            closing: false,
            closed: false,
        })
    }
    /// Returns the stable root identity.
    pub fn root(&self) -> NodeId {
        self.root
    }
    /// Copies cumulative committed-work counters without causing any work.
    pub fn diagnostics(&self) -> UiDiagnostics {
        self.diagnostics
    }
    /// Whether this ID currently identifies a live node in this tree.
    pub fn contains(&self, id: NodeId) -> bool {
        self.node(id).is_ok()
    }
    /// Returns the number of live nodes, including the root.
    pub fn len(&self) -> usize {
        self.nodes.iter().filter(|node| node.is_some()).count()
    }
    /// Whether the tree has no live nodes, which occurs only after closure.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Returns ordered child identities.
    pub fn children(&self, id: NodeId) -> Result<&[NodeId], UiError> {
        Ok(&self.node(id)?.children)
    }
    /// Returns the parent, or None for the root.
    pub fn parent(&self, id: NodeId) -> Result<Option<NodeId>, UiError> {
        Ok(self.node(id)?.parent)
    }
    /// Reads the retained style; reserved identities report NotYetCreated.
    pub fn style(&self, id: NodeId) -> Result<&Style, UiError> {
        Ok(&self.node(id)?.style)
    }
    /// Appends a newly created node under a live parent.
    pub fn append(&mut self, parent: NodeId, style: Style) -> Result<NodeId, UiError> {
        let index = self.node(parent)?.children.len();
        self.insert(parent, index, style)
    }
    /// Creates and mounts a node at a zero-based insertion position.
    pub fn insert(
        &mut self,
        parent: NodeId,
        index: usize,
        style: Style,
    ) -> Result<NodeId, UiError> {
        let reservation = Reservation::new(&self.identities)?;
        let id = reservation.id;
        self.insert_reserved(id, parent, index, style)?;
        Ok(id)
    }
    fn insert_reserved(
        &mut self,
        id: NodeId,
        parent: NodeId,
        index: usize,
        style: Style,
    ) -> Result<(), UiError> {
        self.validate_insertion(parent, index, &style)?;
        let scope = self
            .lifetime_owner(parent)?
            .run(oxiview_reactive::Scope::new)
            .map_err(reactive_error)?;
        self.insert_reserved_scoped(id, parent, index, style, scope)
    }
    fn validate_insertion(
        &self,
        parent: NodeId,
        index: usize,
        style: &Style,
    ) -> Result<(), UiError> {
        self.validate_style(style)?;
        let len = self.node(parent)?.children.len();
        if index > len {
            return Err(UiError::ChildIndex { index, len });
        }
        if self.path(parent)?.len() >= MAX_DEPTH {
            return Err(UiError::DepthLimit);
        }
        Ok(())
    }
    fn append_scoped(
        &mut self,
        parent: NodeId,
        style: Style,
        scope: oxiview_reactive::Scope,
    ) -> Result<NodeId, UiError> {
        let index = self.node(parent)?.children.len();
        let reservation = Reservation::new(&self.identities)?;
        let id = reservation.id;
        self.insert_reserved_scoped(id, parent, index, style, scope)?;
        Ok(id)
    }
    fn insert_reserved_scoped(
        &mut self,
        id: NodeId,
        parent: NodeId,
        index: usize,
        style: Style,
        scope: oxiview_reactive::Scope,
    ) -> Result<(), UiError> {
        self.validate_insertion(parent, index, &style)?;
        self.identities
            .try_borrow_mut()
            .map_err(|_| UiError::ReservationBusy)?
            .commit(id)?;
        if id.slot >= self.nodes.len() {
            self.nodes.resize_with(id.slot + 1, || None);
        }
        let slot = self.nodes.get_mut(id.slot).ok_or(UiError::StaleNode)?;
        *slot = Some(Node {
            id,
            parent: Some(parent),
            children: Vec::new(),
            style,
            layout: LayoutBox::default(),
            intrinsic: Size::default(),
            content_revision: 1,
            measure: None,
            content: Vec::new(),
            semantics: None,
            handler: None,
            scopes: vec![scope],
            hidden: false,
            text: None,
            painting: None,
        });
        self.node_mut(parent)?.children.insert(index, id);
        self.dirty_layout(id, true, true)?;
        self.dirty_layout(parent, false, true)?;
        Ok(())
    }
    /// Updates a style atomically; returns whether visual work was invalidated.
    pub fn set_style(&mut self, id: NodeId, style: Style) -> Result<bool, UiError> {
        self.validate_style(&style)?;
        let old = &self.node(id)?.style;
        if old == &style {
            return Ok(false);
        }
        let layout = old.layout != style.layout;
        let box_geometry = old.border_radii != style.border_radii
            || old.layout.border_width != style.layout.border_width
            || old.overflow != style.overflow;
        let rebuild = box_geometry || old.opacity != style.opacity || old.z_index != style.z_index;
        let background = old.background != style.background;
        let border = old.border_color != style.border_color;
        let node = self.node_mut(id)?;
        node.style = style;
        if box_geometry {
            node.painting = None;
        }
        if layout {
            self.dirty_layout(id, true, false)?;
        } else if rebuild {
            self.dirty_paint();
        } else if (background || border)
            && let Some(draws) = self.dirty_draws(id)
        {
            draws.background |= background;
            draws.border |= border;
        }
        Ok(layout || rebuild || background || border)
    }
    /// Sets intrinsic leaf content size, invalidating its layout.
    pub fn set_intrinsic_size(&mut self, id: NodeId, size: Size) -> Result<(), UiError> {
        validate_viewport(size)?;
        if self.node(id)?.intrinsic != size {
            self.bump_measure_revision(id)?;
            self.node_mut(id)?.intrinsic = size;
        }
        Ok(())
    }
    /// Installs an intrinsic leaf measurement callback.
    pub fn set_measure(
        &mut self,
        id: NodeId,
        measure: impl NodeMeasure + 'static,
    ) -> Result<(), UiError> {
        self.bump_measure_revision(id)?;
        self.node_mut(id)?.measure = Some(Box::new(measure));
        Ok(())
    }
    /// Invalidates cached geometry after data used by custom measurement changes.
    pub fn invalidate_measure(&mut self, id: NodeId) -> Result<(), UiError> {
        if let Some(text) = &self.node(id)?.text {
            text.invalidate()?;
            self.node_mut(id)?.painting = None;
        }
        self.bump_measure_revision(id)
    }
    /// Replaces node-local custom Scene primitives, drawn after its box background.
    pub fn set_content(&mut self, id: NodeId, content: Vec<Primitive>) -> Result<(), UiError> {
        let node = self.node_mut(id)?;
        if node.content != content {
            let same_count = node.content.len() == content.len();
            node.content = content;
            if same_count {
                if let Some(draws) = self.dirty_draws(id) {
                    draws.custom = true;
                }
            } else {
                node.painting = None;
                self.dirty_paint();
            }
        }
        Ok(())
    }
    /// Stores optional owned semantics without requesting a visual frame.
    pub fn set_semantics(
        &mut self,
        id: NodeId,
        semantics: Option<NodeSemantics>,
    ) -> Result<(), UiError> {
        self.node_mut(id)?.semantics = semantics;
        Ok(())
    }
    /// Reads reserved semantic data.
    pub fn semantics(&self, id: NodeId) -> Result<Option<&NodeSemantics>, UiError> {
        Ok(self.node(id)?.semantics.as_ref())
    }
    /// Removes an ordinary subtree. Interaction-time edits should use UiCommands.
    pub fn remove(&mut self, id: NodeId) -> Result<(), UiError> {
        self.validate_remove(id)?;
        self.remove_raw(id)
    }
    /// Removes all descendants of a node, preserving that node's identity and state.
    pub fn clear(&mut self, id: NodeId) -> Result<(), UiError> {
        let children = self.node(id)?.children.clone();
        for child in children {
            self.remove_raw(child)?;
        }
        Ok(())
    }
    /// Moves a complete subtree using destination indices after conceptual removal.
    pub fn reparent(&mut self, id: NodeId, parent: NodeId, index: usize) -> Result<(), UiError> {
        let (old_parent, old_index) = self.validate_reparent(id, parent, index)?;
        if old_parent == parent && old_index == index {
            return Ok(());
        }
        if old_parent != parent {
            self.outer_scope_owner(id)?
                .adopt(self.lifetime_owner(parent)?)
                .map_err(reactive_error)?;
        }
        self.node_mut(old_parent)?.children.remove(old_index);
        self.node_mut(parent)?.children.insert(index, id);
        self.node_mut(id)?.parent = Some(parent);
        self.dirty_layout(parent, false, true)?;
        if old_parent != parent {
            self.dirty_layout(old_parent, false, true)?;
            self.dirty_layout(id, false, false)?;
        }
        Ok(())
    }
    /// Prepares logical geometry without submitting a frame.
    pub fn layout(&mut self, viewport: Size) -> Result<(), UiError> {
        validate_viewport(viewport)?;
        if self.closed {
            return Err(UiError::Closed);
        }
        if !self.layout_dirty && self.viewport == viewport {
            return Ok(());
        }
        let mut engine = std::mem::take(&mut self.engine);
        let mut output = HashMap::new();
        let result = {
            let mut adapter = TreeLayout {
                tree: self,
                output: &mut output,
                viewport,
            };
            engine.compute(&mut adapter, self.root, viewport, &self.layout_changes)
        };
        self.engine = engine;
        let stats = result.map_err(|error| UiError::Layout(error.to_string()))?;
        for (id, layout) in output {
            self.node_mut(id)?.layout = layout;
        }
        self.viewport = viewport;
        self.layout_dirty = false;
        self.layout_changes = HashMap::new();
        self.dirty_paint();
        self.diagnostics.record_layout(stats);
        Ok(())
    }
    /// Prepares geometry with a caller-selected owned layout implementation.
    /// The caller-selected engine owns its reuse policy and supplies complete
    /// geometry. A successful pass supersedes the default engine's cached
    /// geometry. The next dirty default pass starts from the live tree.
    pub fn layout_with(
        &mut self,
        engine: &mut impl LayoutEngine<NodeId>,
        viewport: Size,
    ) -> Result<LayoutStats, UiError> {
        validate_viewport(viewport)?;
        if self.closed {
            return Err(UiError::Closed);
        }
        let mut output = HashMap::new();
        let mut adapter = TreeLayout {
            tree: self,
            output: &mut output,
            viewport,
        };
        let stats = engine
            .compute(&mut adapter, self.root, viewport)
            .map_err(|error| UiError::Layout(error.to_string()))?;
        for &id in output.keys() {
            self.node(id)?;
        }
        for (id, layout) in output {
            self.node_mut(id)?.layout = layout;
        }
        self.viewport = viewport;
        self.layout_dirty = false;
        self.layout_changes = HashMap::new();
        self.engine.clear_cache();
        self.dirty_paint();
        self.diagnostics.record_layout(stats);
        Ok(stats)
    }
    /// Returns a committed border box in logical window coordinates.
    pub fn bounds(&self, id: NodeId) -> Result<Rect, UiError> {
        let path = self.path(id)?;
        let mut x = 0.0;
        let mut y = 0.0;
        for ancestor in path {
            let p = self.node(ancestor)?.layout.location;
            x += p.x();
            y += p.y();
        }
        Ok(Rect::from_origin_size(
            Point::new(x, y),
            self.node(id)?.layout.size,
        ))
    }
    /// Returns the current capture holder.
    pub fn capture_holder(&self) -> Option<NodeId> {
        self.pointer.capture
    }
    /// Returns the innermost retained hover member, which may differ from geometry.
    pub fn hover_target(&self) -> Option<NodeId> {
        self.pointer.hover.last().copied()
    }
    /// Returns the retained root-to-target hover path.
    pub fn hover_path(&self) -> &[NodeId] {
        &self.pointer.hover
    }
    fn node(&self, id: NodeId) -> Result<&Node, UiError> {
        if id.owner != self.owner {
            return Err(UiError::ForeignNode);
        }
        if self.closed {
            return Err(UiError::Closed);
        }
        if let Some(Some(node)) = self.nodes.get(id.slot)
            && node.id == id
        {
            return Ok(node);
        }
        match self
            .identities
            .try_borrow()
            .map_err(|_| UiError::ReservationBusy)?
            .state(id)?
        {
            IdentityState::Reserved => Err(UiError::NotYetCreated),
            _ => Err(UiError::StaleNode),
        }
    }
    fn node_mut(&mut self, id: NodeId) -> Result<&mut Node, UiError> {
        self.node(id)?;
        self.nodes
            .get_mut(id.slot)
            .and_then(Option::as_mut)
            .ok_or(UiError::StaleNode)
    }
    fn lifetime_owner(&self, id: NodeId) -> Result<oxiview_reactive::Owner, UiError> {
        self.node(id)?
            .scopes
            .first()
            .map(oxiview_reactive::Scope::owner)
            .ok_or_else(|| UiError::View("node lifetime scope is missing".into()))
    }
    fn outer_scope_owner(&self, id: NodeId) -> Result<oxiview_reactive::Owner, UiError> {
        self.node(id)?
            .scopes
            .last()
            .map(oxiview_reactive::Scope::owner)
            .ok_or_else(|| UiError::View("node ownership scope is missing".into()))
    }
    fn path(&self, id: NodeId) -> Result<Vec<NodeId>, UiError> {
        let mut path = Vec::new();
        let mut next = Some(id);
        while let Some(id) = next {
            if path.len() >= MAX_DEPTH {
                return Err(UiError::DepthLimit);
            }
            let node = self.node(id)?;
            path.push(id);
            next = node.parent;
        }
        path.reverse();
        Ok(path)
    }
    fn suppressed(&self, id: NodeId) -> Result<bool, UiError> {
        for ancestor in self.path(id)? {
            if self.node(ancestor)?.hidden
                || self.node(ancestor)?.style.layout.display == Display::None
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn descendant(&self, id: NodeId, ancestor: NodeId) -> Result<bool, UiError> {
        Ok(self.path(id)?.contains(&ancestor))
    }
    fn validate_remove(&self, id: NodeId) -> Result<(), UiError> {
        self.node(id)?;
        if id == self.root {
            return Err(UiError::RootOperation);
        }
        Ok(())
    }
    fn validate_reparent(
        &self,
        id: NodeId,
        parent: NodeId,
        index: usize,
    ) -> Result<(NodeId, usize), UiError> {
        self.validate_remove(id)?;
        let node = self.node(id)?;
        let old_parent = node.parent.ok_or(UiError::RootOperation)?;
        let len = self
            .node(parent)?
            .children
            .len()
            .saturating_sub(usize::from(old_parent == parent));
        if index > len {
            return Err(UiError::ChildIndex { index, len });
        }
        if self.descendant(parent, id)? {
            return Err(UiError::Cycle);
        }
        if old_parent != parent {
            let parent_depth = self.path(parent)?.len();
            if parent_depth > self.path(old_parent)?.len() {
                let mut stack = vec![(id, parent_depth + 1)];
                while let Some((current, depth)) = stack.pop() {
                    if depth > MAX_DEPTH {
                        return Err(UiError::DepthLimit);
                    }
                    for child in &self.node(current)?.children {
                        stack.push((*child, depth + 1));
                    }
                }
            }
        }
        let old_index = self
            .node(old_parent)?
            .children
            .iter()
            .position(|child| *child == id)
            .ok_or(UiError::StaleNode)?;
        if old_parent != parent {
            self.outer_scope_owner(id)?
                .can_adopt(self.lifetime_owner(parent)?)
                .map_err(reactive_error)?;
        }
        Ok((old_parent, old_index))
    }
    fn postorder(&self, id: NodeId) -> Result<Vec<NodeId>, UiError> {
        let mut result = Vec::new();
        let mut stack = vec![(id, false)];
        while let Some((current, visited)) = stack.pop() {
            if visited {
                result.push(current);
            } else {
                stack.push((current, true));
                for child in self.node(current)?.children.iter().rev() {
                    stack.push((*child, false));
                }
            }
        }
        Ok(result)
    }
    fn remove_raw(&mut self, id: NodeId) -> Result<(), UiError> {
        let order = self.postorder(id)?;
        let parent = self.node(id)?.parent.ok_or(UiError::RootOperation)?;
        self.node_mut(parent)?.children.retain(|child| *child != id);
        self.dirty_layout(parent, false, true)?;
        for current in order {
            self.pointer.hover.retain(|hovered| *hovered != current);
            if self.pointer.capture == Some(current) {
                self.pointer.capture = None;
            }
            let node = self.nodes.get_mut(current.slot).and_then(Option::take);
            self.identities
                .try_borrow_mut()
                .map_err(|_| UiError::ReservationBusy)?
                .release(current, false);
            if self.engine.contains_cached_node(current) {
                self.layout_changes.insert(current, LayoutChange::Remove);
            } else {
                // A creation removed before its first layout has no cached state to retire.
                self.layout_changes.remove(&current);
            }
            drop(node);
        }
        Ok(())
    }
    fn dirty_layout(&mut self, id: NodeId, style: bool, children: bool) -> Result<(), UiError> {
        let parent = self.node(id)?.parent;
        let change = self
            .layout_changes
            .entry(id)
            .or_insert(LayoutChange::Update {
                parent,
                style: false,
                children: false,
            });
        if let LayoutChange::Update {
            style: old_style,
            children: old_children,
            ..
        } = *change
        {
            *change = LayoutChange::Update {
                parent,
                style: style || old_style,
                children: children || old_children,
            };
        }
        self.layout_dirty = true;
        self.dirty_paint();
        Ok(())
    }
    fn dirty_paint(&mut self) {
        self.paint_dirty = true;
        self.paint_cache = None;
    }
    fn bump_measure_revision(&mut self, id: NodeId) -> Result<(), UiError> {
        let revision = self
            .node(id)?
            .content_revision
            .checked_add(1)
            .ok_or(UiError::RevisionExhausted)?;
        self.node_mut(id)?.content_revision = revision;
        self.dirty_layout(id, false, false)
    }
    fn validate_style(&self, style: &Style) -> Result<(), UiError> {
        style
            .validate()
            .map_err(|error| UiError::Style(error.to_string()))
    }
}

impl Drop for UiTree {
    fn drop(&mut self) {
        let _ = self.mailbox.close();
        let _ = self.mailbox.take_pending();
        self.paint_cache = None;
        if !self.closed {
            match self.postorder(self.root) {
                Ok(order) => {
                    for id in order {
                        if let Some(slot) = self.nodes.get_mut(id.slot) {
                            drop(slot.take());
                        }
                    }
                }
                Err(error) => tracing::error!(%error, "UI teardown encountered invalid structure"),
            }
        }
        if let Ok(mut identities) = self.identities.try_borrow_mut() {
            identities.closed = true;
        }
    }
}

struct TreeLayout<'a> {
    tree: &'a UiTree,
    output: &'a mut HashMap<NodeId, LayoutBox>,
    viewport: Size,
}
impl LayoutTree for TreeLayout<'_> {
    type NodeId = NodeId;
    fn children(&self, id: NodeId) -> &[NodeId] {
        self.tree
            .node(id)
            .map_or(&[], |node| node.children.as_slice())
    }
    fn layout_input(&self, id: NodeId) -> LayoutInput {
        let mut input = self.tree.node(id).map_or_else(
            |_| LayoutInput::default(),
            |node| crate::ui_projection::project_style(&node.style.layout),
        );
        if self.tree.node(id).is_ok_and(|node| node.hidden) {
            input.display = oxiview_layout::Display::None;
        }
        if id == self.tree.root {
            if input.width == oxiview_layout::Length::Auto {
                input.width = oxiview_layout::Length::Points(self.viewport.width());
            }
            if input.height == oxiview_layout::Length::Auto {
                input.height = oxiview_layout::Length::Points(self.viewport.height());
            }
        }
        input
    }
    fn set_layout(&mut self, id: NodeId, layout: LayoutBox) {
        self.output.insert(id, layout);
    }
    fn contains_node(&self, id: NodeId) -> bool {
        self.tree.contains(id)
    }
    fn content_revision(&self, id: NodeId) -> u64 {
        self.tree.node(id).map_or(0, |node| node.content_revision)
    }
    fn measure(
        &mut self,
        id: NodeId,
        known: Dimensions<Option<f64>>,
        available: Dimensions<AvailableSpace>,
    ) -> Dimensions<f64> {
        if let Ok(node) = self.tree.node(id)
            && let Some(measure) = &node.measure
        {
            return measure.measure(known, available);
        }
        let intrinsic = self
            .tree
            .node(id)
            .map_or(Size::default(), |node| node.intrinsic);
        Dimensions {
            width: known.width.unwrap_or(intrinsic.width()),
            height: known.height.unwrap_or(intrinsic.height()),
        }
    }
}

fn reactive_error(error: oxiview_reactive::ReactiveError) -> UiError {
    UiError::Reactive(error.to_string())
}

fn validate_viewport(size: Size) -> Result<(), UiError> {
    if !size.width().is_finite()
        || !size.height().is_finite()
        || size.width() < 0.0
        || size.height() < 0.0
    {
        return Err(UiError::InvalidViewport);
    }
    Ok(())
}

#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tree_contract_tests;
