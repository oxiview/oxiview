//! Deterministic end-to-end window-loop tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test assertions and channel failures.
use oxiview_app::{HeadlessOutput, run_headless};
use oxiview_base::{Color, Point, Size};
use oxiview_core::*;
use oxiview_platform::*;
use oxiview_platform_headless::{Driver, HeadlessPlatform, ResizeOutcome, WaitKind};
use oxiview_scene::{NodeKind, Scene};
use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};
fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn attributes() -> WindowAttributes {
    WindowAttributes {
        title: "S3".into(),
        logical_size: Size::new(8.0, 6.0),
        resize_timeout: ms(100),
    }
}
enum Notice {
    Event(WindowEvent, WindowWaker),
    Presented(Option<LatencyRecord>),
}
struct App {
    tx: Sender<Notice>,
    timed: bool,
    invalidate: bool,
}
impl Application for App {
    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        waker: &WindowWaker,
    ) -> Result<(), oxiview_core::Error> {
        if matches!(event, WindowEvent::PointerMoved { .. }) && self.invalidate {
            scheduler.invalidate();
        }
        if matches!(event, WindowEvent::Wake) && self.timed {
            scheduler.request_at(ms(25));
            self.timed = false;
        }
        self.tx
            .send(Notice::Event(event.clone(), waker.clone()))
            .unwrap();
        Ok(())
    }
    fn paint(
        &mut self,
        _: FrameTiming,
        _: PhysicalSize,
        _: f64,
    ) -> Result<PaintedScene, oxiview_core::Error> {
        let mut scene = Scene::new();
        let root = scene.insert(None, 0, NodeKind::Opacity(1.0)).unwrap();
        Ok(PaintedScene { scene, root })
    }
    fn presented(&mut self, record: Option<LatencyRecord>) {
        self.tx.send(Notice::Presented(record)).unwrap();
    }
}
fn event(rx: &Receiver<Notice>) -> (WindowEvent, WindowWaker) {
    match rx.recv().unwrap() {
        Notice::Event(event, waker) => (event, waker),
        _ => panic!("expected event"),
    }
}
fn presented(rx: &Receiver<Notice>) -> Option<LatencyRecord> {
    match rx.recv().unwrap() {
        Notice::Presented(record) => record,
        _ => panic!("expected presentation"),
    }
}
fn run(
    timed: bool,
    invalidate: bool,
    test: impl FnOnce(Driver, WindowId, HeadlessOutput, &Receiver<Notice>, WindowWaker) + Send,
) {
    run_application(
        move |tx| App {
            tx,
            timed,
            invalidate,
        },
        test,
    );
}

fn run_application<A: Application>(
    create: impl FnOnce(Sender<Notice>) -> A + Send + 'static,
    test: impl FnOnce(Driver, WindowId, HeadlessOutput, &Receiver<Notice>, WindowWaker) + Send,
) {
    let (platform, driver) = HeadlessPlatform::new(ms(10)).unwrap();
    let (tx, rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    thread::scope(|scope| {
        scope.spawn(move || {
            let (id, output) = ready_rx.recv().unwrap();
            let cleanup = driver.clone();
            // A failed assertion must close the window, or run_headless never returns
            // and the test hangs instead of failing.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let (initial, waker) = event(&rx);
                let WindowEvent::Resized {
                    token,
                    predicted_presentation_timestamp,
                    ..
                } = initial
                else {
                    panic!("initial resize")
                };
                assert_eq!(predicted_presentation_timestamp, ms(10));
                assert_eq!(presented(&rx), None);
                assert_eq!(
                    driver.wait_resize(id, token).unwrap(),
                    ResizeOutcome::Acknowledged
                );
                driver.wait_until_blocked(id, WaitKind::Event).unwrap();
                test(driver.clone(), id, output, &rx, waker);
                driver.request_user_close(id).unwrap();
                assert!(matches!(event(&rx).0, WindowEvent::CloseRequested));
                assert!(matches!(event(&rx).0, WindowEvent::Destroyed));
            }));
            if let Err(panic) = outcome {
                let _ = cleanup.request_user_close(id);
                std::panic::resume_unwind(panic);
            }
        });
        run_headless(
            platform,
            attributes(),
            move || Ok(create(tx)),
            |_, id, output| {
                ready_tx.send((id, output)).unwrap();
                Ok(())
            },
        )
        .unwrap();
    });
}

struct InputPixelApp {
    tree: UiTree,
    notices: Sender<Notice>,
}

impl InputPixelApp {
    fn new(notices: Sender<Notice>) -> Self {
        let mut tree = UiTree::new().unwrap();
        let child = tree.append(tree.root(), Default::default()).unwrap();
        tree.set_intrinsic_size(child, Size::new(4.0, 4.0)).unwrap();
        let mut style = tree.style(child).unwrap().clone();
        style.background = Color::from_rgba8(255, 0, 0, 255);
        tree.set_style(child, style).unwrap();
        tree.set_pointer_handler(child, move |event, tree, commands| {
            if event.kind == PointerEventKind::Down {
                let mut style = tree.style(child).unwrap().clone();
                style.background = Color::from_rgba8(0, 0, 255, 255);
                commands.set_style(child, style);
            }
        })
        .unwrap();
        Self { tree, notices }
    }
}

impl Application for InputPixelApp {
    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        waker: &WindowWaker,
    ) -> Result<(), oxiview_core::Error> {
        let result = self
            .tree
            .dispatch(event, attributes().logical_size, |_, _, _| {})
            .unwrap();
        if result.changed {
            scheduler.invalidate();
        }
        self.notices
            .send(Notice::Event(event.clone(), waker.clone()))
            .unwrap();
        Ok(())
    }

    fn paint(
        &mut self,
        _: FrameTiming,
        _: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, oxiview_core::Error> {
        Ok(self.tree.paint(attributes().logical_size, scale).unwrap())
    }

    fn presented(&mut self, record: Option<LatencyRecord>) {
        self.notices.send(Notice::Presented(record)).unwrap();
    }
}

#[test]
fn retained_hit_changes_pixels_on_the_next_frame_and_then_stays_idle() {
    run_application(InputPixelApp::new, |driver, id, output, rx, _| {
        let pixel = |x: usize, y: usize| {
            let image = output.snapshot().unwrap().image.unwrap();
            let offset = 4 * (y * image.width() as usize + x);
            <[u8; 4]>::try_from(&image.pixels()[offset..offset + 4]).unwrap()
        };
        assert_eq!(pixel(1, 1), [255, 0, 0, 255]);
        let outside = pixel(7, 5);
        let position = Point::new(1.0, 1.0);
        driver.advance_to(ms(3)).unwrap();
        driver
            .pointer_button(id, PointerButton::Primary, true, position, None)
            .unwrap();
        assert!(matches!(
            event(rx).0,
            WindowEvent::PointerButton { pressed: true, .. }
        ));
        driver.wait_until_blocked(id, WaitKind::Event).unwrap();
        assert_eq!(output.snapshot().unwrap().presents, 1);
        driver.advance_to(ms(10)).unwrap();
        assert!(matches!(event(rx).0, WindowEvent::Frame { .. }));
        assert_eq!(presented(rx).unwrap().input_capture_timestamp, ms(3));
        assert_eq!(pixel(1, 1), [0, 0, 255, 255]);
        assert_eq!(pixel(7, 5), outside);

        driver
            .pointer_button(id, PointerButton::Primary, false, position, None)
            .unwrap();
        assert!(matches!(
            event(rx).0,
            WindowEvent::PointerButton { pressed: false, .. }
        ));
        driver.wait_until_blocked(id, WaitKind::Event).unwrap();
        for _ in 0..1000 {
            driver.advance_by(ms(10)).unwrap();
        }
        assert_eq!(driver.counters(id).unwrap().frames, 1);
        assert_eq!(output.snapshot().unwrap().presents, 2);
    });
}
#[test]
fn idle_has_zero_additional_frames_and_presents() {
    run(false, true, |d, id, output, _, _| {
        let initial = output.snapshot().unwrap().presents;
        for _ in 0..100 {
            d.advance_by(ms(10)).unwrap();
        }
        assert_eq!(d.counters(id).unwrap().frames, 0);
        assert_eq!(output.snapshot().unwrap().presents, initial);
    });
}
#[test]
fn deterministic_input_phases_meet_one_interval_and_then_remain_idle() {
    run(false, true, |d, id, output, rx, _| {
        // Includes the full-interval boundary and a point immediately before vsync.
        let interval = ms(10);
        for (cycle, phase) in [0, 1, 3, 5, 7, 9].into_iter().enumerate() {
            let start = interval * cycle as u32;
            let capture = start + ms(phase);
            d.advance_to(capture).unwrap();
            d.pointer_moved(id, Point::new(cycle as f64, 2.0), None)
                .unwrap();
            assert!(matches!(event(rx).0, WindowEvent::PointerMoved { .. }));
            d.wait_until_blocked(id, WaitKind::Event).unwrap();
            d.advance_to(start + interval).unwrap();
            assert!(matches!(event(rx).0, WindowEvent::Frame { .. }));
            let record = presented(rx).unwrap();
            assert_eq!(record.input_capture_timestamp, capture);
            assert_eq!(record.timing.refresh_interval, interval);
            assert!(record.submit_latency().unwrap() <= interval);
            assert_eq!(record.submission_timestamp, start + interval);
            d.wait_until_blocked(id, WaitKind::Event).unwrap();
        }
        let initial = output.snapshot().unwrap().presents;
        let events = d.counters(id).unwrap().frames;
        for _ in 0..1000 {
            d.advance_by(interval).unwrap();
        }
        assert_eq!(output.snapshot().unwrap().presents, initial);
        assert_eq!(d.counters(id).unwrap().frames, events);
    });
}

#[test]
fn pointer_produces_one_next_vsync_frame_with_latency() {
    run(false, true, |d, id, output, rx, _| {
        d.advance_to(ms(3)).unwrap();
        d.pointer_moved(id, Point::new(2.0, 3.0), None).unwrap();
        assert!(matches!(event(rx).0, WindowEvent::PointerMoved { .. }));
        d.wait_until_blocked(id, WaitKind::Event).unwrap();
        d.advance_to(ms(10)).unwrap();
        assert!(matches!(event(rx).0, WindowEvent::Frame { .. }));
        let record = presented(rx).unwrap();
        assert_eq!(record.timing.kind, FrameKind::Vsync);
        assert_eq!(record.input_capture_timestamp, ms(3));
        assert_eq!(record.timing.predicted_presentation_timestamp, ms(10));
        assert_eq!(record.latency(), Some(ms(7)));
        d.wait_until_blocked(id, WaitKind::Event).unwrap();
        d.advance_to(ms(1000)).unwrap();
        assert_eq!(d.counters(id).unwrap().frames, 1);
        assert_eq!(output.snapshot().unwrap().presents, 2);
    });
}
#[test]
fn resize_presents_before_ack_zero_suspends_and_skips_clean_input() {
    run(false, false, |d, id, output, rx, _| {
        d.advance_to(ms(3)).unwrap();
        d.pointer_moved(id, Point::new(1.0, 1.0), Some(ms(2)))
            .unwrap();
        event(rx);
        d.wait_until_blocked(id, WaitKind::Event).unwrap();
        let token = d
            .resize(
                id,
                PhysicalSize {
                    width: 16,
                    height: 12,
                },
                2.0,
            )
            .unwrap();
        assert!(matches!(
            event(rx).0,
            WindowEvent::ScaleFactorChanged { scale_factor: 2.0 }
        ));
        assert!(matches!(event(rx).0, WindowEvent::Resized { .. }));
        // The pointer did not invalidate, so it is not attributed to this frame.
        assert_eq!(presented(rx), None);
        assert_eq!(
            d.wait_resize(id, token).unwrap(),
            ResizeOutcome::Acknowledged
        );
        let snapshot = output.snapshot().unwrap();
        let image = snapshot.image.unwrap();
        assert_eq!((image.width(), image.height()), (16, 12));
        assert_eq!(snapshot.presents, 2);
        let token = d
            .resize(
                id,
                PhysicalSize {
                    width: 0,
                    height: 12,
                },
                2.0,
            )
            .unwrap();
        event(rx);
        assert_eq!(
            d.wait_resize(id, token).unwrap(),
            ResizeOutcome::Acknowledged
        );
        d.wait_until_blocked(id, WaitKind::Event).unwrap();
        d.advance_to(ms(1000)).unwrap();
        assert_eq!(output.snapshot().unwrap().presents, 2);
        assert_eq!(d.counters(id).unwrap().frames, 0);
    });
}
#[test]
fn timed_deadline_and_cross_thread_wake() {
    run(true, true, |d, id, output, rx, waker| {
        thread::spawn({
            let waker = waker.clone();
            move || waker.wake().unwrap()
        })
        .join()
        .unwrap();
        assert!(matches!(event(rx).0, WindowEvent::Wake));
        d.wait_until_blocked(id, WaitKind::Event).unwrap();
        d.advance_to(ms(25)).unwrap();
        // Observe a delivered event before the next blocked handshake. Whether
        // it precedes or follows the timeout, the expired deadline is consumed
        // before the loop can block again.
        waker.wake().unwrap();
        assert!(matches!(event(rx).0, WindowEvent::Wake));
        d.wait_until_blocked(id, WaitKind::Event).unwrap();
        assert_eq!(output.snapshot().unwrap().presents, 1);
        d.advance_to(ms(30)).unwrap();
        assert!(matches!(event(rx).0, WindowEvent::Frame { .. }));
        assert_eq!(presented(rx), None);
        assert_eq!(output.snapshot().unwrap().presents, 2);
    });
}
#[test]
fn request_exit_joins_window_cleanly() {
    let (platform, _) = HeadlessPlatform::new(ms(10)).unwrap();
    let (tx, rx) = mpsc::channel();
    run_headless(
        platform,
        attributes(),
        move || {
            Ok(App {
                tx,
                timed: false,
                invalidate: true,
            })
        },
        |ctx, _, _| ctx.request_exit(),
    )
    .unwrap();
    assert!(matches!(event(&rx).0, WindowEvent::Destroyed));
}
#[test]
fn initialization_panic_is_reported_and_endpoint_is_released() {
    let (platform, _) = HeadlessPlatform::new(ms(10)).unwrap();
    let result = run_headless::<App>(
        platform,
        attributes(),
        || panic!("test unexpected application unwind"),
        |_, _, _| Ok(()),
    );
    assert!(matches!(
        result,
        Err(oxiview_app::Error::Core(oxiview_core::Error::Panicked))
    ));
}
