//! Real headless window-loop checks for Snake scheduling, input, and text output.

use oxiview_app::{HeadlessOutput, run_headless};
use oxiview_base::Size;
use oxiview_core::{
    Application, FrameSubmission, FrameTiming, LatencyRecord, PaintedScene, Scheduler,
};
use oxiview_platform::{
    KeyCode, PhysicalSize, WindowAttributes, WindowEvent, WindowId, WindowWaker,
};
use oxiview_platform_headless::{Driver, HeadlessPlatform, ResizeOutcome, WaitKind};
use oxiview_scene::{EncodedNode, Primitive};
use snake::{
    application::{Config, Counters, SnakeApp},
    game::{Phase, Snapshot},
};
use std::{
    error::Error,
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;
const INTERVAL: Duration = Duration::from_millis(10);
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(10);
const WATCHDOG_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct State {
    snapshot: Snapshot,
    deadline: Option<Duration>,
    debug_deadline: Option<Duration>,
    debug_fps: Option<f64>,
}
struct EventNotice {
    event: WindowEvent,
    state: State,
    waker: WindowWaker,
}
struct Presentation {
    state: State,
    text_images: usize,
}
struct ObservedApp {
    app: SnakeApp,
    events: Sender<EventNotice>,
    presentations: Sender<Presentation>,
    text_images: usize,
}
impl ObservedApp {
    fn state(&self) -> State {
        State {
            snapshot: self.app.snapshot(),
            deadline: self.app.next_deadline(),
            debug_deadline: self.app.debug_overlay_deadline(),
            debug_fps: self.app.debug_overlay_fps(),
        }
    }
}
impl Application for ObservedApp {
    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        waker: &WindowWaker,
    ) -> Result<(), oxiview_core::Error> {
        self.app.event(event, scheduler, waker)?;
        let _ = self.events.send(EventNotice {
            event: event.clone(),
            state: self.state(),
            waker: waker.clone(),
        });
        Ok(())
    }
    fn paint(
        &mut self,
        timing: FrameTiming,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, oxiview_core::Error> {
        let painted = self.app.paint(timing, size, scale)?;
        let encoded = painted
            .scene
            .encode(painted.root)
            .map_err(|error| oxiview_core::Error::Application(error.to_string()))?;
        self.text_images = encoded
            .nodes
            .iter()
            .filter(|node| {
                matches!(
                    node,
                    EncodedNode::Draw {
                        primitive: Primitive::Image(..),
                        ..
                    }
                )
            })
            .count();
        Ok(painted)
    }
    fn presented(&mut self, record: Option<LatencyRecord>) {
        self.app.presented(record);
        let _ = self.presentations.send(Presentation {
            state: self.state(),
            text_images: self.text_images,
        });
    }
    fn should_close(&mut self) -> bool {
        self.app.should_close()
    }
    fn frame_submitted(&mut self, frame: FrameSubmission) {
        self.app.frame_submitted(frame);
        let _ = self.presentations.send(Presentation {
            state: self.state(),
            text_images: self.text_images,
        });
    }
}

struct CloseOnDrop {
    driver: Driver,
    id: WindowId,
}
impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        let _ = self.driver.request_user_close(self.id);
    }
}
struct Harness {
    driver: Driver,
    id: WindowId,
    output: HeadlessOutput,
    events: Receiver<EventNotice>,
    presentations: Receiver<Presentation>,
    waker: WindowWaker,
    counters: Arc<Counters>,
    state: State,
}
impl Harness {
    fn event(&mut self, expected: impl FnOnce(&WindowEvent) -> bool) -> TestResult<WindowEvent> {
        let notice = self.events.recv_timeout(RECEIVE_TIMEOUT)?;
        if !expected(&notice.event) {
            return Err(format!("unexpected window event: {:?}", notice.event).into());
        }
        self.state = notice.state;
        Ok(notice.event)
    }
    fn presented(&mut self) -> TestResult<()> {
        let notice = self.presentations.recv_timeout(RECEIVE_TIMEOUT)?;
        assert!(
            notice.text_images > 0,
            "the actual painted scene must contain rasterized text images"
        );
        self.state = notice.state;
        assert_eq!(self.counters.screen_mounts.load(Ordering::SeqCst), 1);
        Ok(())
    }
    fn blocked(&self) -> TestResult<()> {
        // A separate watchdog closes the window if the worker fails to progress.
        self.driver.wait_until_blocked(self.id, WaitKind::Event)?;
        Ok(())
    }
    fn press(&mut self, code: KeyCode) -> TestResult<()> {
        self.driver.key(self.id, code, true, false, None)?;
        self.event(|event|matches!(event,WindowEvent::Key {code:received,pressed:true,..} if *received==code))?;
        self.blocked()?;
        self.driver.key(self.id, code, false, false, None)?;
        self.event(|event| matches!(event, WindowEvent::Key { code: received, pressed: false, .. } if *received == code))?;
        self.blocked()
    }
    fn present_requested_frame(&mut self) -> TestResult<()> {
        self.blocked()?;
        self.driver.advance_by(INTERVAL)?;
        self.event(|event| matches!(event, WindowEvent::Frame { .. }))?;
        self.presented()?;
        self.blocked()
    }
    fn timed_step(&mut self) -> TestResult<()> {
        let deadline = self
            .state
            .deadline
            .ok_or("running game has no tick deadline")?;
        self.driver.advance_to(deadline)?;
        // As in the platform's timer integration test, deliver one wake before
        // observing the next blocked state. The expired deadline must be consumed
        // before the window loop can block again and the frame is armed.
        self.waker.wake()?;
        self.event(|event| matches!(event, WindowEvent::Wake))?;
        self.blocked()?;
        self.driver.advance_by(INTERVAL)?;
        self.event(|event| matches!(event, WindowEvent::Frame { .. }))?;
        self.presented()?;
        self.blocked()
    }
    fn idle_intervals(&mut self, count: usize) -> TestResult<()> {
        self.blocked()?;
        let frames = self.driver.counters(self.id)?.frames;
        let presents = self.output.snapshot()?.presents;
        let observed = self.counters.presented.load(Ordering::SeqCst);
        let board_updates = self.counters.board_updates.load(Ordering::SeqCst);
        for _ in 0..count {
            self.driver.advance_by(INTERVAL)?;
        }
        assert_eq!(self.driver.counters(self.id)?.frames, frames);
        assert_eq!(self.output.snapshot()?.presents, presents);
        assert_eq!(self.counters.presented.load(Ordering::SeqCst), observed);
        assert_eq!(
            self.counters.board_updates.load(Ordering::SeqCst),
            board_updates
        );
        assert!(matches!(
            self.events.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert!(matches!(
            self.presentations.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(self.counters.screen_mounts.load(Ordering::SeqCst), 1);
        Ok(())
    }
}

fn run_case(test: impl FnOnce(&mut Harness) -> TestResult<()> + Send) -> TestResult<()> {
    run_configured_case(Config::default(), test)
}
fn run_configured_case(
    config: Config,
    test: impl FnOnce(&mut Harness) -> TestResult<()> + Send,
) -> TestResult<()> {
    let (platform, driver) = HeadlessPlatform::new(INTERVAL)?;
    let (events_tx, events_rx) = mpsc::channel::<EventNotice>();
    let (presents_tx, presents_rx) = mpsc::channel::<Presentation>();
    let (ready_tx, ready_rx) = mpsc::channel::<(WindowId, HeadlessOutput)>();
    let (watch_ready_tx, watch_ready_rx) = mpsc::channel::<WindowId>();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let counters = Arc::new(Counters::default());
    let observed = Arc::clone(&counters);
    let watchdog = driver.clone();
    let attributes = WindowAttributes {
        title: "Snake window-loop test".into(),
        logical_size: Size::new(f64::from(config.width), f64::from(config.height)),
        resize_timeout: Duration::from_secs(1),
    };
    thread::scope(|scope| -> TestResult<()> {
        scope.spawn(move || {
            if let Ok(id) = watch_ready_rx.recv_timeout(WATCHDOG_TIMEOUT)
                && matches!(
                    done_rx.recv_timeout(WATCHDOG_TIMEOUT),
                    Err(mpsc::RecvTimeoutError::Timeout)
                )
            {
                let _ = watchdog.request_user_close(id);
            }
        });
        let scenario = scope.spawn(move || -> TestResult<()> {
            let (id, output) = ready_rx.recv_timeout(RECEIVE_TIMEOUT)?;
            let _close = CloseOnDrop {
                driver: driver.clone(),
                id,
            };
            let initial = events_rx.recv_timeout(RECEIVE_TIMEOUT)?;
            let token = match &initial.event {
                WindowEvent::Resized { token, .. } => *token,
                _ => return Err(format!("expected initial resize, got {:?}", initial.event).into()),
            };
            let mut harness = Harness {
                driver,
                id,
                output,
                events: events_rx,
                presentations: presents_rx,
                waker: initial.waker,
                counters: observed,
                state: initial.state,
            };
            harness.presented()?;
            if config.debug_overlay {
                harness.event(|event| matches!(event, WindowEvent::Wake))?;
            }
            harness.blocked()?;
            assert_eq!(
                harness.driver.resize_outcome(id, token)?,
                Some(ResizeOutcome::Acknowledged)
            );
            test(&mut harness)?;
            assert_eq!(
                harness.counters.rejected_commands.load(Ordering::SeqCst),
                0,
                "deferred command rejection after {:?} at tick {}",
                harness.state.snapshot.phase,
                harness.state.snapshot.ticks
            );
            Ok(())
        });
        let running = run_headless(
            platform,
            attributes,
            move || {
                Ok(ObservedApp {
                    app: SnakeApp::new(config, counters)?,
                    events: events_tx,
                    presentations: presents_tx,
                    text_images: 0,
                })
            },
            move |_, id, output| {
                let _ = watch_ready_tx.send(id);
                ready_tx
                    .send((id, output))
                    .map_err(|error| oxiview_platform::Error::OperationFailed(error.to_string()))
            },
        );
        let _ = done_tx.send(());
        let scenario = scenario
            .join()
            .map_err(|_| "Snake window-loop assertions panicked")?;
        running?;
        scenario
    })
}

#[test]
fn ready_screen_is_mounted_once_and_has_no_idle_frames() -> TestResult<()> {
    run_case(|harness| {
        assert_eq!(harness.state.snapshot.phase, Phase::Ready);
        assert_eq!(harness.state.snapshot.ticks, 0);
        assert_eq!(harness.state.deadline, None);
        assert_eq!(harness.output.snapshot()?.presents, 1);
        harness.idle_intervals(1000)
    })
}

#[test]
fn keyboard_and_activation_drive_timed_play_pause_and_fresh_resume() -> TestResult<()> {
    run_case(|harness| {
        harness.press(KeyCode::Enter)?;
        assert_eq!(harness.state.snapshot.phase, Phase::Running);
        assert!(harness.state.deadline.is_some());
        harness.present_requested_frame()?;
        assert_eq!(harness.state.snapshot.ticks, 0);
        let initial_head = harness.state.snapshot.body.first().copied();
        harness.timed_step()?;
        assert_eq!(harness.state.snapshot.phase, Phase::Running);
        assert_eq!(harness.state.snapshot.ticks, 1);
        assert_ne!(harness.state.snapshot.body.first().copied(), initial_head);

        harness.press(KeyCode::Space)?;
        assert_eq!(harness.state.snapshot.phase, Phase::Paused);
        assert_eq!(harness.state.deadline, None);
        harness.present_requested_frame()?;
        let paused_ticks = harness.state.snapshot.ticks;
        harness.idle_intervals(1000)?;

        let resumed_at = harness.driver.now()?;
        harness.press(KeyCode::Space)?;
        assert_eq!(harness.state.snapshot.phase, Phase::Running);
        assert_eq!(harness.state.snapshot.ticks, paused_ticks);
        let deadline = harness
            .state
            .deadline
            .ok_or("resume omitted its fresh deadline")?;
        assert_eq!(deadline, resumed_at + Duration::from_millis(125));
        harness.present_requested_frame()?;
        assert_eq!(harness.state.snapshot.ticks, paused_ticks);
        let before = harness.output.snapshot()?.presents;
        harness
            .driver
            .advance_to(deadline - Duration::from_millis(1))?;
        assert_eq!(harness.output.snapshot()?.presents, before);
        harness.timed_step()?;
        assert_eq!(harness.state.snapshot.ticks, paused_ticks + 1);
        assert_eq!(harness.state.snapshot.phase, Phase::Running);

        harness.driver.activation_changed(harness.id, false, None)?;
        harness
            .event(|event| matches!(event, WindowEvent::ActivationChanged { active: false, .. }))?;
        assert_eq!(harness.state.snapshot.phase, Phase::Paused);
        assert_eq!(harness.state.deadline, None);
        harness.present_requested_frame()?;
        let interrupted_ticks = harness.state.snapshot.ticks;
        harness.idle_intervals(500)?;
        harness.driver.activation_changed(harness.id, true, None)?;
        harness
            .event(|event| matches!(event, WindowEvent::ActivationChanged { active: true, .. }))?;
        assert_eq!(harness.state.snapshot.phase, Phase::Paused);
        assert_eq!(harness.state.snapshot.ticks, interrupted_ticks);
        assert_eq!(harness.state.deadline, None);
        harness.idle_intervals(500)?;
        assert_eq!(harness.counters.screen_mounts.load(Ordering::SeqCst), 1);
        Ok(())
    })
}

#[test]
fn debug_overlay_settles_to_zero_with_one_extra_frame_and_then_stays_idle() -> TestResult<()> {
    let config = Config {
        debug_overlay: true,
        ..Config::default()
    };
    run_configured_case(config, |harness| {
        let settle = |harness: &mut Harness| -> TestResult<()> {
            let deadline = harness
                .state
                .debug_deadline
                .ok_or("HUD omitted idle expiry")?;
            let before = harness.output.snapshot()?.presents;
            harness.driver.advance_to(deadline)?;
            harness.waker.wake()?;
            harness.event(|event| matches!(event, WindowEvent::Wake))?;
            harness.blocked()?;
            harness.driver.advance_by(INTERVAL)?;
            harness.event(|event| matches!(event, WindowEvent::Frame { .. }))?;
            harness.presented()?;
            harness.blocked()?;
            assert_eq!(harness.output.snapshot()?.presents, before + 1);
            assert_eq!(harness.state.debug_fps, Some(0.0));
            assert_eq!(harness.state.debug_deadline, None);
            harness.idle_intervals(1000)
        };
        assert_eq!(harness.state.snapshot.phase, Phase::Ready);
        settle(harness)?;
        harness.press(KeyCode::Enter)?;
        harness.present_requested_frame()?;
        harness.event(|event| matches!(event, WindowEvent::Wake))?;
        harness.blocked()?;
        assert!(harness.state.debug_deadline.is_some());
        harness.press(KeyCode::Space)?;
        harness.present_requested_frame()?;
        harness.event(|event| matches!(event, WindowEvent::Wake))?;
        harness.blocked()?;
        assert_eq!(harness.state.snapshot.phase, Phase::Paused);
        assert_eq!(harness.state.deadline, None);
        settle(harness)?;
        harness.driver.visibility_changed(harness.id, false, None)?;
        harness.event(|event| {
            matches!(event, WindowEvent::VisibilityChanged { visible: false, .. })
        })?;
        harness.blocked()?;
        assert_eq!(harness.state.debug_deadline, None);
        harness.idle_intervals(1000)
    })
}
