//! Observes the editor from its window thread and drives the headless platform.

use super::Result;
use crate::{
    app::{EditorApp, error},
    preview::{CompileStats, Diagnostic},
};
use oxiview_app::{HeadlessOutput, run_headless};
use oxiview_base::{Point, Rect, RgbaImage, Size};
use oxiview_core::{Application, Error, FrameSubmission, FrameTiming, PaintedScene, Scheduler};
use oxiview_platform::{
    KeyCode, KeyModifiers, PhysicalSize, PlatformCommand, PointerButton, ShortcutPlatform,
    SystemPreferences, TextInputEdit, TextInputEvent, TextInputSnapshot, WindowAttributes,
    WindowEvent, WindowId, WindowWaker,
};
use oxiview_platform_headless::{Driver, HeadlessPlatform, KeyTransition, ResizeOutcome, WaitKind};
use std::{
    cell::RefCell,
    collections::HashMap,
    ops::Range,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

const INTERVAL: Duration = Duration::from_millis(16);
const TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(2);
pub(super) const WIDTH: u32 = 1120;
pub(super) const HEIGHT: u32 = 720;
pub(super) const IDLE_OPPORTUNITIES: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Focus {
    Source,
    Query,
    Replacement,
    Other,
}

#[derive(Clone, Debug)]
pub(super) struct State {
    pub published: u64,
    pub submitted: u64,
    pub stats: CompileStats,
    pub diagnostics: Vec<Diagnostic>,
    pub source: String,
    pub source_selection: Range<usize>,
    pub source_marked: Option<Range<usize>>,
    pub query: String,
    pub replacement: String,
    pub dirty: bool,
    pub file: Option<PathBuf>,
    pub notice: String,
    pub focus: Focus,
    pub source_bounds: Rect,
    pub preview: Rect,
    buttons: HashMap<String, Point>,
}

impl State {
    fn observe(app: &EditorApp) -> std::result::Result<Self, Error> {
        let focus = app.tree.focused_node();
        let source = app
            .ui
            .source
            .node()
            .ok_or_else(|| error("Replay source is unmounted"))?;
        let focus = match focus {
            Some(node) if node == source => Focus::Source,
            Some(node) if Some(node) == app.ui.query.node() => Focus::Query,
            Some(node) if Some(node) == app.ui.replacement.node() => Focus::Replacement,
            _ => Focus::Other,
        };
        let mut buttons = HashMap::new();
        let mut pending = vec![app.tree.root()];
        while let Some(node) = pending.pop() {
            pending.extend(app.tree.children(node)?.iter().copied());
            if let Some(semantics) = app.tree.semantics(node)?
                && semantics.role == "button"
            {
                let bounds = app.tree.bounds(node)?;
                let point = center(bounds);
                if bounds.size().width() > 0.
                    && bounds.size().height() > 0.
                    && app.tree.hit_test(point)? == Some(node)
                {
                    buttons.insert(semantics.label.clone(), point);
                }
            }
        }
        let model = app.ui.source.editor();
        let editor = model.borrow();
        Ok(Self {
            published: app.preview.published,
            submitted: app.preview.submitted,
            stats: app.preview.stats,
            diagnostics: app.preview.diagnostic_details().to_vec(),
            source: editor.source().into(),
            source_selection: editor.selection().range(),
            source_marked: editor.presentation().marked,
            query: app.ui.query.editor().borrow().source().into(),
            replacement: app.ui.replacement.editor().borrow().source().into(),
            dirty: app.dirty(),
            file: app.document.path.clone(),
            notice: app.notice.clone(),
            focus,
            source_bounds: app.tree.bounds(source)?,
            preview: app.tree.bounds(app.ui.slots.preview.get()?)?,
            buttons,
        })
    }

    pub fn current_preview(&self) -> bool {
        self.published > 0 && self.published == self.submitted
    }
}

pub(super) fn center(rect: Rect) -> Point {
    let [left, top, right, bottom] = rect.edges();
    Point::new((left + right) * 0.5, (top + bottom) * 0.5)
}

#[derive(Default)]
struct Observation {
    editor: Option<State>,
    redraw_requested: u64,
    redraw_presented: u64,
}

struct Observed {
    app: EditorApp,
    observation: Arc<Mutex<Observation>>,
    force_close: Arc<AtomicBool>,
    idle_barrier: Sender<()>,
}

impl Application for Observed {
    fn initialize_preferences(
        &mut self,
        preferences: SystemPreferences,
    ) -> std::result::Result<(), Error> {
        self.app.initialize_preferences(preferences)
    }

    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        waker: &WindowWaker,
    ) -> std::result::Result<(), Error> {
        if self.force_close.load(Ordering::Acquire) {
            return Ok(());
        }
        if matches!(event, WindowEvent::PreferencesChanged { .. }) {
            // The replay reserves this event as a FIFO marker. Leaving the app
            // and scheduler untouched allows the idle probe to detect deadlines.
            let _ = self.idle_barrier.send(());
            return Ok(());
        }
        self.app.event(event, scheduler, waker)?;
        if matches!(event, WindowEvent::Destroyed) {
            return Ok(());
        }
        let editor = State::observe(&self.app)?;
        let mut observation = self.observation.lock().map_err(error)?;
        observation.editor = Some(editor);
        if matches!(event, WindowEvent::RedrawRequested) {
            observation.redraw_requested += 1;
        }
        Ok(())
    }

    fn flush_platform(
        &mut self,
        commands: &mut Vec<PlatformCommand>,
    ) -> std::result::Result<(), Error> {
        if self.force_close.load(Ordering::Acquire) {
            return Ok(());
        }
        self.app.flush_platform(commands)
    }

    fn paint(
        &mut self,
        timing: FrameTiming,
        size: PhysicalSize,
        scale: f64,
    ) -> std::result::Result<PaintedScene, Error> {
        self.app.paint(timing, size, scale)
    }

    fn frame_submitted(&mut self, frame: FrameSubmission) {
        self.app.frame_submitted(frame);
        if let Ok(mut observation) = self.observation.lock() {
            observation.redraw_presented = observation.redraw_requested;
        }
    }

    fn should_close(&mut self) -> bool {
        self.force_close.load(Ordering::Acquire) || self.app.should_close()
    }
}

struct Endpoint {
    driver: Driver,
    id: WindowId,
    output: HeadlessOutput,
}

enum Lifecycle {
    Ready(Endpoint),
    Finished(Result<()>),
}

pub(super) struct Window {
    pub driver: Driver,
    pub id: WindowId,
    output: HeadlessOutput,
    observation: Arc<Mutex<Observation>>,
    force_close: Arc<AtomicBool>,
    idle_barriers: Option<Receiver<()>>,
    lifecycle: Receiver<Lifecycle>,
    completion: RefCell<Option<Result<()>>>,
}

impl Window {
    fn running(&self) -> Result<()> {
        match self.lifecycle.try_recv() {
            Ok(Lifecycle::Finished(result)) => {
                *self.completion.borrow_mut() = Some(result);
                Err("Window exited during the replay".into())
            }
            Ok(Lifecycle::Ready(_)) => Err("Window reported readiness twice".into()),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Window stopped reporting its lifecycle".into())
            }
            Err(mpsc::TryRecvError::Empty) => Ok(()),
        }
    }

    pub fn until<T>(&self, label: &str, mut test: impl FnMut() -> Result<Option<T>>) -> Result<T> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            self.running()?;
            if let Some(value) = test()? {
                return Ok(value);
            }
            if Instant::now() >= deadline {
                return Err(format!("Timed out waiting for {label}").into());
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    pub fn state(&self, label: &str, predicate: impl Fn(&State) -> bool) -> Result<State> {
        self.until(label, || {
            let observation = self
                .observation
                .lock()
                .map_err(|_| "Replay observation lock failed")?;
            Ok(observation
                .editor
                .as_ref()
                .filter(|state| predicate(state))
                .cloned())
        })
        .map_err(|error| match self.observation.lock() {
            Ok(observation) => {
                if let Some(state) = &observation.editor {
                    format!(
                        "{error}; last focus {:?}, selection {:?}, preview {}/{}, notice {:?}",
                        state.focus,
                        state.source_selection,
                        state.published,
                        state.submitted,
                        state.notice
                    )
                    .into()
                } else {
                    error
                }
            }
            Err(_) => error,
        })
    }

    pub fn snapshot(
        &self,
        label: &str,
        predicate: impl Fn(&TextInputSnapshot) -> bool,
    ) -> Result<TextInputSnapshot> {
        self.until(label, || {
            Ok(self
                .driver
                .text_input_snapshot(self.id)?
                .filter(|snapshot| predicate(snapshot)))
        })
    }

    pub fn key(&self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
        for pressed in [true, false] {
            self.driver.key_transition(
                self.id,
                KeyTransition {
                    code,
                    logical: code.us_logical_key(),
                    modifiers,
                    pressed,
                    repeat: false,
                    timestamp: None,
                },
            )?;
        }
        Ok(())
    }

    pub fn shortcut(&self, code: KeyCode, shift: bool) -> Result<()> {
        self.key(
            code,
            KeyModifiers {
                shift,
                ..ShortcutPlatform::current().primary_modifier()
            },
        )
    }

    pub fn click(&self, label: &str) -> Result<()> {
        let state = self.state(label, |state| state.buttons.contains_key(label))?;
        let point = *state
            .buttons
            .get(label)
            .ok_or("Expected a visible button")?;
        for pressed in [true, false] {
            self.driver
                .pointer_button(self.id, PointerButton::Primary, pressed, point, None)?;
        }
        Ok(())
    }

    pub fn drag(&self, start: Point, end: Point) -> Result<()> {
        self.driver
            .pointer_button(self.id, PointerButton::Primary, true, start, None)?;
        self.driver.pointer_moved(self.id, end, None)?;
        self.driver
            .pointer_button(self.id, PointerButton::Primary, false, end, None)?;
        Ok(())
    }

    pub fn input(&self, snapshot: &TextInputSnapshot, edit: TextInputEdit) -> Result<()> {
        self.driver.text_input(
            self.id,
            TextInputEvent {
                session: snapshot.session,
                revision: snapshot.revision,
                sequence: snapshot.acknowledged + 1,
                timestamp: self.driver.now()?,
                edit,
            },
        )?;
        Ok(())
    }

    pub fn resize(&self, width: u32, height: u32, scale: f64) -> Result<RgbaImage> {
        let token = self
            .driver
            .resize(self.id, PhysicalSize { width, height }, scale)?;
        let outcome = self.until("resize acknowledgement", || {
            Ok(self.driver.resize_outcome(self.id, token)?)
        })?;
        if outcome != ResizeOutcome::Acknowledged {
            return Err(format!("Replay resize was {outcome:?}").into());
        }
        let image = self
            .output
            .snapshot()?
            .image
            .ok_or("Acknowledged resize did not produce a CPU image")?;
        if image.width() != width || image.height() != height {
            return Err(format!("Resize did not present at {width}×{height}").into());
        }
        Ok(image)
    }

    pub fn capture(&self, name: &str) -> Result<RgbaImage> {
        let target = self
            .observation
            .lock()
            .map_err(|_| "Replay observation lock failed")?
            .redraw_requested
            + 1;
        self.driver.request_redraw(self.id)?;
        self.until(name, || {
            let presented = self
                .observation
                .lock()
                .map_err(|_| "Replay observation lock failed")?
                .redraw_presented;
            if presented == target {
                return Ok(self.output.snapshot()?.image);
            }
            self.driver.advance_by(INTERVAL)?;
            Ok(None)
        })
    }

    pub fn idle_presentations(&mut self) -> Result<u64> {
        let barriers = self.idle_barriers.take().ok_or("Idle probe already ran")?;
        let driver = self.driver.clone();
        let output = self.output.clone();
        let id = self.id;
        let (finished, result) = mpsc::channel();
        thread::Builder::new()
            .name("markdown-replay-idle".into())
            .spawn(move || {
                let run = || -> Result<u64> {
                    let settle = || -> Result<()> {
                        driver.preferences_changed(id, SystemPreferences::default())?;
                        barriers.recv_timeout(TIMEOUT)?;
                        // Once the marker ran, a wait can only be registered after
                        // all preceding events and elapsed deadlines were handled.
                        driver.wait_until_blocked(id, WaitKind::Event)?;
                        Ok(())
                    };
                    settle()?;
                    let before = output.snapshot()?.presents;
                    for _ in 0..IDLE_OPPORTUNITIES {
                        driver.advance_by(INTERVAL)?;
                        settle()?;
                    }
                    output
                        .snapshot()?
                        .presents
                        .checked_sub(before)
                        .ok_or_else(|| "CPU presentation counter went backwards".into())
                };
                let _ = finished.send(run());
            })?;
        // Supervise the driver's blocking handshake. Forced close releases a
        // blocked driver as well as the window endpoint on a replay failure.
        match result.recv_timeout(TIMEOUT) {
            Ok(result) => result,
            Err(error) => Err(format!("Idle probe did not finish: {error}").into()),
        }
    }

    fn abort(&self) {
        self.force_close.store(true, Ordering::Release);
        let _ = self.driver.request_user_close(self.id);
    }

    fn wait_for_exit(&self) -> Result<Result<()>> {
        if let Some(result) = self.completion.borrow_mut().take() {
            return Ok(result);
        }
        match self.lifecycle.recv_timeout(TIMEOUT) {
            Ok(Lifecycle::Finished(result)) => Ok(result),
            Ok(Lifecycle::Ready(_)) => Err("Window reported readiness twice".into()),
            Err(error) => Err(format!("Replay window did not report shutdown: {error}").into()),
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        self.abort();
    }
}

pub(super) fn with_window<T>(
    create_app: impl FnOnce() -> std::result::Result<EditorApp, Error> + Send + 'static,
    scenario: impl FnOnce(&mut Window) -> Result<T>,
) -> Result<T> {
    let observation = Arc::new(Mutex::new(Observation::default()));
    let force_close = Arc::new(AtomicBool::new(false));
    let (status, lifecycle) = mpsc::channel();
    let (accept_ready, ready_accepted) = mpsc::sync_channel(1);
    let (idle_barrier, idle_barriers) = mpsc::channel();
    let run_observation = observation.clone();
    let run_force_close = force_close.clone();
    thread::Builder::new()
        .name("markdown-replay-platform".into())
        .spawn(move || {
            let run = || -> Result<()> {
                let (backend, driver) = HeadlessPlatform::new(INTERVAL)?;
                run_headless(
                    backend,
                    WindowAttributes {
                        title: "Markdown replay".into(),
                        logical_size: Size::new(f64::from(WIDTH), f64::from(HEIGHT)),
                        resize_timeout: Duration::from_millis(500),
                    },
                    move || {
                        Ok(Observed {
                            app: create_app()?,
                            observation: run_observation,
                            force_close: run_force_close,
                            idle_barrier,
                        })
                    },
                    |_, id, output| {
                        status
                            .send(Lifecycle::Ready(Endpoint { driver, id, output }))
                            .map_err(|_| oxiview_platform::Error::EventChannelClosed)?;
                        // If startup times out on the driver, returning an error
                        // here makes the backend destroy its newly opened window.
                        ready_accepted
                            .recv_timeout(TIMEOUT)
                            .map_err(|_| oxiview_platform::Error::EventChannelClosed)
                    },
                )?;
                Ok(())
            };
            let _ = status.send(Lifecycle::Finished(run()));
        })?;
    let Endpoint { driver, id, output } = match lifecycle.recv_timeout(TIMEOUT) {
        Ok(Lifecycle::Ready(endpoint)) => endpoint,
        Ok(Lifecycle::Finished(result)) => {
            result?;
            return Err("Window stopped before becoming ready".into());
        }
        Err(error) => return Err(format!("Replay window did not become ready: {error}").into()),
    };
    let mut window = Window {
        driver,
        id,
        output,
        observation,
        force_close,
        idle_barriers: Some(idle_barriers),
        lifecycle,
        completion: RefCell::new(None),
    };
    let result = accept_ready
        .send(())
        .map_err(|_| "Replay window stopped accepting initialization".into())
        .and_then(|()| scenario(&mut window))
        .and_then(|report| {
            window.driver.request_user_close(window.id)?;
            Ok(report)
        });
    if result.is_err() {
        window.abort();
    }
    match window.wait_for_exit() {
        Ok(exit) => {
            // The driver's window-destroyed error can precede the actual
            // composition error. Prefer the cause reported by the window.
            exit?;
            result
        }
        Err(shutdown) => {
            window.abort();
            if let Ok(exit) = window.wait_for_exit() {
                exit?;
            }
            match result {
                Ok(_) => Err(shutdown),
                Err(cause) => Err(format!("{cause}; shutdown also failed: {shutdown}").into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_surfaces_the_application_factory_error() {
        let result = with_window(
            || Err(error("replay fixture could not initialize")),
            |window| window.state("initial state", |_| true),
        );
        assert!(
            matches!(result, Err(error) if error.to_string().contains("replay fixture could not initialize"))
        );
    }
}
