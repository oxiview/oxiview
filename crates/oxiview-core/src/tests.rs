use super::*;
fn timing(kind: FrameKind) -> FrameTiming {
    FrameTiming {
        predicted_presentation_timestamp: Duration::from_millis(20),
        refresh_interval: Duration::from_millis(10),
        kind,
    }
}
#[test]
fn idle_invalidation_redraw_and_coalescing() {
    let mut s = Scheduler::default();
    assert_eq!(s.schedule(), Schedule::default());
    s.invalidate();
    assert!(s.schedule().request_frame);
    s.redraw();
    assert!(!s.schedule().request_frame);
    assert!(s.frame());
    assert_eq!(
        s.produced(timing(FrameKind::Vsync), Duration::from_millis(10)),
        None
    );
    assert_eq!(s.schedule(), Schedule::default());
    assert!(!s.frame());
    s.redraw();
    assert!(s.schedule().request_frame);
}
#[test]
fn earliest_deadline_expires_once() {
    let mut s = Scheduler::default();
    s.request_at(Duration::from_millis(30));
    s.request_at(Duration::from_millis(10));
    assert_eq!(s.schedule().deadline, Some(Duration::from_millis(10)));
    s.deadline_reached(Duration::from_millis(9));
    assert!(!s.schedule().request_frame);
    s.deadline_reached(Duration::from_millis(10));
    assert_eq!(
        s.schedule(),
        Schedule {
            request_frame: true,
            deadline: None
        }
    );
}
#[test]
fn resize_consumes_input_without_duplicate_vsync_and_zero_suspends() {
    let mut s = Scheduler::default();
    s.invalidate();
    s.input(Duration::from_millis(8));
    s.input(Duration::from_millis(3));
    assert!(s.schedule().request_frame);
    assert!(s.resize(PhysicalSize {
        width: 8,
        height: 8
    }));
    let record = s.produced(timing(FrameKind::Resize), Duration::from_millis(10));
    assert_eq!(
        record.map(|r| r.latency()),
        Some(Some(Duration::from_millis(17)))
    );
    assert!(!s.frame());
    assert!(!s.resize(PhysicalSize {
        width: 0,
        height: 8
    }));
    s.request_at(Duration::ZERO);
    s.input(Duration::from_millis(6));
    assert_eq!(s.schedule(), Schedule::default());
    assert!(!s.frame());
    assert!(s.resize(PhysicalSize {
        width: 8,
        height: 8
    }));
    assert!(
        s.produced(timing(FrameKind::Resize), Duration::from_millis(10))
            .is_some()
    );
    assert!(
        s.produced(timing(FrameKind::Vsync), Duration::from_millis(10))
            .is_none()
    );
}
#[test]
fn input_that_does_not_invalidate_is_not_attributed() {
    let mut s = Scheduler::default();
    s.input(Duration::from_millis(1));
    s.invalidate();
    s.input(Duration::from_millis(5));
    let record = s.produced(timing(FrameKind::Vsync), Duration::from_millis(10));
    assert_eq!(
        record.map(|r| r.input_capture_timestamp),
        Some(Duration::from_millis(5))
    );
}

#[test]
#[allow(clippy::unwrap_used)] // Scene construction is a test fixture.
fn skipped_frame_retains_input_and_rearms_until_presented() {
    struct App {
        records: Vec<Option<LatencyRecord>>,
    }
    impl Application for App {
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
            let mut scene = Scene::new();
            let root = scene
                .insert(None, 0, oxiview_scene::NodeKind::Opacity(1.0))
                .unwrap();
            Ok(PaintedScene { scene, root })
        }
        fn presented(&mut self, record: Option<LatencyRecord>) {
            self.records.push(record);
        }
    }
    struct Skipper(bool);
    impl Presenter for Skipper {
        fn configure(&mut self, _: PhysicalSize, _: f64, _: bool) -> Result<(), Error> {
            Ok(())
        }
        fn present(&mut self, _: &PaintedScene) -> Result<PresentationOutcome, Error> {
            Ok(if std::mem::take(&mut self.0) {
                PresentationOutcome::Skipped
            } else {
                PresentationOutcome::Presented
            })
        }
    }
    for kind in [FrameKind::Resize, FrameKind::Vsync] {
        let mut scheduler = Scheduler::default();
        let size = PhysicalSize {
            width: 10,
            height: 10,
        };
        scheduler.resize(size);
        scheduler.input(Duration::from_millis(3));
        if kind == FrameKind::Vsync {
            assert!(scheduler.schedule().request_frame);
            assert!(scheduler.frame());
        }
        let mut app = App {
            records: Vec::new(),
        };
        let mut presenter = Skipper(true);
        assert_eq!(
            produce(
                || Duration::from_millis(10),
                &mut app,
                &mut presenter,
                &mut scheduler,
                timing(kind),
                size,
                1.0
            )
            .unwrap(),
            PresentationOutcome::Skipped
        );
        assert!(app.records.is_empty());
        assert!(scheduler.schedule().request_frame);
        assert!(scheduler.frame());
        assert_eq!(
            produce(
                || Duration::from_millis(10),
                &mut app,
                &mut presenter,
                &mut scheduler,
                timing(FrameKind::Vsync),
                size,
                1.0
            )
            .unwrap(),
            PresentationOutcome::Presented
        );
        assert_eq!(app.records.len(), 1);
        assert_eq!(
            app.records[0].unwrap().input_capture_timestamp,
            Duration::from_millis(3)
        );
        assert_eq!(scheduler.schedule(), Schedule::default());
    }
}

#[test]
#[allow(clippy::unwrap_used)] // Deterministic endpoint fixture.
fn resize_token_waits_for_a_presented_retry() {
    use oxiview_platform::{Capabilities, FramePolicy, WindowId, WindowWake};
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };
    #[derive(Default)]
    struct Observed {
        attempts: usize,
        presented: usize,
        acknowledgements: Vec<ResizeToken>,
    }
    struct Wake;
    impl WindowWake for Wake {
        fn wake(&self) -> Result<(), oxiview_platform::Error> {
            Ok(())
        }
    }
    struct Endpoint {
        events: VecDeque<WindowEvent>,
        observed: Arc<Mutex<Observed>>,
        requests: usize,
    }
    impl WindowEndpoint for Endpoint {
        fn id(&self) -> WindowId {
            WindowId::new(1)
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                frame_policies: vec![FramePolicy::OnDemand],
                native_surface: false,
            }
        }
        fn now(&self) -> Duration {
            Duration::ZERO
        }
        fn waker(&self) -> WindowWaker {
            WindowWaker::new(Arc::new(Wake))
        }
        fn wait_event(&mut self) -> Result<WindowEvent, oxiview_platform::Error> {
            self.events
                .pop_front()
                .ok_or(oxiview_platform::Error::EventChannelClosed)
        }
        fn wait_event_until(
            &mut self,
            _: Duration,
        ) -> Result<Option<WindowEvent>, oxiview_platform::Error> {
            self.wait_event().map(Some)
        }
        fn poll_event(&mut self) -> Result<Option<WindowEvent>, oxiview_platform::Error> {
            Ok(self.events.pop_front())
        }
        fn request_frame(&mut self) -> Result<(), oxiview_platform::Error> {
            let observed = self.observed.lock().unwrap();
            assert_eq!(observed.attempts, 1);
            assert_eq!(observed.presented, 0);
            assert!(observed.acknowledgements.is_empty());
            self.requests += 1;
            Ok(())
        }
        fn set_frame_policy(&mut self, _: FramePolicy) -> Result<(), oxiview_platform::Error> {
            Ok(())
        }
        fn acknowledge_resize(
            &mut self,
            token: ResizeToken,
        ) -> Result<(), oxiview_platform::Error> {
            let mut observed = self.observed.lock().unwrap();
            assert_eq!(observed.attempts, 2);
            assert_eq!(observed.presented, 1);
            observed.acknowledgements.push(token);
            Ok(())
        }
        fn native_surface_handle(
            &mut self,
        ) -> Result<Option<oxiview_base::NativeSurfaceHandle>, oxiview_platform::Error> {
            Ok(None)
        }
        fn request_close(&mut self) -> Result<(), oxiview_platform::Error> {
            Ok(())
        }
    }
    struct App(Arc<Mutex<Observed>>);
    impl Application for App {
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
            let mut scene = Scene::new();
            let root = scene
                .insert(None, 0, oxiview_scene::NodeKind::Opacity(1.0))
                .unwrap();
            Ok(PaintedScene { scene, root })
        }
        fn presented(&mut self, _: Option<LatencyRecord>) {
            self.0.lock().unwrap().presented += 1;
        }
    }
    struct Skipper(Arc<Mutex<Observed>>);
    impl Presenter for Skipper {
        fn configure(&mut self, _: PhysicalSize, _: f64, _: bool) -> Result<(), Error> {
            Ok(())
        }
        fn present(&mut self, _: &PaintedScene) -> Result<PresentationOutcome, Error> {
            let mut state = self.0.lock().unwrap();
            state.attempts += 1;
            Ok(if state.attempts == 1 {
                PresentationOutcome::Skipped
            } else {
                PresentationOutcome::Presented
            })
        }
    }
    let token = ResizeToken::new(WindowId::new(1), 1);
    let observed = Arc::new(Mutex::new(Observed::default()));
    let mut endpoint = Endpoint {
        requests: 0,
        observed: observed.clone(),
        events: VecDeque::from([
            WindowEvent::Resized {
                display_sync: false,
                physical_size: PhysicalSize {
                    width: 10,
                    height: 10,
                },
                scale_factor: 1.0,
                token,
                predicted_presentation_timestamp: Duration::from_millis(10),
                refresh_interval: Duration::from_millis(10),
            },
            WindowEvent::Frame {
                predicted_presentation_timestamp: Duration::from_millis(20),
                refresh_interval: Duration::from_millis(10),
            },
            WindowEvent::Destroyed,
        ]),
    };
    window_loop(
        &mut endpoint,
        &mut App(observed.clone()),
        &mut Skipper(observed.clone()),
    )
    .unwrap();
    assert_eq!(endpoint.requests, 1);
    assert_eq!(observed.lock().unwrap().acknowledgements, vec![token]);
}
