// Test-only Scene setup and callback assertions use fixture unwraps.
#![allow(clippy::unwrap_used)]

use super::*;
use std::{cell::Cell, rc::Rc};

fn timing() -> FrameTiming {
    FrameTiming {
        predicted_presentation_timestamp: Duration::from_millis(16),
        refresh_interval: Duration::from_millis(16),
        kind: FrameKind::Vsync,
    }
}
fn size() -> PhysicalSize {
    PhysicalSize {
        width: 20,
        height: 20,
    }
}
fn scene() -> PaintedScene {
    let mut scene = Scene::new();
    let root = scene
        .insert(None, 0, oxiview_scene::NodeKind::Opacity(1.0))
        .unwrap();
    PaintedScene { scene, root }
}
#[derive(Default)]
struct SubmissionApp {
    frames: Vec<FrameSubmission>,
    legacy_calls: usize,
}
impl Application for SubmissionApp {
    fn event(&mut self, _: &WindowEvent, _: &mut Scheduler, _: &WindowWaker) -> Result<(), Error> {
        Ok(())
    }
    fn paint(&mut self, _: FrameTiming, _: PhysicalSize, _: f64) -> Result<PaintedScene, Error> {
        Ok(scene())
    }
    fn frame_submitted(&mut self, frame: FrameSubmission) {
        self.frames.push(frame);
    }
    fn presented(&mut self, _: Option<LatencyRecord>) {
        self.legacy_calls += 1;
    }
}
struct Output {
    outcome: PresentationOutcome,
    calls: Rc<Cell<usize>>,
}
impl Presenter for Output {
    fn configure(&mut self, _: PhysicalSize, _: f64, _: bool) -> Result<(), Error> {
        Ok(())
    }
    fn present(&mut self, _: &PaintedScene) -> Result<PresentationOutcome, Error> {
        self.calls.set(self.calls.get() + 1);
        Ok(self.outcome)
    }
}

#[test]
fn successful_submission_uses_one_post_presenter_sample_even_without_input() {
    for input in [None, Some(Duration::from_millis(3))] {
        let mut app = SubmissionApp::default();
        let calls = Rc::new(Cell::new(0));
        let mut presenter = Output {
            outcome: PresentationOutcome::Presented,
            calls: Rc::clone(&calls),
        };
        let samples = Cell::new(0);
        let mut scheduler = Scheduler::default();
        scheduler.resize(size());
        if let Some(input) = input {
            scheduler.input(input);
        }
        let submitted = Duration::from_millis(250);
        let outcome = produce(
            || {
                assert_eq!(calls.get(), 1);
                samples.set(samples.get() + 1);
                submitted
            },
            &mut app,
            &mut presenter,
            &mut scheduler,
            timing(),
            size(),
            1.0,
        )
        .unwrap();
        assert_eq!(outcome, PresentationOutcome::Presented);
        assert_eq!(samples.get(), 1);
        assert_eq!(app.legacy_calls, 0);
        assert_eq!(
            app.frames,
            vec![FrameSubmission {
                timing: timing(),
                submission_timestamp: submitted,
                latency: input.map(|input_capture_timestamp| LatencyRecord {
                    input_capture_timestamp,
                    timing: timing(),
                    submission_timestamp: submitted
                })
            }]
        );
        assert_eq!(scheduler.schedule(), Schedule::default());
    }
}

#[test]
fn skipped_attempts_do_not_sample_or_notify_and_retry_keeps_input() {
    let mut app = SubmissionApp::default();
    let mut presenter = Output {
        outcome: PresentationOutcome::Skipped,
        calls: Rc::new(Cell::new(0)),
    };
    let samples = Cell::new(0);
    let mut scheduler = Scheduler::default();
    scheduler.resize(size());
    scheduler.input(Duration::from_millis(3));
    let skipped = produce(
        || {
            samples.set(samples.get() + 1);
            Duration::from_millis(10)
        },
        &mut app,
        &mut presenter,
        &mut scheduler,
        timing(),
        size(),
        1.0,
    )
    .unwrap();
    assert_eq!(skipped, PresentationOutcome::Skipped);
    assert_eq!(samples.get(), 0);
    assert!(app.frames.is_empty());
    assert_eq!(app.legacy_calls, 0);
    assert!(scheduler.schedule().request_frame);
    assert!(scheduler.frame());
    presenter.outcome = PresentationOutcome::Presented;
    produce(
        || {
            samples.set(samples.get() + 1);
            Duration::from_millis(90)
        },
        &mut app,
        &mut presenter,
        &mut scheduler,
        timing(),
        size(),
        1.0,
    )
    .unwrap();
    assert_eq!(samples.get(), 1);
    assert_eq!(app.frames.len(), 1);
    let frame = app.frames[0];
    assert_eq!(frame.submission_timestamp, Duration::from_millis(90));
    assert_eq!(
        frame.latency.unwrap().input_capture_timestamp,
        Duration::from_millis(3)
    );
}

#[test]
fn default_submission_hook_forwards_once_to_legacy_presented() {
    struct LegacyApp(Vec<Option<LatencyRecord>>);
    impl Application for LegacyApp {
        fn event(
            &mut self,
            _: &WindowEvent,
            _: &mut Scheduler,
            _: &WindowWaker,
        ) -> Result<(), Error> {
            Ok(())
        }
        fn paint(
            &mut self,
            _: FrameTiming,
            _: PhysicalSize,
            _: f64,
        ) -> Result<PaintedScene, Error> {
            Ok(scene())
        }
        fn presented(&mut self, latency: Option<LatencyRecord>) {
            self.0.push(latency);
        }
    }
    let mut app = LegacyApp(Vec::new());
    let mut presenter = Output {
        outcome: PresentationOutcome::Presented,
        calls: Rc::new(Cell::new(0)),
    };
    let mut scheduler = Scheduler::default();
    scheduler.resize(size());
    produce(
        || Duration::from_millis(20),
        &mut app,
        &mut presenter,
        &mut scheduler,
        timing(),
        size(),
        1.0,
    )
    .unwrap();
    scheduler.invalidate();
    scheduler.input(Duration::from_millis(5));
    produce(
        || Duration::from_millis(30),
        &mut app,
        &mut presenter,
        &mut scheduler,
        timing(),
        size(),
        1.0,
    )
    .unwrap();
    assert_eq!(
        app.0,
        vec![
            None,
            Some(LatencyRecord {
                input_capture_timestamp: Duration::from_millis(5),
                timing: timing(),
                submission_timestamp: Duration::from_millis(30)
            })
        ]
    );
}
