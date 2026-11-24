//! Window-thread controller shared by native presentation and headless replay.

mod input;
mod presentation;
#[cfg(test)]
mod tests;

use crate::{
    model::{Column, LogBook, Severity},
    source::SourceMonitor,
    view::{self, Ui},
};
use oxiview_base::{Point, Size};
use oxiview_core::{
    Application, EditError, EditorInputError, Error, FrameTiming, PaintedScene, Scheduler,
    UiPointerEvent, UiTree,
};
use oxiview_platform::{
    PhysicalSize, PlatformCommand, SystemPreferences, TextInputSnapshot, WindowEvent, WindowWaker,
};
use oxiview_style::Theme;
use oxiview_widgets::{
    collection::{VirtualList, VirtualTable},
    controls::{CONTROL_THEME, ScrollbarAxis},
};
use std::{cell::RefCell, collections::VecDeque, path::PathBuf, rc::Rc, time::Duration};

use input::{Desktop, HeadingDrag, RecordDrag};
use presentation::Presentation;

pub(super) fn error(error: impl std::fmt::Display) -> Error {
    Error::Application(error.to_string())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Mode {
    #[default]
    List,
    Table,
}

#[cfg(any(test, target_os = "macos"))]
pub(crate) struct PaintedListPosition {
    pub first_key: u64,
    pub offset_y: f64,
    pub row_top_y: f64,
    pub scene_fingerprint: u64,
}

#[derive(Clone, Copy)]
pub(crate) enum Intent {
    Open,
    Copy,
    CopyShortcut,
    CutSearch,
    PasteSearch,
    FocusSearch,
    Severity(Severity, bool),
    Follow(bool),
    Mode(Mode),
    Scrollbar(ScrollbarAxis, f64),
    Heading(UiPointerEvent),
    RecordPointer(UiPointerEvent),
    Navigate(input::Navigation, bool),
    Sort(Column),
    ActivateRecord,
}

#[derive(Clone, Default)]
pub(crate) struct Intents(Rc<RefCell<VecDeque<Intent>>>);

impl Intents {
    pub(crate) fn push(&self, intent: Intent) {
        self.0.borrow_mut().push_back(intent);
    }

    fn pop(&self) -> Option<Intent> {
        self.0.borrow_mut().pop_front()
    }
}

/// Persistent state and framework handles for the native log viewer.
pub(crate) struct ViewerApp {
    pub(crate) tree: UiTree,
    pub(crate) ui: Ui,
    pub(crate) book: LogBook,
    pub(crate) viewport: Size,
    pub(crate) list: VirtualList,
    pub(crate) table: VirtualTable,
    mode: Mode,
    list_key: Option<u64>,
    expanded: Option<u64>,
    source: SourceMonitor,
    startup: Option<PathBuf>,
    intents: Intents,
    desktop: Desktop,
    display: Presentation,
    heading_drag: Option<HeadingDrag>,
    record_drag: Option<RecordDrag>,
    physical_size: PhysicalSize,
    now: Duration,
    published_input: Option<TextInputSnapshot>,
    search_revision: u64,
    notice: String,
}

impl ViewerApp {
    /// Builds the acceptance app and defers a startup file open to the first event.
    pub(crate) fn new(startup: Option<PathBuf>) -> Result<Self, Error> {
        let mut text = oxiview_core::TextSystem::new();
        text.register_font(
            include_bytes!("../../../crates/oxiview-text/tests/fonts/NotoSans.ttf").to_vec(),
        )
        .map_err(error)?;
        Self::with_text(startup, text)
    }

    /// Builds the same viewer with caller-provided fonts for deterministic rendering.
    pub(crate) fn with_text(
        startup: Option<PathBuf>,
        text: oxiview_core::TextSystem,
    ) -> Result<Self, Error> {
        let intents = Intents::default();
        let ui = Ui::new(text, intents.clone());
        let mut tree = UiTree::new()?;
        tree.set_theme(Theme::from_toml(CONTROL_THEME).map_err(error)?)?;
        tree.set_style(tree.root(), view::shell_style())?;
        tree.mount(tree.root(), view::screen(ui.clone(), ui.inputs()?))?;
        input::register(&mut tree, &ui, &intents)?;
        let list = VirtualList::new(view::LIST_HEIGHT, 3).map_err(error)?;
        let mut table =
            VirtualTable::new(view::TABLE_HEIGHT, view::HEADER_HEIGHT, 3, 1).map_err(error)?;
        table
            .set_columns(
                Column::ALL
                    .into_iter()
                    .zip([71., 113., 81., 164., 628., 124., 86.])
                    .map(|(column, width)| oxiview_widgets::collection::TableColumn {
                        key: column as u64,
                        width,
                    }),
            )
            .map_err(error)?;
        let mut app = Self {
            tree,
            ui,
            book: LogBook::sample(),
            viewport: Size::new(1120., 720.),
            list,
            table,
            mode: Mode::List,
            list_key: None,
            expanded: None,
            source: SourceMonitor::default(),
            startup,
            intents,
            desktop: Desktop::default(),
            display: Presentation::default(),
            heading_drag: None,
            record_drag: None,
            physical_size: PhysicalSize {
                width: 1120,
                height: 720,
            },
            now: Duration::ZERO,
            published_input: None,
            search_revision: 0,
            notice: "Ready".into(),
        };
        app.sync()?;
        Ok(app)
    }

    /// Routes the measurement's distance through the same viewport as a discrete wheel step.
    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn scroll_sample(&mut self, delta_y: f64) -> Result<(), Error> {
        self.sync()?;
        let body = self.ui.collection.get()?;
        let bounds = self.tree.bounds(body)?;
        let origin = bounds.origin();
        let position = Point::new(
            origin.x() + bounds.size().width() * 0.5,
            origin.y() + bounds.size().height() * 0.5,
        );
        let before = self.tree.scroll_offset(body)?;
        self.tree.dispatch(
            &WindowEvent::Wheel {
                position,
                delta_x: 0.,
                delta_y,
                timestamp: self.now,
            },
            self.viewport,
            |_, _, _| {},
        )?;
        self.pause_if_scrolled_back(before)?;
        self.sync()?;
        Ok(())
    }

    /// Returns the first logical row intersecting the current viewport.
    pub(crate) fn first_visible_key(&self) -> Option<u64> {
        let body = self.ui.collection.get().ok()?;
        if self.tree.bounds(body).ok()?.size().height() <= 0. {
            return None;
        }
        let offset = self.tree.scroll_offset(body).ok()?.y();
        match self.mode {
            Mode::List => self.list.key_at(self.list.index_at(offset)?),
            Mode::Table => self.table.row_key(self.table.row_at(offset)?),
        }
    }

    /// Reads the mounted row and viewport backgrounds from the scene sent to the renderer.
    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn painted_list_position(
        &self,
        scene: &PaintedScene,
    ) -> Result<PaintedListPosition, Error> {
        use oxiview_scene::{NodeKind, Primitive};
        use std::hash::{DefaultHasher, Hash, Hasher};
        let key = self
            .first_visible_key()
            .ok_or_else(|| error("No visible list row"))?;
        let row = self
            .list
            .row_node(key)
            .ok_or_else(|| error("First visible row was not mounted"))?;
        let body = self.ui.collection.get()?;
        let top = |node| {
            self.tree
                .painted_background_bounds(scene, node)
                .map(|rect| rect.origin().y())
                .ok_or_else(|| error("The submitted scene is missing a measured background"))
        };
        // The following row is fully visible even when the leading row is
        // clipped. Its line-number caption contains actual positioned glyphs.
        let following = self
            .list
            .index_at(self.tree.scroll_offset(body)?.y())
            .and_then(|index| self.list.key_at(index + 1))
            .and_then(|next| self.list.row_node(next))
            .ok_or_else(|| error("No fully visible row to inspect"))?;
        let caption = self
            .tree
            .children(following)?
            .get(1)
            .and_then(|id| self.tree.painted_content_node(*id))
            .ok_or_else(|| error("The rendered row has no line-number drawing"))?;
        let NodeKind::Transform(transform) = scene.scene.content(caption).map_err(error)? else {
            return Err(error("The painted line-number scope lost its transform"));
        };
        let mut signature = DefaultHasher::new();
        transform
            .coefficients()
            .map(f64::to_bits)
            .hash(&mut signature);
        let glyphs = scene.scene.children(Some(caption)).map_err(error)?;
        if glyphs.is_empty() {
            return Err(error("The painted line-number caption is empty"));
        }
        for &id in glyphs {
            match scene.scene.content(id).map_err(error)? {
                NodeKind::Draw(Primitive::Image(rect, image)) => {
                    rect.edges().map(f64::to_bits).hash(&mut signature);
                    image.width().hash(&mut signature);
                    image.height().hash(&mut signature);
                    image.pixels().hash(&mut signature);
                }
                NodeKind::Draw(Primitive::Glyph(glyph)) => {
                    glyph.glyph_id.hash(&mut signature);
                    glyph.face_index.hash(&mut signature);
                    glyph.size.to_bits().hash(&mut signature);
                    [glyph.origin.x(), glyph.origin.y()]
                        .map(f64::to_bits)
                        .hash(&mut signature);
                }
                _ => return Err(error("Unexpected line-number drawing in the scroll sample")),
            }
        }
        Ok(PaintedListPosition {
            first_key: key,
            offset_y: self.tree.scroll_offset(body)?.y(),
            row_top_y: top(row)? - top(body)?,
            scene_fingerprint: signature.finish(),
        })
    }

    /// Returns the stable key of the selected record or active table cell.
    pub(crate) fn active_key(&self) -> Option<u64> {
        match self.mode {
            Mode::List => self.list_key,
            Mode::Table => self.table.selection().map(|selection| selection.active.row),
        }
    }

    /// Reports the active presentation for replay and measurement.
    pub(crate) fn is_table(&self) -> bool {
        self.mode == Mode::Table
    }

    fn set_viewport(&mut self, physical_size: PhysicalSize, scale: f64) {
        self.physical_size = physical_size;
        self.viewport = Size::new(
            f64::from(physical_size.width) / scale,
            f64::from(physical_size.height) / scale,
        );
    }

    fn drain(&mut self) {
        if let Some(notice) = self.source.drain(&mut self.book).pop() {
            self.notice = notice;
        }
    }

    fn pause_if_scrolled_back(&mut self, before: Point) -> Result<(), Error> {
        let after = self.tree.scroll_offset(self.ui.collection.get()?)?;
        if after.y() < before.y() {
            self.book.pause_follow();
        }
        Ok(())
    }

    fn sync_search(&mut self) -> Result<(), Error> {
        let model = self.ui.search.editor();
        let editor = model.borrow();
        let revision = editor.committed_revision();
        if revision != self.search_revision {
            self.display.sort_or_filter |= self.book.set_query(editor.source());
            self.search_revision = revision;
            drop(editor);
            self.ui.search.sync_value(&mut self.tree)?;
        }
        Ok(())
    }
}

impl Application for ViewerApp {
    fn initialize_preferences(&mut self, preferences: SystemPreferences) -> Result<(), Error> {
        self.tree.set_system_preferences(preferences)?;
        Ok(())
    }

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
            self.open_path(path, waker);
        }
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
            } => {
                self.now = self.now.max(*predicted_presentation_timestamp);
            }
            WindowEvent::Key { timestamp, .. }
            | WindowEvent::PointerButton { timestamp, .. }
            | WindowEvent::PointerMoved { timestamp, .. }
            | WindowEvent::PointerLeft { timestamp }
            | WindowEvent::PointerInterrupted { timestamp }
            | WindowEvent::ActivationChanged { timestamp, .. }
            | WindowEvent::VisibilityChanged { timestamp, .. }
            | WindowEvent::Wheel { timestamp, .. }
            | WindowEvent::Scroll { timestamp, .. } => self.now = self.now.max(*timestamp),
            WindowEvent::TextInput(input) => {
                self.now = self.now.max(input.timestamp);
                match self
                    .tree
                    .apply_text_input_event(input.clone(), self.viewport)
                {
                    Ok(_) | Err(EditorInputError::Edit(EditError::StaleInput)) => {}
                    Err(reason) => self.notice = format!("Input could not be applied: {reason}"),
                }
            }
            WindowEvent::Clipboard { id, result } => self.finish_clipboard(*id, result)?,
            WindowEvent::FilePanel { id, result } => self.finish_panel(*id, result, waker),
            _ => {}
        }
        self.drain();
        self.sync_search()?;
        let mut changed = self.sync()?;
        let body = self.ui.collection.get()?;
        let before = self.tree.scroll_offset(body)?;
        changed |= self
            .tree
            .dispatch(event, self.viewport, |_, _, _| {})?
            .changed;
        if matches!(event, WindowEvent::Frame { .. }) {
            changed |= self.advance_drag()?;
        }
        self.pause_if_scrolled_back(before)?;
        while let Some(intent) = self.intents.pop() {
            changed |= self.perform(intent)?;
        }
        self.sync_search()?;
        changed |= self.sync()?;
        let (caret_changed, caret_deadline) = self.tree.advance_editor_caret(self.now)?;
        if changed || caret_changed {
            scheduler.invalidate();
        }
        scheduler.cancel_timed_request();
        if let Some(deadline) = caret_deadline {
            scheduler.request_at(deadline);
        }
        scheduler.set_scroll_animation_active(self.tree.scroll_animation_active());
        if self.drag_can_scroll()? {
            let interval = match event {
                WindowEvent::Frame {
                    refresh_interval, ..
                } => *refresh_interval,
                _ => Duration::from_millis(16),
            };
            scheduler.request_at(self.now.saturating_add(interval));
        }
        Ok(())
    }

    fn flush_platform(&mut self, commands: &mut Vec<PlatformCommand>) -> Result<(), Error> {
        let input = self.tree.focused_text_input_snapshot(self.viewport)?;
        if input != self.published_input {
            commands.push(PlatformCommand::TextInput(input.clone()));
            self.published_input = input;
        }
        commands.append(&mut self.desktop.commands);
        Ok(())
    }

    fn paint(
        &mut self,
        _timing: FrameTiming,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, Error> {
        self.set_viewport(size, scale);
        self.drain();
        self.sync()?;
        Ok(self.tree.paint(self.viewport, scale)?)
    }
}
