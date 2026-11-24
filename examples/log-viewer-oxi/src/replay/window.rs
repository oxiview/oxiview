//! Drives the same headless event loop and CPU presentation path as the M6 replay.

use super::{Result, check};
use crate::app::ViewerApp;
use oxiview_app::{HeadlessOutput, run_headless};
use oxiview_base::{Point, Rect, RgbaImage, Size};
use oxiview_core::{
    Application, Error, FrameSubmission, FrameTiming, NodeId, PaintedScene, Scheduler,
};
use oxiview_platform::{
    ClipboardRequest, ClipboardResponse, FilePanelKind, KeyCode, KeyModifiers, PhysicalSize,
    PlatformCommand, PointerButton, ScrollPhase, ShortcutPlatform, SystemPreferences,
    TextInputEdit, TextInputEvent, TextInputSnapshot, WindowAttributes, WindowEvent, WindowId,
    WindowWaker,
};
use oxiview_platform_headless::{Driver, HeadlessPlatform, KeyTransition, ResizeOutcome, WaitKind};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

const INTERVAL: Duration = Duration::from_millis(16);
const TIMEOUT: Duration = Duration::from_secs(12);

type Inspection = Box<dyn FnOnce(&ViewerApp) + Send>;

#[derive(Default)]
struct Frames {
    redraws: AtomicU64,
    completed_redraws: AtomicU64,
    submissions: AtomicU64,
}

struct Observed {
    app: ViewerApp,
    inspect: Receiver<Inspection>,
    frames: Arc<Frames>,
    abort: Arc<AtomicBool>,
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
        if self.abort.load(Ordering::Acquire) {
            return Ok(());
        }
        // The driver uses a reserved preferences event as a FIFO inspection or
        // idle barrier, leaving the viewer and its scheduler unmodified.
        if matches!(event, WindowEvent::PreferencesChanged { .. })
            && let Ok(inspect) = self.inspect.try_recv()
        {
            inspect(&self.app);
            return Ok(());
        }
        self.app.event(event, scheduler, waker)?;
        if matches!(event, WindowEvent::RedrawRequested) {
            self.frames.redraws.fetch_add(1, Ordering::Release);
        }
        Ok(())
    }

    fn flush_platform(
        &mut self,
        commands: &mut Vec<PlatformCommand>,
    ) -> std::result::Result<(), Error> {
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
        self.frames.submissions.fetch_add(1, Ordering::Release);
        self.frames.completed_redraws.store(
            self.frames.redraws.load(Ordering::Acquire),
            Ordering::Release,
        );
    }

    fn should_close(&mut self) -> bool {
        self.abort.load(Ordering::Acquire) || self.app.should_close()
    }
}

#[derive(Clone)]
pub(super) struct Window {
    driver: Driver,
    id: WindowId,
    output: HeadlessOutput,
    frames: Arc<Frames>,
    inspect: Sender<Inspection>,
}

impl Window {
    pub(super) fn inspect<T: Send + 'static>(
        &self,
        read: impl FnOnce(&ViewerApp) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (send, receive) = mpsc::sync_channel(1);
        self.inspect
            .send(Box::new(move |app| {
                let _ = send.send(read(app));
            }))
            .map_err(|_| "Replay window closed before inspection")?;
        self.driver
            .preferences_changed(self.id, SystemPreferences::default())?;
        receive.recv_timeout(TIMEOUT)?
    }

    pub(super) fn until<T>(
        &self,
        description: &str,
        mut attempt: impl FnMut() -> Result<Option<T>>,
    ) -> Result<T> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(value) = attempt()? {
                return Ok(value);
            }
            if Instant::now() >= deadline {
                return Err(format!("Timed out waiting for {description}").into());
            }
            thread::sleep(Duration::from_millis(3));
        }
    }

    pub(super) fn click(&self, point: Point) -> Result<()> {
        self.button(true, point)?;
        self.button(false, point)
    }

    pub(super) fn button(&self, pressed: bool, point: Point) -> Result<()> {
        self.driver
            .pointer_button(self.id, PointerButton::Primary, pressed, point, None)?;
        Ok(())
    }

    pub(super) fn drag(&self, from: Point, to: Point) -> Result<()> {
        self.button(true, from)?;
        self.driver.pointer_moved(self.id, to, None)?;
        self.button(false, to)
    }

    pub(super) fn control(
        &self,
        name: &str,
        node: impl FnOnce(&ViewerApp) -> Option<NodeId> + Send + 'static,
    ) -> Result<()> {
        let name = name.to_owned();
        let point = self.inspect(move |app| {
            let id = node(app).ok_or_else(|| format!("{name} is unmounted"))?;
            let point = center(app.tree.bounds(id)?);
            check(hit_within(app, point, id)?, || {
                format!("{name} is not hittable at {point:?}")
            })?;
            Ok(point)
        })?;
        self.click(point)
    }

    pub(super) fn key(&self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
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

    pub(super) fn shortcut(&self, code: KeyCode) -> Result<()> {
        self.key(code, ShortcutPlatform::current().primary_modifier())
    }

    pub(super) fn wheel(&self, point: Point, dx: f64, dy: f64) -> Result<()> {
        self.driver.wheel(self.id, point, dx, dy, None)?;
        Ok(())
    }

    pub(super) fn scroll(&self, point: Point, dy: f64, phase: ScrollPhase) -> Result<()> {
        self.driver.scroll(self.id, point, 0., dy, phase, None)?;
        Ok(())
    }

    pub(super) fn advance(&self) -> Result<()> {
        self.driver.advance_by(INTERVAL)?;
        self.inspect(|_| Ok(()))
    }

    pub(super) fn input_snapshot(
        &self,
        predicate: impl Fn(&TextInputSnapshot) -> bool,
    ) -> Result<TextInputSnapshot> {
        self.until("native text input snapshot", || {
            Ok(self.driver.text_input_snapshot(self.id)?.filter(&predicate))
        })
    }

    pub(super) fn edit(&self, snapshot: &TextInputSnapshot, edit: TextInputEdit) -> Result<()> {
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

    pub(super) fn copy(&self, trigger: impl FnOnce() -> Result<()>, expected: &str) -> Result<()> {
        let previous = self.driver.clipboard_requests(self.id)?.len();
        trigger()?;
        let request = self.until("clipboard write", || {
            Ok(self
                .driver
                .clipboard_requests(self.id)?
                .get(previous)
                .cloned())
        })?;
        let ClipboardRequest::Write { id, text } = request else {
            return Err(format!("Expected a clipboard write, got {request:?}").into());
        };
        check(text == expected, || {
            let position = text
                .bytes()
                .zip(expected.bytes())
                .take_while(|(a, b)| a == b)
                .count();
            let row = expected.as_bytes()[..position]
                .iter()
                .filter(|&&byte| byte == b'\n')
                .count()
                + 1;
            format!(
                "Clipboard differs at byte {position}, row {row} (actual {} bytes, expected {} bytes)",
                text.len(),
                expected.len()
            )
        })?;
        self.driver
            .clipboard_reply(self.id, id, Ok(ClipboardResponse::Written))?;
        self.inspect(|_| Ok(()))
    }

    pub(super) fn open(&self, result: std::result::Result<Option<PathBuf>, String>) -> Result<()> {
        let previous = self.driver.file_panel_requests(self.id)?.len();
        self.control("Open", |app| app.ui.open.node())?;
        let request = self.until("native Open panel request", || {
            Ok(self
                .driver
                .file_panel_requests(self.id)?
                .get(previous)
                .cloned())
        })?;
        check(request.kind == FilePanelKind::Open, || {
            format!("Wrong file panel kind: {:?}", request.kind)
        })?;
        self.driver.file_panel_reply(self.id, request.id, result)?;
        self.inspect(|_| Ok(()))
    }

    pub(super) fn resize(&self, width: u32, height: u32) -> Result<()> {
        let token = self
            .driver
            .resize(self.id, PhysicalSize { width, height }, 1.)?;
        let outcome = self.until("CPU resize acknowledgement", || {
            Ok(self.driver.resize_outcome(self.id, token)?)
        })?;
        check(outcome == ResizeOutcome::Acknowledged, || {
            format!("Resize failed: {outcome:?}")
        })?;
        let snapshot = self.output.snapshot()?;
        check(
            snapshot
                .image
                .is_some_and(|image| image.width() == width && image.height() == height),
            || format!("Resize did not present exactly {width}×{height} physical pixels"),
        )
    }

    pub(super) fn capture(&self) -> Result<RgbaImage> {
        self.inspect(|_| Ok(()))?;
        let target = self.frames.redraws.load(Ordering::Acquire) + 1;
        self.driver.request_redraw(self.id)?;
        self.until("CPU presentation of requested redraw", || {
            if self.frames.completed_redraws.load(Ordering::Acquire) >= target {
                return Ok(self.output.snapshot()?.image);
            }
            self.advance()?;
            Ok(None)
        })
    }

    pub(super) fn idle(&self, opportunities: usize) -> Result<(u64, u64)> {
        let window = self.clone();
        let (send, receive) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("log-replay-idle".into())
            .spawn(move || {
                let _ = send.send(window.measure_idle(opportunities));
            })?;
        receive.recv_timeout(TIMEOUT)?
    }

    fn measure_idle(&self, opportunities: usize) -> Result<(u64, u64)> {
        let settle = || -> Result<()> {
            self.inspect(|_| Ok(()))?;
            self.driver.wait_until_blocked(self.id, WaitKind::Event)?;
            Ok(())
        };
        settle()?;
        let submitted = self.frames.submissions.load(Ordering::Acquire);
        let presented = self.output.snapshot()?.presents;
        for _ in 0..opportunities {
            self.driver.advance_by(INTERVAL)?;
            settle()?;
        }
        Ok((
            self.frames.submissions.load(Ordering::Acquire) - submitted,
            self.output.snapshot()?.presents - presented,
        ))
    }
}

pub(super) fn center(rect: Rect) -> Point {
    let [left, top, right, bottom] = rect.edges();
    Point::new((left + right) * 0.5, (top + bottom) * 0.5)
}

pub(super) fn hit_within(app: &ViewerApp, point: Point, ancestor: NodeId) -> Result<bool> {
    let mut hit = app.tree.hit_test(point)?;
    while let Some(node) = hit {
        if node == ancestor {
            return Ok(true);
        }
        hit = app.tree.parent(node)?;
    }
    Ok(false)
}

pub(super) fn with_window<T>(scenario: impl FnOnce(&Window) -> Result<T>) -> Result<T> {
    let frames = Arc::new(Frames::default());
    let abort = Arc::new(AtomicBool::new(false));
    let (inspect, inspections) = mpsc::channel();
    let (ready, endpoint) = mpsc::sync_channel(1);
    let (accept, accepted) = mpsc::sync_channel(1);
    let (done, completion) = mpsc::sync_channel(1);
    let run_frames = frames.clone();
    let run_abort = abort.clone();
    let worker = thread::Builder::new()
        .name("log-replay-platform".into())
        .spawn(move || {
            let run = || -> Result<()> {
                let (backend, driver) = HeadlessPlatform::new(INTERVAL)?;
                run_headless(
                    backend,
                    WindowAttributes {
                        title: "Oxiview Logs replay".into(),
                        logical_size: Size::new(1120., 720.),
                        resize_timeout: Duration::from_millis(500),
                    },
                    move || {
                        Ok(Observed {
                            app: ViewerApp::new(None)?,
                            inspect: inspections,
                            frames: run_frames,
                            abort: run_abort,
                        })
                    },
                    |_, id, output| {
                        ready
                            .send((driver, id, output))
                            .map_err(|_| oxiview_platform::Error::EventChannelClosed)?;
                        accepted
                            .recv_timeout(TIMEOUT)
                            .map_err(|_| oxiview_platform::Error::EventChannelClosed)
                    },
                )?;
                Ok(())
            };
            let _ = done.send(run());
        })?;
    let (driver, id, output) = match endpoint.recv_timeout(TIMEOUT) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            drop(accept);
            completion.recv_timeout(TIMEOUT)??;
            return Err(format!("Headless window did not become ready: {error}").into());
        }
    };
    let window = Window {
        driver,
        id,
        output,
        frames,
        inspect,
    };
    let result = accept
        .send(())
        .map_err(|_| "Headless window stopped accepting initialization".into())
        .and_then(|()| {
            window.until("initial CPU presentation", || {
                Ok((window.frames.submissions.load(Ordering::Acquire) > 0).then_some(()))
            })?;
            scenario(&window)
        });
    if result.is_err() {
        abort.store(true, Ordering::Release);
    }
    let close = window.driver.request_user_close(window.id);
    let shutdown = completion.recv_timeout(TIMEOUT);
    match shutdown {
        Ok(exit) => {
            worker
                .join()
                .map_err(|_| "Headless platform thread panicked")?;
            exit?;
        }
        Err(error) => {
            abort.store(true, Ordering::Release);
            let _ = window.driver.request_user_close(window.id);
            return Err(format!(
                "Headless shutdown failed: {error}; replay error: {:?}",
                result.err()
            )
            .into());
        }
    }
    let report = result?;
    close?;
    Ok(report)
}
