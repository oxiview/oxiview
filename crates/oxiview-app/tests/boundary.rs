//! Failure boundaries and resize ordering using the real headless endpoint.
#![allow(clippy::unwrap_used, clippy::panic)] // Deliberate failure injection in tests.
use oxiview_base::Size;
use oxiview_core::*;
use oxiview_platform::*;
use oxiview_platform_headless::{HeadlessPlatform, ResizeOutcome};
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
struct App {
    panic: bool,
    token: mpsc::Sender<ResizeToken>,
}
impl Application for App {
    fn event(
        &mut self,
        event: &WindowEvent,
        _: &mut Scheduler,
        _: &WindowWaker,
    ) -> Result<(), oxiview_core::Error> {
        if let WindowEvent::Resized { token, .. } = event {
            self.token.send(*token).unwrap();
        }
        Ok(())
    }
    fn paint(
        &mut self,
        _: FrameTiming,
        _: PhysicalSize,
        _: f64,
    ) -> Result<PaintedScene, oxiview_core::Error> {
        assert!(!self.panic, "test paint unwind");
        let mut scene = Scene::new();
        let root = scene.insert(None, 0, NodeKind::Opacity(1.0)).unwrap();
        Ok(PaintedScene { scene, root })
    }
}
struct GatedPresenter {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
    dropped: Arc<AtomicBool>,
    fail: bool,
}
impl Presenter for GatedPresenter {
    fn configure(&mut self, _: PhysicalSize, _: f64) -> Result<(), oxiview_core::Error> {
        Ok(())
    }
    fn present(&mut self, _: &PaintedScene) -> Result<(), oxiview_core::Error> {
        self.entered.send(()).unwrap();
        self.release.recv().unwrap();
        if self.fail {
            Err(oxiview_core::Error::Present("test failure".into()))
        } else {
            Ok(())
        }
    }
}
impl Drop for GatedPresenter {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}
fn exercise(panic: bool, fail: bool) {
    let (platform, driver) = HeadlessPlatform::new(Duration::from_millis(10)).unwrap();
    let (token_tx, token_rx) = mpsc::channel();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let dropped = Arc::new(AtomicBool::new(false));
    let drop_check = dropped.clone();
    thread::scope(|scope| {
        let mut worker = None;
        platform
            .run(|context| {
                let window = context.create_window(WindowAttributes {
                    title: "boundary".into(),
                    logical_size: Size::new(8.0, 8.0),
                    resize_timeout: Duration::from_secs(1),
                })?;
                let id = window.main.id();
                scope.spawn(move || {
                    let token = token_rx.recv().unwrap();
                    if !panic {
                        entered_rx.recv().unwrap();
                        assert_eq!(
                            driver.resize_outcome(id, token).unwrap(),
                            None,
                            "must not acknowledge before present returns"
                        );
                        release_tx.send(()).unwrap();
                        if !fail {
                            assert_eq!(
                                driver.wait_resize(id, token).unwrap(),
                                ResizeOutcome::Acknowledged
                            );
                            driver.request_user_close(id).unwrap();
                        }
                    }
                });
                worker = Some(scope.spawn(move || {
                    run_window(window.endpoint, || {
                        Ok((
                            App {
                                panic,
                                token: token_tx,
                            },
                            GatedPresenter {
                                entered: entered_tx,
                                release: release_rx,
                                dropped,
                                fail,
                            },
                        ))
                    })
                }));
                Ok(())
            })
            .unwrap();
        // Platform run waits for endpoint drop; presenter teardown must already
        // be complete, including on an application unwind or a present error.
        assert!(drop_check.load(Ordering::SeqCst));
        let result = worker.unwrap().join().unwrap();
        if panic {
            assert!(matches!(result, Err(oxiview_core::Error::Panicked)));
        } else if fail {
            assert!(matches!(result, Err(oxiview_core::Error::Present(_))));
        } else {
            assert!(result.is_ok());
        }
    });
}
#[test]
fn resize_acknowledgement_waits_for_present_completion() {
    exercise(false, false);
}
#[test]
fn present_error_terminates_and_releases_resources() {
    exercise(false, true);
}
#[test]
fn paint_panic_terminates_and_releases_resources() {
    exercise(true, false);
}
