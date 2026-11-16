//! Skip/retry and resize acknowledgement with the real deterministic endpoint.
#![allow(clippy::unwrap_used, clippy::panic)] // Test fixtures and assertions.
use oxiview_base::{Point, Size};
use oxiview_core::*;
use oxiview_platform::*;
use oxiview_platform_headless::{HeadlessPlatform, ResizeOutcome, WaitKind};
use oxiview_scene::{NodeKind, Scene};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
struct App(mpsc::Sender<Option<LatencyRecord>>, mpsc::Sender<()>);
impl Application for App {
    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        _: &WindowWaker,
    ) -> Result<(), oxiview_core::Error> {
        if matches!(event, WindowEvent::PointerMoved { .. }) {
            scheduler.invalidate();
            self.1.send(()).unwrap();
        }
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
        self.0.send(record).unwrap();
    }
}
struct Skipper {
    skip: Arc<AtomicBool>,
    attempts: mpsc::Sender<PresentationOutcome>,
}
impl Presenter for Skipper {
    fn configure(&mut self, _: PhysicalSize, _: f64, _: bool) -> Result<(), oxiview_core::Error> {
        Ok(())
    }
    fn present(&mut self, _: &PaintedScene) -> Result<PresentationOutcome, oxiview_core::Error> {
        let outcome = if self.skip.load(Ordering::SeqCst) {
            PresentationOutcome::Skipped
        } else {
            PresentationOutcome::Presented
        };
        self.attempts.send(outcome).unwrap();
        Ok(outcome)
    }
}
fn exercise(mode: u8) {
    let (platform, driver) = HeadlessPlatform::new(ms(10)).unwrap();
    let skip = Arc::new(AtomicBool::new(false));
    let (present_tx, present_rx) = mpsc::channel::<Option<LatencyRecord>>();
    let (input_tx, input_rx) = mpsc::channel();
    let (attempt_tx, attempt_rx) = mpsc::channel();
    thread::scope(|scope| {
        let mut worker = None;
        platform
            .run(|context| {
                let window = context.create_window(WindowAttributes {
                    title: "skip".into(),
                    logical_size: Size::new(10.0, 10.0),
                    resize_timeout: ms(100),
                })?;
                let id = window.main.id();
                let control = skip.clone();
                scope.spawn(move || {
                    let cleanup = driver.clone();
                    let outcome = std::panic::catch_unwind(move || {
                        assert_eq!(attempt_rx.recv().unwrap(), PresentationOutcome::Presented);
                        assert_eq!(present_rx.recv().unwrap(), None);
                        driver.wait_until_blocked(id, WaitKind::Event).unwrap();
                        assert_eq!(driver.counters(id).unwrap().resize_acknowledgements, 1);
                        control.store(true, Ordering::SeqCst);
                        driver.advance_to(ms(3)).unwrap();
                        driver
                            .pointer_moved(id, Point::new(1.0, 1.0), Some(ms(2)))
                            .unwrap();
                        input_rx.recv().unwrap();
                        driver.wait_until_blocked(id, WaitKind::Event).unwrap();
                        let pending = if mode != 0 {
                            Some(
                                driver
                                    .resize(
                                        id,
                                        PhysicalSize {
                                            width: 20,
                                            height: 20,
                                        },
                                        1.0,
                                    )
                                    .unwrap(),
                            )
                        } else {
                            None
                        };
                        if pending.is_none() {
                            driver.advance_to(ms(10)).unwrap();
                            assert_eq!(attempt_rx.recv().unwrap(), PresentationOutcome::Skipped);
                        } else {
                            assert_eq!(attempt_rx.recv().unwrap(), PresentationOutcome::Skipped);
                        }
                        driver.wait_until_blocked(id, WaitKind::Event).unwrap();
                        assert!(present_rx.try_recv().is_err());
                        if let Some(token) = pending {
                            assert_eq!(driver.resize_outcome(id, token).unwrap(), None);
                        }
                        let replacement = if mode == 2 {
                            let token = driver
                                .resize(
                                    id,
                                    PhysicalSize {
                                        width: 30,
                                        height: 30,
                                    },
                                    1.0,
                                )
                                .unwrap();
                            assert_eq!(attempt_rx.recv().unwrap(), PresentationOutcome::Skipped);
                            driver.wait_until_blocked(id, WaitKind::Event).unwrap();
                            assert_eq!(
                                driver.resize_outcome(id, pending.unwrap()).unwrap(),
                                Some(ResizeOutcome::Superseded)
                            );
                            Some(token)
                        } else {
                            pending
                        };
                        control.store(false, Ordering::SeqCst);
                        driver.advance_by(ms(10)).unwrap();
                        assert_eq!(attempt_rx.recv().unwrap(), PresentationOutcome::Presented);
                        let record = present_rx.recv().unwrap().unwrap();
                        assert_eq!(record.input_capture_timestamp, ms(2));
                        assert_eq!(record.timing.kind, FrameKind::Vsync);
                        if let Some(token) = replacement {
                            assert_eq!(
                                driver.wait_resize(id, token).unwrap(),
                                ResizeOutcome::Acknowledged
                            );
                        }
                        driver.wait_until_blocked(id, WaitKind::Event).unwrap();
                        let counters = driver.counters(id).unwrap();
                        assert_eq!(
                            counters.resize_acknowledgements,
                            if mode == 0 { 1 } else { 2 }
                        );
                        driver.advance_by(ms(1000)).unwrap();
                        assert_eq!(driver.counters(id).unwrap().frames, counters.frames);
                        assert!(attempt_rx.try_recv().is_err());
                        driver.request_user_close(id).unwrap();
                    });
                    if let Err(payload) = outcome {
                        let _ = cleanup.request_user_close(id);
                        std::panic::resume_unwind(payload);
                    }
                });
                worker = Some(scope.spawn(move || {
                    run_window(window.endpoint, |endpoint| {
                        assert!(endpoint.native_surface_handle()?.is_none());
                        Ok((
                            App(present_tx, input_tx),
                            Skipper {
                                skip,
                                attempts: attempt_tx,
                            },
                        ))
                    })
                }));
                Ok(())
            })
            .unwrap();
        worker.unwrap().join().unwrap().unwrap();
    });
}
#[test]
fn skipped_vsync_retains_input_and_retries() {
    exercise(0);
}
#[test]
fn skipped_resize_is_acknowledged_on_later_vsync() {
    exercise(1);
}
#[test]
fn newer_resize_supersedes_skipped_resize() {
    exercise(2);
}
