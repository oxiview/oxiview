//! Window-thread document controller and native/headless application entry point.

mod actions;
mod file_actions;
mod presentation;
#[cfg(test)]
mod tests;

use self::{
    actions::Matches,
    file_actions::{FileActions, Intent},
    presentation::Presentation,
};
use crate::{
    commands::{Command, Commands},
    document::{Document, Files},
    preview_pane::PreviewPane,
    view::{self, Slots, Ui},
};
use oxiview_base::{Point, Rect, Size};
use oxiview_core::{
    Application, Editable, Error, FrameTiming, NodeId, PaintedScene, Scheduler, TextSystem, UiTree,
};
use oxiview_platform::{
    ClipboardRequest, ClipboardResponse, FilePanelKind, FilePanelRequest, PhysicalSize,
    PlatformCommand, RequestId, TextEditingCommand, TextInputEdit, TextInputEvent, WindowEvent,
    WindowWaker,
};
use oxiview_text::{EditMode, Editor, Selection};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

pub(crate) fn error(e: impl std::fmt::Display) -> Error {
    Error::Application(e.to_string())
}
pub(crate) const WELCOME: &str = include_str!("../fixtures/acceptance.md");
pub(crate) fn fixture_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(contents) = exe.parent().and_then(Path::parent)
        && let resources = contents.join("Resources/fixtures")
        && resources.join("hill.png").is_file()
    {
        return resources;
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

struct ClipTarget {
    node: NodeId,
    field: Editable,
    revision: u64,
    session: u64,
    range: std::ops::Range<usize>,
    command: Command,
}

pub(crate) struct EditorApp {
    pub(crate) tree: UiTree,
    pub(crate) ui: Ui,
    pub(crate) preview: PreviewPane,
    pub(crate) viewport: Size,
    physical_size: PhysicalSize,
    pub(crate) document: Document,
    files: Files,
    file_actions: FileActions,
    pub(crate) now: Duration,
    generation: u64,
    saved_state: u64,
    next: u64,
    startup: Option<PathBuf>,
    base_dir: PathBuf,
    clips: HashMap<RequestId, ClipTarget>,
    platform: Vec<PlatformCommand>,
    presentation: Presentation,
    find_open: bool,
    matches: Matches,
    pub(crate) notice: String,
}

impl EditorApp {
    pub(crate) fn new(path: Option<PathBuf>) -> Result<Self, Error> {
        let mut system = TextSystem::new();
        system
            .register_font(
                include_bytes!("../../../crates/oxiview-text/tests/fonts/NotoSans.ttf").to_vec(),
            )
            .map_err(error)?;
        Self::with_text(path, WELCOME, system)
    }
    pub(crate) fn with_text(
        path: Option<PathBuf>,
        initial: &str,
        system: TextSystem,
    ) -> Result<Self, Error> {
        let ui = Ui {
            text: Rc::new(RefCell::new(system)),
            source: Editable::new(initial, EditMode::MultiLine),
            query: Editable::new("", EditMode::SingleLine),
            replacement: Editable::new("", EditMode::SingleLine),
            slots: Slots::default(),
            commands: Commands::default(),
        };
        let mut tree = UiTree::new()?;
        tree.set_style(tree.root(), view::shell_style())?;
        tree.mount(tree.root(), view::screen(ui.clone()))?;
        ui.commands
            .register(&mut tree, &ui.source, [&ui.query, &ui.replacement])?;
        let id = ui
            .source
            .node()
            .ok_or_else(|| error("Source did not mount"))?;
        tree.set_focus_return_target(ui.slots.guard.get()?, Some(id))?;
        tree.set_focus_return_target(ui.slots.find.get()?, Some(id))?;
        tree.focus(id)?;
        Ok(Self {
            tree,
            ui,
            preview: PreviewPane::new().map_err(error)?,
            viewport: Size::new(1120., 720.),
            physical_size: PhysicalSize {
                width: 1120,
                height: 720,
            },
            document: Document::default(),
            files: Files::new().map_err(error)?,
            file_actions: FileActions::default(),
            now: Duration::ZERO,
            generation: 0,
            saved_state: 0,
            next: 0,
            startup: path,
            base_dir: fixture_dir(),
            clips: HashMap::new(),
            platform: Vec::new(),
            presentation: Presentation::default(),
            find_open: false,
            matches: Matches::default(),
            notice: "Ready".into(),
        })
    }
    fn token(&mut self) -> Result<u64, Error> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| error("Request IDs exhausted"))?;
        Ok(self.next)
    }
    fn source(&self) -> Rc<RefCell<Editor>> {
        self.ui.source.editor()
    }
    fn focus(&mut self, field: &Editable) -> Result<(), Error> {
        self.tree.focus(
            field
                .node()
                .ok_or_else(|| error("Editor has no live view"))?,
        )?;
        Ok(())
    }
    fn focus_source(&mut self) -> Result<(), Error> {
        self.focus(&self.ui.source.clone())
    }
    fn refresh(&mut self, field: &Editable) -> Result<(), Error> {
        if let Some(id) = field.node() {
            self.tree.refresh_editor(id)?;
        }
        Ok(())
    }
    fn field(&self) -> Option<Editable> {
        let id = self.tree.focused_node()?;
        [&self.ui.source, &self.ui.query, &self.ui.replacement]
            .into_iter()
            .find(|e| e.node() == Some(id))
            .cloned()
    }
    pub(crate) fn dirty(&self) -> bool {
        self.source().borrow().document_state() != self.saved_state
    }

    fn set_viewport(&mut self, size: PhysicalSize, scale: f64) {
        self.physical_size = size;
        self.viewport = Size::new(
            f64::from(size.width) / scale,
            f64::from(size.height) / scale,
        );
    }

    fn native_input(&mut self, input: &TextInputEvent) {
        let Some(field) = self.field() else {
            return;
        };
        let model = field.editor();
        let editor = model.borrow();
        if editor
            .check_native(input.session.0, input.revision, input.sequence)
            .is_err()
        {
            return;
        }
        let submit = matches!(
            input.edit,
            TextInputEdit::Command(TextEditingCommand::InsertNewline)
        ) && editor.mode() == EditMode::SingleLine
            && editor.presentation().marked.is_none();
        drop(editor);
        match self
            .tree
            .apply_text_input_event(input.clone(), self.viewport)
        {
            Ok(_) if submit => self.ui.commands.push(Command::Next),
            Ok(_) => {}
            Err(e) => self.notice = format!("Input could not be applied: {e}"),
        }
    }

    fn wheel(&mut self, at: Point, delta: Point) -> Result<bool, Error> {
        let preview = self.tree.bounds(self.ui.slots.preview.get()?)?;
        if contains(preview, at) {
            return Ok(self.preview.scroll(delta.y(), preview.size().height()));
        }
        if let Some(id) = self.ui.source.node()
            && contains(self.tree.bounds(id)?, at)
            && self.tree.scroll_editor(id, delta)?
        {
            self.presentation.source_scrolled();
            return Ok(true);
        }
        Ok(false)
    }
}

fn contains(rect: Rect, point: Point) -> bool {
    let [l, t, r, b] = rect.edges();
    point.x() >= l && point.x() < r && point.y() >= t && point.y() < b
}

impl Application for EditorApp {
    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        waker: &WindowWaker,
    ) -> Result<(), Error> {
        if matches!(event, WindowEvent::Destroyed) {
            scheduler.cancel_timed_request();
            self.tree.dispatch(event, self.viewport, |_, _, _| {})?;
            return Ok(());
        }
        if let Some(path) = self.startup.take() {
            self.open_startup(path, waker)?;
        }
        let mut app_changed = false;
        match event {
            WindowEvent::Resized {
                physical_size,
                scale_factor,
                predicted_presentation_timestamp,
                ..
            } => {
                self.set_viewport(*physical_size, *scale_factor);
                self.now = self.now.max(*predicted_presentation_timestamp);
            }
            WindowEvent::ScaleFactorChanged { scale_factor } => {
                self.set_viewport(self.physical_size, *scale_factor);
            }
            WindowEvent::Frame {
                predicted_presentation_timestamp,
                ..
            } => self.now = self.now.max(*predicted_presentation_timestamp),
            WindowEvent::Key { timestamp, .. }
            | WindowEvent::PointerButton { timestamp, .. }
            | WindowEvent::PointerMoved { timestamp, .. }
            | WindowEvent::PointerLeft { timestamp }
            | WindowEvent::PointerInterrupted { timestamp }
            | WindowEvent::ActivationChanged { timestamp, .. }
            | WindowEvent::VisibilityChanged { timestamp, .. } => {
                self.now = self.now.max(*timestamp)
            }
            WindowEvent::TextInput(input) => {
                self.now = self.now.max(input.timestamp);
                self.native_input(input);
            }
            WindowEvent::Wheel {
                position,
                delta_x,
                delta_y,
                timestamp,
            } => {
                self.now = self.now.max(*timestamp);
                self.tree.layout(self.viewport)?;
                if self.wheel(*position, Point::new(*delta_x, *delta_y))? {
                    scheduler.invalidate()
                }
            }
            WindowEvent::Clipboard { id, result } => self.finish_clip(*id, result)?,
            WindowEvent::FilePanel { id, result } => self.finish_panel(*id, result, waker)?,
            WindowEvent::CloseRequested => self.ui.commands.push(Command::Close),
            _ => {}
        }
        let routed = self.tree.dispatch(event, self.viewport, |_, _, _| {})?;
        if routed.changed {
            scheduler.invalidate()
        }
        while let Some(command) = self.ui.commands.pop() {
            self.perform(command, waker)?;
            app_changed = true;
        }
        self.sync(waker, scheduler, app_changed)
    }
    fn flush_platform(&mut self, commands: &mut Vec<PlatformCommand>) -> Result<(), Error> {
        self.publish_input(commands)?;
        commands.append(&mut self.platform);
        Ok(())
    }
    fn paint(
        &mut self,
        _timing: FrameTiming,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, Error> {
        self.set_viewport(size, scale);
        self.tree.layout(self.viewport)?;
        self.preview.update(&mut self.tree, &self.ui, scale)?;
        Ok(self.tree.paint(self.viewport, scale)?)
    }
    fn should_close(&mut self) -> bool {
        false
    }
}
