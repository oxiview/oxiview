use super::{
    FocusPolicy, NodeId, PointerEventKind, UiCommands, UiError, UiPointerEvent, UiTree, View,
    allocate_tree_owner, input::Command, node,
};
use oxiview_base::{Point, Rect, Size};
use oxiview_layout::{AvailableSpace, Dimensions};
use oxiview_platform::{
    PointerButton, TextCommitKind, TextInputEdit, TextInputEvent, TextInputGeometry,
    TextInputSession, TextInputSnapshot,
};
use oxiview_text::{
    Caret, EditError, EditLayout, EditMode, Editor, NativeEdit, Selection, TextStyle, TextSystem,
};
use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    ops::Range,
    rc::Rc,
    sync::atomic::AtomicU64,
    time::Duration,
};

mod keys;
mod painting;
#[cfg(test)]
mod tests;
pub(super) use painting::EditorDraws;

static NEXT_EDITOR_SESSION: AtomicU64 = AtomicU64::new(1);
const BLINK_INTERVAL: Duration = Duration::from_millis(500);

/// Cloneable window-thread control shared by application logic and one retained editor.
#[derive(Clone)]
pub struct Editable(Rc<EditableInner>);

struct EditableInner {
    model: Rc<RefCell<Editor>>,
    node: Cell<Option<NodeId>>,
}

impl Editable {
    /// Creates an editable control with a new model.
    pub fn new(source: impl Into<String>, mode: EditMode) -> Self {
        Self::from_editor(Rc::new(RefCell::new(Editor::new(source, mode))))
    }

    /// Binds a control to an existing window-thread model.
    pub fn from_editor(editor: Rc<RefCell<Editor>>) -> Self {
        Self(Rc::new(EditableInner {
            model: editor,
            node: Cell::new(None),
        }))
    }

    /// Shares the model used by the retained view and the native input bridge.
    pub fn editor(&self) -> Rc<RefCell<Editor>> {
        self.0.model.clone()
    }

    /// Returns the mounted node's identity while it is retained.
    pub fn node(&self) -> Option<NodeId> {
        self.0.node.get()
    }

    fn model(&self) -> Result<Ref<'_, Editor>, UiError> {
        self.0.model.try_borrow().map_err(|_| UiError::HandlerBusy)
    }

    fn model_mut(&self) -> Result<RefMut<'_, Editor>, UiError> {
        self.0
            .model
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)
    }
}

/// Retained editor geometry and native text state in logical window coordinates.
#[derive(Clone)]
pub struct EditorWindowState {
    /// Focused editor node.
    pub node: NodeId,
    /// Focus lifetime.
    pub session: u64,
    /// Revision of the editor's current presentation.
    pub revision: u64,
    /// Base revision for input-method callbacks.
    pub input_revision: u64,
    /// Highest native event sequence already accepted.
    pub acknowledged: u64,
    /// The displayed text, including active preedit.
    pub text: String,
    /// Selection and active caret in displayed UTF-8 bytes.
    pub selection: Selection,
    /// Marked range in displayed UTF-8 bytes.
    pub marked: Option<Range<usize>>,
    /// Active insertion rectangle.
    pub caret: Rect,
    /// Visible content box in the window.
    pub viewport: Rect,
    layout: EditLayout,
    origin: Point,
}

impl EditorWindowState {
    /// Returns the actual range and geometry of the first connected visual segment.
    pub fn first_rect(&self, range: Range<usize>) -> Option<(Range<usize>, Rect)> {
        if range.is_empty() && range.start == self.selection.active.offset {
            return Some((range, self.caret));
        }
        self.layout
            .first_rect(range)
            .map(|(range, rect)| (range, translate_rect(rect, self.origin)))
    }

    /// Finds the UTF-8 insertion offset nearest the supplied window point.
    pub fn character_index_at_point(&self, point: Point) -> usize {
        self.layout.hit_test(relative(point, self.origin)).offset
    }

    fn into_platform(self) -> TextInputSnapshot {
        let [x0, y0, x1, y1] = self.viewport.edges();
        let visible_y = y0 - self.origin.y()..y1 - self.origin.y();
        let mut geometry = Vec::new();
        let mut last = None;
        let active = self.selection.active.offset;
        let mut active_included = false;
        for (range, rect) in self.layout.visible_range_rects(visible_y) {
            let rect = translate_rect(rect, self.origin);
            let [left, _, right, _] = rect.edges();
            if x0 >= right || left >= x1 {
                continue;
            }
            if last != Some(range.start) {
                active_included |= range.start == active;
                if let Some((range, rect)) = self.first_rect(range.start..range.start) {
                    geometry.push(TextInputGeometry { range, rect });
                }
                last = Some(range.start);
            }
            geometry.push(TextInputGeometry { range, rect });
        }
        if !active_included && nonempty(intersect(self.viewport, self.caret)) {
            let index = geometry.partition_point(|fragment| fragment.range.start < active);
            geometry.insert(
                index,
                TextInputGeometry {
                    range: active..active,
                    rect: self.caret,
                },
            );
        }
        TextInputSnapshot {
            session: TextInputSession(self.session),
            revision: self.input_revision,
            acknowledged: self.acknowledged,
            text: self.text,
            selection: self.selection.range(),
            marked: self.marked,
            caret: self.caret,
            geometry,
        }
    }
}

/// An error while applying input to the focused retained editor.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditorInputError {
    /// The model rejected a stale or invalid edit.
    #[error(transparent)]
    Edit(#[from] EditError),
    /// Retained UI or geometry preparation failed.
    #[error(transparent)]
    Ui(#[from] UiError),
}

struct CachedLayout {
    layout: EditLayout,
    revision: u64,
    width: Option<f64>,
}

#[derive(Clone, Copy)]
struct Blink {
    visible: bool,
    deadline: Option<Duration>,
}

impl Default for Blink {
    fn default() -> Self {
        Self {
            visible: true,
            deadline: None,
        }
    }
}

pub(super) struct EditorNode {
    control: Editable,
    system: Rc<RefCell<TextSystem>>,
    style: TextStyle,
    layout: RefCell<Option<CachedLayout>>,
    scroll: Cell<Point>,
    viewport: Cell<Size>,
    reveal: Cell<bool>,
    blink: Cell<Blink>,
    last_edit: Cell<u64>,
    desired_x: Cell<Option<f64>>,
    drag_anchor: Cell<Option<Caret>>,
}

impl Drop for EditorNode {
    fn drop(&mut self) {
        self.control.0.node.set(None);
        if let Ok(mut model) = self.control.model_mut()
            && let Some(session) = model.session()
        {
            let _ = model.end_session(session);
        }
    }
}

struct PreparedGeometry {
    layout: EditLayout,
    origin: Point,
    viewport: Rect,
}

impl PreparedGeometry {
    fn local(&self, point: Point) -> Point {
        relative(point, self.origin)
    }
}

/// Builds a focusable retained editor using either line policy.
///
/// Apply an ordinary View style to size the viewport. The field participates in
/// Tab traversal, handles its default editing Keymap, and accepts platform text
/// through [UiTree::apply_text_input_event].
///
/// ```
/// use oxiview_core::{EditMode, Editable, Selection, TextStyle, TextSystem, UiTree, editable_text};
/// use std::{cell::RefCell, rc::Rc};
///
/// let editor = Editable::new("# Notes", EditMode::MultiLine);
/// editor.editor().borrow_mut().set_selection(Selection::caret(7))?;
/// let fonts = Rc::new(RefCell::new(TextSystem::new()));
/// let mut tree = UiTree::new()?;
/// let field = tree.mount(tree.root(), editable_text(fonts, editor, TextStyle::default()))?;
/// tree.focus(field)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn editable_text(system: Rc<RefCell<TextSystem>>, control: Editable, style: TextStyle) -> View {
    let line_height = style.font_size * style.line_height;
    let preferred_width = style.font_size * 10.0;
    let mut view = node()
        .focusable(FocusPolicy::Tab)
        .measure(
            move |known: Dimensions<Option<f64>>, available: Dimensions<AvailableSpace>| {
                let width = known.width.unwrap_or(match available.width {
                    AvailableSpace::Definite(width) => preferred_width.min(width),
                    AvailableSpace::MaxContent => preferred_width,
                    AvailableSpace::MinContent => 0.0,
                });
                Dimensions {
                    width,
                    height: known.height.unwrap_or(line_height),
                }
            },
        )
        .on_pointer(handle_pointer);
    view.setup.push(Box::new(move |tree, id, _| {
        if control.node().is_some()
            || tree
                .editors
                .values()
                .any(|node| Rc::ptr_eq(&control.0.model, &node.control.0.model))
        {
            return Err(UiError::View("editable control is already mounted".into()));
        }
        let (mode, revision) = {
            let model = control.model()?;
            (model.mode(), model.presentation().revision)
        };
        keys::install(tree, id, mode)?;
        control.0.node.set(Some(id));
        tree.editors.insert(
            id,
            EditorNode {
                control,
                system,
                style,
                layout: RefCell::new(None),
                scroll: Cell::default(),
                viewport: Cell::default(),
                reveal: Cell::new(true),
                blink: Cell::default(),
                last_edit: Cell::new(revision),
                desired_x: Cell::new(None),
                drag_anchor: Cell::new(None),
            },
        );
        Ok(())
    }));
    view
}

fn handle_pointer(event: &UiPointerEvent, tree: &UiTree, commands: &mut UiCommands) {
    let id = event.current_target;
    let primary = event.button == Some(PointerButton::Primary);
    let dragging = tree.capture_holder() == Some(id);
    if tree.focused_node() != Some(id) && event.kind != PointerEventKind::Cancel {
        return;
    }
    match event.kind {
        PointerEventKind::Down if primary => {
            commands.editor_pointer(*event);
            commands.capture(id);
            commands.stop_propagation();
        }
        PointerEventKind::Move if dragging && event.buttons & 1 != 0 => {
            commands.editor_pointer(*event);
            commands.stop_propagation();
        }
        PointerEventKind::Up if primary && dragging => {
            commands.editor_pointer(*event);
            commands.release_capture(id);
            commands.stop_propagation();
        }
        PointerEventKind::Cancel => commands.editor_pointer(*event),
        _ => {}
    }
}

impl UiCommands {
    /// Queues an editor refresh after a direct change through its shared model.
    pub fn refresh_editor(&mut self, id: NodeId) {
        self.push(Command::Mutation(
            id,
            Box::new(move |tree, _, _| tree.refresh_editor(id).map(|_| ())),
        ));
    }

    fn editor_pointer(&mut self, event: UiPointerEvent) {
        self.push(Command::Mutation(
            event.current_target,
            Box::new(move |tree, _, _| tree.editor_pointer(event)),
        ));
    }
}

impl UiTree {
    /// Schedules repaint and caret reveal after an application edits the shared model.
    pub fn refresh_editor(&mut self, id: NodeId) -> Result<bool, UiError> {
        if !self.editor(id)?.note_edit(true)? {
            return Ok(false);
        }
        self.dirty_editor(id);
        Ok(true)
    }

    /// Scrolls the viewport by logical horizontal and vertical distances.
    pub fn scroll_editor(&mut self, id: NodeId, delta: Point) -> Result<bool, UiError> {
        if !delta.x().is_finite() || !delta.y().is_finite() {
            return Err(UiError::InvalidViewport);
        }
        self.layout(self.viewport)?;
        if self.suppressed(id)? {
            return Ok(false);
        }
        let editor = self.editor(id)?;
        let prepared = editor.prepare_geometry(self.editor_viewport(id)?)?;
        let old = editor.scroll.get();
        let scroll = clamp_scroll(
            Point::new(old.x() + delta.x(), old.y() + delta.y()),
            prepared.layout.bounds(),
            prepared.viewport.size(),
        );
        if old == scroll {
            return Ok(false);
        }
        editor.scroll.set(scroll);
        editor.reveal.set(false);
        self.dirty_editor(id);
        Ok(true)
    }

    /// Returns the current focused presentation and its window-space editing geometry.
    pub fn focused_editor_state(
        &mut self,
        viewport: Size,
    ) -> Result<Option<EditorWindowState>, UiError> {
        if self.active_editor_id().is_none() {
            return Ok(None);
        }
        self.layout(viewport)?;
        let Some(id) = self.active_editor_id() else {
            return Ok(None);
        };
        self.refresh_editor(id)?;
        let editor = self.editor(id)?;
        let geometry = editor.prepare_geometry(self.editor_viewport(id)?)?;
        let visible = self.editor_visible_rect(id, geometry.viewport)?;
        let model = editor.control.model()?;
        let Some(session) = model.session() else {
            return Ok(None);
        };
        let presentation = model.presentation();
        let caret = caret_for_geometry(&geometry, presentation.selection.active)?;
        Ok(Some(EditorWindowState {
            node: id,
            session,
            revision: presentation.revision,
            input_revision: model.input_revision(),
            acknowledged: model.acknowledged(),
            text: presentation.text.to_owned(),
            selection: presentation.selection,
            marked: presentation.marked,
            caret,
            viewport: visible,
            layout: geometry.layout,
            origin: geometry.origin,
        }))
    }

    /// Builds the snapshot sent to the platform when the focused editor changes.
    pub fn focused_text_input_snapshot(
        &mut self,
        viewport: Size,
    ) -> Result<Option<TextInputSnapshot>, UiError> {
        self.focused_editor_state(viewport)
            .map(|state| state.map(EditorWindowState::into_platform))
    }

    /// Applies one ordered operation to the active editor and schedules visual updates.
    pub fn apply_focused_editor_input(
        &mut self,
        session: u64,
        revision: u64,
        sequence: u64,
        edit: NativeEdit,
        now: Duration,
    ) -> Result<bool, EditorInputError> {
        self.apply_editor_input(session, revision, sequence, edit, now, None)
    }

    /// Routes the platform's owned input and resolved text commands to the focused editor.
    pub fn apply_text_input_event(
        &mut self,
        event: TextInputEvent,
        viewport: Size,
    ) -> Result<bool, EditorInputError> {
        let id = self.active_editor_id().ok_or(EditError::StaleInput)?;
        let session = event.session.0;
        let revision = event.revision;
        let sequence = event.sequence;
        let (edit, desired) = match event.edit {
            TextInputEdit::SetMarked {
                range,
                text,
                selection,
            } => (
                NativeEdit::Mark {
                    text,
                    replacement: Some(range),
                    selection,
                },
                None,
            ),
            TextInputEdit::Commit { range, text, kind } => {
                let replacement = Some(range);
                let edit = match kind {
                    TextCommitKind::Typing => NativeEdit::Typing { text, replacement },
                    TextCommitKind::Composition => NativeEdit::Commit { text, replacement },
                };
                (edit, None)
            }
            TextInputEdit::Cancel => (NativeEdit::Cancel, None),
            TextInputEdit::Command(command) => {
                self.editor(id)?
                    .control
                    .model()?
                    .check_native(session, revision, sequence)?;
                self.layout(viewport)?;
                if self.active_editor_id() != Some(id) {
                    return Err(EditError::StaleInput.into());
                }
                let Some((edit, desired)) = self
                    .editor(id)?
                    .resolve_command(self.editor_viewport(id)?, command)?
                else {
                    return Ok(false);
                };
                (NativeEdit::Command(edit), desired)
            }
            _ => return Ok(false),
        };
        self.apply_editor_input(session, revision, sequence, edit, event.timestamp, desired)
    }

    fn apply_editor_input(
        &mut self,
        session: u64,
        revision: u64,
        sequence: u64,
        edit: NativeEdit,
        now: Duration,
        desired: Option<f64>,
    ) -> Result<bool, EditorInputError> {
        let id = self.active_editor_id().ok_or(EditError::StaleInput)?;
        let editor = self.editor(id)?;
        let changed = editor
            .control
            .model_mut()?
            .apply_native(session, revision, sequence, edit, now)?;
        editor.desired_x.set(desired);
        if changed {
            editor.note_edit(false)?;
            self.dirty_editor(id);
        }
        Ok(changed)
    }

    /// Advances the focused caret and returns its next absolute update deadline.
    pub fn advance_editor_caret(
        &mut self,
        now: Duration,
    ) -> Result<(bool, Option<Duration>), UiError> {
        let Some(id) = self.active_editor_id() else {
            return Ok((false, None));
        };
        let editor = self.editor(id)?;
        let viewport = self.editor_viewport(id)?;
        let visible = self.editor_visible_rect(id, viewport)?;
        let selection = editor.control.model()?.presentation().selection;
        if !selection.is_empty() || !nonempty(visible) {
            editor.blink.set(Blink::default());
            return Ok((false, None));
        }
        let geometry = editor.prepare_geometry(viewport)?;
        let caret = caret_for_geometry(&geometry, selection.active)?;
        if !nonempty(intersect(visible, caret)) {
            editor.blink.set(Blink::default());
            return Ok((false, None));
        }
        let mut blink = editor.blink.get();
        let changed = blink.deadline.is_some_and(|deadline| now >= deadline);
        if changed {
            blink.visible = !blink.visible;
        }
        if blink.deadline.is_none() || changed {
            blink.deadline = now.checked_add(BLINK_INTERVAL);
        }
        editor.blink.set(blink);
        if changed {
            self.dirty_editor(id);
        }
        Ok((changed, blink.deadline))
    }

    /// Updates editor activation for hosts that manage window lifecycle separately.
    /// Ordinary dispatch handles activation and visibility changes automatically.
    pub fn set_editor_window_active(&mut self, active: bool) -> Result<bool, UiError> {
        let had_editor = self.active_editor_id().is_some();
        self.update_window_input(Some(active), None)?;
        Ok(had_editor != self.active_editor_id().is_some())
    }

    fn active_editor_id(&self) -> Option<NodeId> {
        self.focused_node()
            .filter(|id| self.accepts_input() && self.editors.contains_key(id))
    }

    fn editor(&self, id: NodeId) -> Result<&EditorNode, UiError> {
        self.node(id)?;
        self.editors
            .get(&id)
            .ok_or_else(|| UiError::View("node is not an editable field".into()))
    }

    pub(super) fn editor_focus_transition(
        &mut self,
        old: Option<NodeId>,
        new: Option<NodeId>,
    ) -> Result<(), UiError> {
        {
            // Acquire both borrows before mutating either session, so a busy
            // incoming model leaves the outgoing editor focused and untouched.
            let borrow = |id: Option<NodeId>| {
                id.and_then(|id| self.editors.get(&id))
                    .map(|editor| editor.control.model_mut())
                    .transpose()
            };
            let outgoing = borrow(old)?;
            let incoming = borrow(new)?;
            let next_session = incoming
                .as_ref()
                .map(|_| allocate_tree_owner(&NEXT_EDITOR_SESSION))
                .transpose()?;
            if let Some(mut model) = outgoing
                && let Some(session) = model.session()
            {
                model.end_session(session).map_err(ui_edit_error)?;
            }
            if let Some((mut model, session)) = incoming.zip(next_session) {
                model.begin_session(session).map_err(ui_edit_error)?;
            }
        }
        for (target, focused) in [(old, false), (new, true)] {
            let Some(id) = target else { continue };
            let Some(editor) = self.editors.get(&id) else {
                continue;
            };
            editor.note_edit(true)?;
            editor.blink.set(Blink::default());
            editor.desired_x.set(None);
            editor.drag_anchor.set(None);
            editor.reveal.set(focused);
            self.dirty_editor(id);
        }
        Ok(())
    }

    fn dirty_editor(&mut self, id: NodeId) {
        if let Some(draws) = self.dirty_draws(id) {
            draws.editor = true;
        }
    }

    fn editor_pointer(&mut self, event: UiPointerEvent) -> Result<(), UiError> {
        let id = event.current_target;
        let editor = self.editor(id)?;
        if event.kind == PointerEventKind::Cancel {
            editor.drag_anchor.set(None);
            return Ok(());
        }
        let geometry = editor.prepare_geometry(self.editor_viewport(id)?)?;
        let layout = editor.committed_layout(&geometry)?;
        let caret = layout.hit_test(geometry.local(event.position));
        let anchor = match event.kind {
            PointerEventKind::Down => {
                editor.drag_anchor.set(Some(caret));
                caret
            }
            PointerEventKind::Up => editor.drag_anchor.take().unwrap_or(caret),
            _ => editor.drag_anchor.get().unwrap_or(caret),
        };
        let selection = Selection {
            anchor,
            active: caret,
        };
        editor
            .control
            .model_mut()?
            .set_selection(selection)
            .map_err(ui_edit_error)?;
        editor.desired_x.set(None);
        self.refresh_editor(id)?;
        Ok(())
    }

    pub(super) fn editor_viewport(&self, id: NodeId) -> Result<Rect, UiError> {
        let layout = self.node(id)?.layout;
        let at = self.bounds(id)?.origin();
        let left = layout.border.left + layout.padding.left;
        let top = layout.border.top + layout.padding.top;
        let width =
            (layout.size.width() - left - layout.border.right - layout.padding.right).max(0.0);
        let height =
            (layout.size.height() - top - layout.border.bottom - layout.padding.bottom).max(0.0);
        Ok(Rect::from_origin_size(
            Point::new(at.x() + left, at.y() + top),
            Size::new(width, height),
        ))
    }

    fn editor_visible_rect(&self, id: NodeId, viewport: Rect) -> Result<Rect, UiError> {
        let mut clip = intersect(
            viewport,
            Rect::from_origin_size(Point::default(), self.viewport),
        );
        for node in self.ancestors(id).skip(1) {
            let node = node?;
            if node.clips_descendants() {
                let (bounds, _) = self.padding_clip(node.id)?;
                clip = intersect(clip, bounds);
            }
        }
        Ok(clip)
    }
}

impl EditorNode {
    fn note_edit(&self, reset_column: bool) -> Result<bool, UiError> {
        let revision = self.control.model()?.presentation().revision;
        if self.last_edit.replace(revision) == revision {
            return Ok(false);
        }
        if reset_column {
            self.desired_x.set(None);
        }
        self.reveal.set(true);
        self.blink.set(Blink::default());
        Ok(true)
    }

    fn layout_for(&self, model: &Editor, viewport: Rect) -> Result<EditLayout, UiError> {
        let width = (model.mode() == EditMode::MultiLine).then_some(viewport.size().width());
        let presented = model.presentation();
        let mut cache = self
            .layout
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)?;
        if let Some(cached) = cache.as_ref()
            && cached.revision == presented.text_revision
            && cached.width == width
        {
            return Ok(cached.layout.clone());
        }
        let mut fonts = self
            .system
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)?;
        let layout = if let Some(cached) = cache.as_ref() {
            fonts.update_editable_layout(&cached.layout, presented.text, &self.style, width)
        } else {
            fonts.layout_editable(presented.text, &self.style, width)
        }
        .map_err(ui_layout_error)?;
        *cache = Some(CachedLayout {
            layout: layout.clone(),
            revision: presented.text_revision,
            width,
        });
        Ok(layout)
    }

    fn committed_layout(&self, geometry: &PreparedGeometry) -> Result<EditLayout, UiError> {
        let model = self.control.model()?;
        if model.presentation().marked.is_none() {
            return Ok(geometry.layout.clone());
        }
        let width =
            (model.mode() == EditMode::MultiLine).then_some(geometry.viewport.size().width());
        self.system
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)?
            .update_editable_layout(&geometry.layout, model.source(), &self.style, width)
            .map_err(ui_layout_error)
    }

    fn prepare_geometry(&self, viewport: Rect) -> Result<PreparedGeometry, UiError> {
        let model = self.control.model()?;
        let layout = self.layout_for(&model, viewport)?;
        let size = viewport.size();
        let resized = self.viewport.replace(size) != size;
        let mut scroll = self.scroll.get();
        if (self.reveal.replace(false) || (resized && model.session().is_some()))
            && let Some(caret) = layout.caret_rect(model.presentation().selection.active)
        {
            scroll = reveal_caret(scroll, size, caret);
        }
        scroll = clamp_scroll(scroll, layout.bounds(), size);
        self.scroll.set(scroll);
        let at = viewport.origin();
        let origin = Point::new(at.x() - scroll.x(), at.y() - scroll.y());
        Ok(PreparedGeometry {
            layout,
            origin,
            viewport,
        })
    }
}

fn clamp_scroll(scroll: Point, bounds: Rect, viewport: Size) -> Point {
    let [left, top, right, bottom] = bounds.edges();
    Point::new(
        scroll
            .x()
            .clamp(left.min(0.0), (right - viewport.width()).max(0.0)),
        scroll
            .y()
            .clamp(top.min(0.0), (bottom - viewport.height()).max(0.0)),
    )
}

fn reveal_caret(scroll: Point, viewport: Size, rect: Rect) -> Point {
    fn reveal_axis(scroll: f64, size: f64, low: f64, high: f64) -> f64 {
        let margin = ((size - (high - low)) / 2.0).clamp(0.0, 4.0);
        if low - margin < scroll || high - low > size {
            low - margin
        } else if high + margin > scroll + size {
            high + margin - size
        } else {
            scroll
        }
    }
    let [left, top, right, bottom] = rect.edges();
    Point::new(
        reveal_axis(scroll.x(), viewport.width(), left, right),
        reveal_axis(scroll.y(), viewport.height(), top, bottom),
    )
}

fn caret_for_geometry(geometry: &PreparedGeometry, caret: Caret) -> Result<Rect, UiError> {
    geometry
        .layout
        .caret_rect(caret)
        .map(|rect| translate_rect(rect, geometry.origin))
        .ok_or_else(|| UiError::Text("editing caret lies outside displayed text".into()))
}

fn translate_rect(rect: Rect, delta: Point) -> Rect {
    let [left, top, right, bottom] = rect.edges();
    Rect::new(
        left + delta.x(),
        top + delta.y(),
        right + delta.x(),
        bottom + delta.y(),
    )
}

fn relative(point: Point, origin: Point) -> Point {
    Point::new(point.x() - origin.x(), point.y() - origin.y())
}

fn intersect(left: Rect, right: Rect) -> Rect {
    let [ax0, ay0, ax1, ay1] = left.edges();
    let [bx0, by0, bx1, by1] = right.edges();
    let x = ax0.max(bx0);
    let y = ay0.max(by0);
    Rect::new(x, y, ax1.min(bx1).max(x), ay1.min(by1).max(y))
}

fn nonempty(rect: Rect) -> bool {
    let size = rect.size();
    size.width() > 0.0 && size.height() > 0.0
}

fn ui_edit_error(error: EditError) -> UiError {
    UiError::Text(error.to_string())
}

fn ui_layout_error(error: oxiview_text::Error) -> UiError {
    UiError::Text(error.to_string())
}
