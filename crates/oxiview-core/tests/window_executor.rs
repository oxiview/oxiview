//! Window-owned local execution without a native window, timers, or manual polling.
// Fixture assertions and unwraps identify violated scheduling/lifetime contracts.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use oxiview_base::{NativeSurfaceHandle, Point, Size};
use oxiview_core::{
    Application, Error, FrameTiming, PaintedScene, PointerEventKind, PresentationOutcome,
    Presenter, Scheduler, UiTree, node, run_window,
};
use oxiview_platform::{
    Capabilities, FramePolicy, KeyCode, PhysicalSize, PointerButton, ResizeToken, WindowEndpoint,
    WindowEvent, WindowId, WindowWake, WindowWaker,
};
use oxiview_reactive::{ReactiveError, Signal, signal, spawn_local};
use oxiview_style::{Length, Position, Style};
use std::{
    cell::Cell,
    collections::VecDeque,
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    time::Duration,
};

#[derive(Default)]
struct Observed {
    events: VecDeque<WindowEvent>,
    wake_pending: bool,
    closed: bool,
    requests: usize,
    frames: Vec<i32>,
    drops: Vec<&'static str>,
    wakes: usize,
}
struct Notify(Arc<Mutex<Observed>>);
impl WindowWake for Notify {
    fn wake(&self) -> Result<(), oxiview_platform::Error> {
        let mut state = self.0.lock().unwrap();
        if state.closed {
            return Err(oxiview_platform::Error::WindowDestroyed);
        }
        state.wakes += 1;
        if !state.wake_pending {
            state.wake_pending = true;
            state.events.push_back(WindowEvent::Wake);
        }
        Ok(())
    }
}
#[derive(Default)]
struct Gate {
    ready: AtomicBool,
    polls: AtomicUsize,
    waker: Mutex<Option<Waker>>,
}
impl Gate {
    fn open(&self) {
        self.ready.store(true, Ordering::Release);
        if let Some(waker) = self.waker.lock().unwrap().as_ref() {
            waker.wake_by_ref();
        }
    }
}
struct WaitGate(Arc<Gate>);
impl Future for WaitGate {
    type Output = ();
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        self.0.polls.fetch_add(1, Ordering::Relaxed);
        if self.0.ready.load(Ordering::Acquire) {
            Poll::Ready(())
        } else {
            *self.0.waker.lock().unwrap() = Some(context.waker().clone());
            Poll::Pending
        }
    }
}
struct Probe(&'static str, Arc<Mutex<Observed>>);
impl Drop for Probe {
    fn drop(&mut self) {
        self.1.lock().unwrap().drops.push(self.0);
    }
}
enum Step {
    Event(WindowEvent),
    Open(Arc<Gate>),
}
struct Endpoint {
    script: VecDeque<Step>,
    observed: Arc<Mutex<Observed>>,
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        let mut observed = self.observed.lock().unwrap();
        observed.closed = true;
        observed.drops.push("endpoint");
    }
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
        WindowWaker::new(Arc::new(Notify(Arc::clone(&self.observed))))
    }
    fn wait_event(&mut self) -> Result<WindowEvent, oxiview_platform::Error> {
        loop {
            {
                let mut observed = self.observed.lock().unwrap();
                if let Some(event) = observed.events.pop_front() {
                    if matches!(event, WindowEvent::Wake) {
                        observed.wake_pending = false;
                    }
                    return Ok(event);
                }
            }
            match self
                .script
                .pop_front()
                .ok_or(oxiview_platform::Error::EventChannelClosed)?
            {
                Step::Event(event) => return Ok(event),
                Step::Open(gate) => gate.open(),
            }
        }
    }
    #[allow(clippy::panic)] // Any timer wait in this event-driven fixture is a test failure.
    fn wait_event_until(
        &mut self,
        _: Duration,
    ) -> Result<Option<WindowEvent>, oxiview_platform::Error> {
        panic!("sleeping local tasks must not install polling deadlines")
    }
    fn poll_event(&mut self) -> Result<Option<WindowEvent>, oxiview_platform::Error> {
        Ok(None)
    }
    fn request_frame(&mut self) -> Result<(), oxiview_platform::Error> {
        let mut observed = self.observed.lock().unwrap();
        observed.requests += 1;
        observed.events.push_back(WindowEvent::Frame {
            predicted_presentation_timestamp: Duration::from_millis(16),
            refresh_interval: Duration::from_millis(16),
        });
        Ok(())
    }
    fn set_frame_policy(&mut self, _: FramePolicy) -> Result<(), oxiview_platform::Error> {
        Ok(())
    }
    fn acknowledge_resize(&mut self, _: ResizeToken) -> Result<(), oxiview_platform::Error> {
        Ok(())
    }
    fn native_surface_handle(
        &mut self,
    ) -> Result<Option<NativeSurfaceHandle>, oxiview_platform::Error> {
        Ok(None)
    }
    fn request_close(&mut self) -> Result<(), oxiview_platform::Error> {
        self.observed
            .lock()
            .unwrap()
            .events
            .push_back(WindowEvent::Destroyed);
        Ok(())
    }
}
struct Output(Arc<Mutex<Observed>>);
impl Drop for Output {
    fn drop(&mut self) {
        self.0.lock().unwrap().drops.push("presenter");
    }
}
impl Presenter for Output {
    fn configure(&mut self, _: PhysicalSize, _: f64, _: bool) -> Result<(), Error> {
        Ok(())
    }
    fn present(&mut self, _: &PaintedScene) -> Result<PresentationOutcome, Error> {
        Ok(PresentationOutcome::Presented)
    }
}
struct App {
    tree: UiTree,
    child: oxiview_core::NodeId,
    value: Signal<i32>,
    observed: Arc<Mutex<Observed>>,
}
impl Application for App {
    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        _: &WindowWaker,
    ) -> Result<(), Error> {
        if matches!(
            event,
            WindowEvent::Key {
                code: KeyCode::Escape,
                pressed: true,
                ..
            }
        ) {
            self.tree.command_sender().remove(self.child)?;
        }
        let result = self
            .tree
            .dispatch(event, Size::new(200.0, 100.0), |_, _, _| {})?;
        if result.changed {
            scheduler.invalidate();
        }
        Ok(())
    }
    fn paint(
        &mut self,
        _: FrameTiming,
        _: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, Error> {
        self.observed.lock().unwrap().frames.push(self.value.get()?);
        Ok(self.tree.paint(Size::new(200.0, 100.0), scale)?)
    }
}
fn resize() -> WindowEvent {
    WindowEvent::Resized {
        display_sync: false,
        physical_size: PhysicalSize {
            width: 200,
            height: 100,
        },
        scale_factor: 1.0,
        predicted_presentation_timestamp: Duration::from_millis(16),
        refresh_interval: Duration::from_millis(16),
        token: ResizeToken::new(WindowId::new(1), 1),
    }
}

#[test]
fn window_automatically_runs_woken_tasks_and_disposes_node_and_window_tasks() {
    let observed = Arc::new(Mutex::new(Observed::default()));
    let initial_gate = Arc::new(Gate::default());
    let node_gate = Arc::new(Gate::default());
    let window_gate = Arc::new(Gate::default());
    let saved_window_signal = Rc::new(Cell::new(None::<Signal<i32>>));
    let saved_node_signal = Rc::new(Cell::new(None::<Signal<i32>>));
    let script = VecDeque::from([
        Step::Event(resize()),
        Step::Open(Arc::clone(&initial_gate)),
        Step::Event(WindowEvent::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
            position: Point::new(10.0, 10.0),
            timestamp: Duration::ZERO,
        }),
        Step::Event(WindowEvent::Wake),
        Step::Event(WindowEvent::Wake),
        Step::Event(WindowEvent::Wake),
        Step::Event(WindowEvent::Key {
            code: KeyCode::Escape,
            pressed: true,
            repeat: false,
            timestamp: Duration::ZERO,
        }),
        Step::Event(WindowEvent::Wake),
        Step::Event(WindowEvent::Destroyed),
    ]);
    let endpoint = Endpoint {
        script,
        observed: Arc::clone(&observed),
    };
    let state = Arc::clone(&observed);
    let task_gate = Arc::clone(&initial_gate);
    let never_gate = Arc::clone(&window_gate);
    let handler_gate = Arc::clone(&node_gate);
    let signal_slot = Rc::clone(&saved_window_signal);
    let node_signal_slot = Rc::clone(&saved_node_signal);
    run_window(Box::new(endpoint), move |_| {
        let value = signal(0)?;
        signal_slot.set(Some(value));
        let probe = Probe("window_task", Arc::clone(&state));
        spawn_local(async move {
            WaitGate(never_gate).await;
            drop(probe);
            Ok(())
        })?;
        let mut tree = UiTree::new()?;
        let task_state = Arc::clone(&state);
        let child = tree.mount(
            tree.root(),
            node()
                .on_mount(move |_| {
                    spawn_local(async move {
                        WaitGate(task_gate).await;
                        value.set(42)?;
                        Ok(())
                    })
                    .unwrap();
                })
                .bind_style(move || {
                    let mut style = Style::default();
                    style.layout.position = Position::Absolute;
                    style.layout.width = Length::Points(80.0 + f64::from(value.get()?));
                    style.layout.height = Length::Points(40.0);
                    Ok(style)
                })
                .on_pointer(move |event, _, _| {
                    if event.kind == PointerEventKind::Down {
                        node_signal_slot.set(Some(signal(7).unwrap()));
                        let gate = Arc::clone(&handler_gate);
                        let probe = Probe("node_task", Arc::clone(&task_state));
                        spawn_local(async move {
                            WaitGate(gate).await;
                            drop(probe);
                            Ok(())
                        })
                        .unwrap();
                    }
                }),
        )?;
        Ok((
            App {
                tree,
                child,
                value,
                observed: Arc::clone(&state),
            },
            Output(state),
        ))
    })
    .unwrap();
    let state = observed.lock().unwrap();
    assert_eq!(state.frames, vec![0, 42, 42]);
    assert_eq!(state.requests, 2);
    assert_eq!(initial_gate.polls.load(Ordering::Relaxed), 2);
    assert_eq!(node_gate.polls.load(Ordering::Relaxed), 1);
    assert_eq!(window_gate.polls.load(Ordering::Relaxed), 1);
    assert_eq!(
        state.drops,
        vec!["node_task", "window_task", "presenter", "endpoint"]
    );
    assert_eq!(
        saved_window_signal.get().unwrap().get(),
        Err(ReactiveError::Disposed)
    );
    assert_eq!(
        saved_node_signal.get().unwrap().get(),
        Err(ReactiveError::Disposed)
    );
    let wakes = state.wakes;
    drop(state);
    node_gate.open();
    window_gate.open();
    assert_eq!(observed.lock().unwrap().wakes, wakes);
}

#[test]
fn a_failed_local_task_uses_normal_window_error_cleanup() {
    let observed = Arc::new(Mutex::new(Observed::default()));
    let endpoint = Endpoint {
        script: VecDeque::new(),
        observed: Arc::clone(&observed),
    };
    let signal_slot = Rc::new(Cell::new(None::<Signal<i32>>));
    let saved = Rc::clone(&signal_slot);
    let state = Arc::clone(&observed);
    let result = run_window(Box::new(endpoint), move |_| {
        let value = signal(0)?;
        saved.set(Some(value));
        spawn_local(async { Err(ReactiveError::User("task failed".into())) })?;
        let tree = UiTree::new()?;
        let child = tree.root();
        Ok((
            App {
                tree,
                child,
                value,
                observed: Arc::clone(&state),
            },
            Output(state),
        ))
    });
    assert!(
        matches!(result, Err(Error::Reactive(ReactiveError::User(message))) if message == "task failed")
    );
    assert_eq!(
        signal_slot.get().unwrap().get(),
        Err(ReactiveError::Disposed)
    );
    assert_eq!(
        observed.lock().unwrap().drops,
        vec!["presenter", "endpoint"]
    );
}
