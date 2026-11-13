//! Unstable headless adapter for the platform context (M1 S2).
//!
//! No rendering or scheduling of application work occurs here. A shared
//! foundation ManualClock starts at zero and advances only through Driver.
//! The mutex makes clock changes and event/deadline decisions atomic across
//! threads; ManualScheduler is unnecessary because there are no application
//! tasks, and no randomness is consumed.
//!
//! Vsync boundaries are positive multiples of the configured refresh interval.
//! An armed request produces one event at the first boundary strictly after the
//! request, with predicted presentation equal to that boundary (zero latency).
//! Advancing over several boundaries never produces unsolicited frames.
//!
//! The creating thread is the simulated main thread. Safe portable std cannot
//! identify the process main thread; native backends must enforce that stronger
//! requirement. All fallible operations reject poisoned state. The infallible
//! port clock accessor reads the last stored clock even after poison.
//! Driver calls are serialized; concurrent producer order follows mutex order.

use oxiview_base::{Clock, ManualClock, NativeSurfaceHandle, Point};
use oxiview_platform::*;
use std::{
    collections::VecDeque,
    marker::PhantomData,
    rc::Rc,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread::{self, ThreadId},
    time::Duration,
};

/// Failures on the unstable test-driver surface.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// Shared state was poisoned.
    #[error("headless state lock poisoned")]
    Poisoned,
    /// Invalid clock, scale, identity, or exhausted counter.
    #[error("invalid headless operation: {0}")]
    Invalid(&'static str),
    /// Port operation failed.
    #[error(transparent)]
    Platform(#[from] Error),
}
type Result<T> = std::result::Result<T, DriverError>;

impl From<DriverError> for Error {
    fn from(value: DriverError) -> Self {
        match value {
            DriverError::Platform(error) => error,
            other => Self::OperationFailed(other.to_string()),
        }
    }
}

/// Final status of a resize transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeOutcome {
    /// Presentation was acknowledged.
    Acknowledged,
    /// A newer resize or destruction replaced this transaction.
    Superseded,
    /// The virtual deadline elapsed.
    TimedOut,
}

/// Per-window cumulative counters, retained after destruction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Frame events enqueued.
    pub frames: u64,
    /// Successful resize acknowledgements.
    pub resize_acknowledgements: u64,
    /// Virtual resize deadlines reached.
    pub resize_timeouts: u64,
}

/// Blocking operation observable by the driver's handshake.
#[derive(Clone, Copy, Debug)]
pub enum WaitKind {
    /// An endpoint is sleeping in either event wait.
    Event,
    /// A driver is sleeping waiting for a resize outcome.
    Resize,
}

struct Resize {
    token: ResizeToken,
    deadline: Duration,
    outcome: Option<ResizeOutcome>,
}
struct Window {
    id: WindowId,
    size: PhysicalSize,
    scale: f64,
    timeout: Duration,
    events: VecDeque<WindowEvent>,
    resizes: Vec<Resize>,
    alive: bool,
    endpoint: bool,
    frame: Option<Duration>,
    wake: bool,
    counters: Counters,
    event_waiters: usize,
    resize_waiters: usize,
}
struct State {
    clock: ManualClock,
    interval: Duration,
    windows: Vec<Window>,
    initialized: bool,
    exiting: bool,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}
impl Shared {
    fn lock(&self) -> Result<MutexGuard<'_, State>> {
        self.state.lock().map_err(|_| DriverError::Poisoned)
    }
    fn wait<'a>(&self, guard: MutexGuard<'a, State>) -> Result<MutexGuard<'a, State>> {
        self.changed.wait(guard).map_err(|_| DriverError::Poisoned)
    }
}
fn window(state: &mut State, id: WindowId) -> Result<&mut Window> {
    state
        .windows
        .iter_mut()
        .find(|w| w.id == id)
        .ok_or(DriverError::Invalid("unknown window"))
}
fn live(w: &Window) -> Result<()> {
    if w.alive {
        Ok(())
    } else {
        Err(Error::WindowDestroyed.into())
    }
}
fn capabilities() -> Capabilities {
    Capabilities {
        frame_policies: vec![FramePolicy::OnDemand],
        native_surface: false,
    }
}
fn destroy(w: &mut Window) {
    if w.alive {
        w.alive = false;
        w.frame = None;
        w.wake = false;
        w.events.clear();
        w.events.push_back(WindowEvent::Destroyed);
        for resize in &mut w.resizes {
            if resize.outcome.is_none() {
                resize.outcome = Some(ResizeOutcome::Superseded);
            }
        }
    }
}
fn resize(w: &mut Window, now: Duration) -> Result<ResizeToken> {
    let deadline = now
        .checked_add(w.timeout)
        .ok_or(DriverError::Invalid("resize deadline overflow"))?;
    let sequence = u64::try_from(w.resizes.len())
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or(DriverError::Invalid("resize sequence exhausted"))?;
    let token = ResizeToken::new(w.id, sequence);
    for old in &mut w.resizes {
        if old.outcome.is_none() {
            old.outcome = Some(ResizeOutcome::Superseded);
        }
    }
    w.resizes.push(Resize {
        token,
        deadline,
        outcome: None,
    });
    w.events.push_back(WindowEvent::Resized {
        physical_size: w.size,
        scale_factor: w.scale,
        token,
    });
    Ok(token)
}

/// Simulated main-thread event loop. Construct and run on the same thread.
pub struct HeadlessPlatform {
    shared: Arc<Shared>,
    owner: ThreadId,
    affinity: PhantomData<Rc<()>>,
}
/// Cloneable controller; can be used on the main thread or a controlling thread.
#[derive(Clone)]
pub struct Driver {
    shared: Arc<Shared>,
}

impl HeadlessPlatform {
    /// Creates a backend and controller with a nonzero virtual refresh interval.
    pub fn new(refresh_interval: Duration) -> Result<(Self, Driver)> {
        if refresh_interval.is_zero() {
            return Err(DriverError::Invalid("zero refresh interval"));
        }
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                clock: ManualClock::default(),
                interval: refresh_interval,
                windows: Vec::new(),
                initialized: false,
                exiting: false,
            }),
            changed: Condvar::new(),
        });
        Ok((
            Self {
                shared: shared.clone(),
                owner: thread::current().id(),
                affinity: PhantomData,
            },
            Driver { shared },
        ))
    }
}
impl Platform for HeadlessPlatform {
    fn run(
        self,
        initialize: impl FnOnce(&mut dyn EventLoopContext) -> std::result::Result<(), Error>,
    ) -> std::result::Result<(), Error> {
        if thread::current().id() != self.owner {
            return Err(Error::MainThreadRequired);
        }
        let mut context = Context {
            shared: self.shared.clone(),
        };
        let mut initialized = initialize(&mut context);
        if initialized.is_err() {
            context.request_exit()?;
        }
        let mut state = self.shared.lock()?;
        state.initialized = true;
        let now = state.clock.now();
        for w in &mut state.windows {
            if w.alive
                && let Err(error) = resize(w, now)
            {
                initialized = Err(error.into());
                break;
            }
        }
        if initialized.is_err() {
            state.exiting = true;
            for w in &mut state.windows {
                destroy(w);
            }
        }
        self.shared.changed.notify_all();
        while state.windows.iter().any(|w| w.alive || w.endpoint) {
            state = self.shared.wait(state)?;
        }
        initialized
    }
}
struct Context {
    shared: Arc<Shared>,
}
impl EventLoopContext for Context {
    fn capabilities(&self) -> Capabilities {
        capabilities()
    }
    fn create_window(
        &mut self,
        attributes: WindowAttributes,
    ) -> std::result::Result<CreatedWindow, Error> {
        let width = attributes.logical_size.width();
        let height = attributes.logical_size.height();
        if attributes.title.contains('\0')
            || !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
            || width > f64::from(u32::MAX)
            || height > f64::from(u32::MAX)
            || attributes.resize_timeout.is_zero()
        {
            return Err(Error::InvalidWindowAttributes(
                "invalid title, size, or resize timeout".into(),
            ));
        }
        let mut state = self.shared.lock()?;
        if state.exiting {
            return Err(Error::OperationFailed("event loop exiting".into()));
        }
        state
            .clock
            .now()
            .checked_add(attributes.resize_timeout)
            .ok_or_else(|| Error::InvalidWindowAttributes("resize timeout overflow".into()))?;
        let raw = u64::try_from(state.windows.len())
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| Error::OperationFailed("window identities exhausted".into()))?;
        let id = WindowId::new(raw);
        state.windows.push(Window {
            id,
            size: PhysicalSize {
                width: width.round() as u32,
                height: height.round() as u32,
            },
            scale: 1.0,
            timeout: attributes.resize_timeout,
            events: VecDeque::new(),
            resizes: Vec::new(),
            alive: true,
            endpoint: true,
            frame: None,
            wake: false,
            counters: Counters::default(),
            event_waiters: 0,
            resize_waiters: 0,
        });
        Ok(CreatedWindow {
            main: Box::new(Main {
                shared: self.shared.clone(),
                id,
                affinity: PhantomData,
            }),
            endpoint: Box::new(Endpoint {
                shared: self.shared.clone(),
                id,
            }),
        })
    }
    fn request_exit(&mut self) -> std::result::Result<(), Error> {
        let mut state = self.shared.lock()?;
        state.exiting = true;
        for w in &mut state.windows {
            destroy(w);
        }
        self.shared.changed.notify_all();
        Ok(())
    }
}
struct Main {
    shared: Arc<Shared>,
    id: WindowId,
    affinity: PhantomData<Rc<()>>,
}
fn close(shared: &Shared, id: WindowId) -> Result<()> {
    let mut state = shared.lock()?;
    destroy(window(&mut state, id)?);
    if state.windows.iter().all(|w| !w.alive) {
        state.exiting = true;
    }
    shared.changed.notify_all();
    Ok(())
}
impl MainThreadWindow for Main {
    fn id(&self) -> WindowId {
        self.id
    }
    fn request_close(&mut self) -> std::result::Result<(), Error> {
        Ok(close(&self.shared, self.id)?)
    }
}
struct Endpoint {
    shared: Arc<Shared>,
    id: WindowId,
}
impl Endpoint {
    fn receive(
        &mut self,
        blocking: bool,
        deadline: Option<Duration>,
    ) -> Result<Option<WindowEvent>> {
        let mut state = self.shared.lock()?;
        let mut registered = false;
        let result = loop {
            let now = state.clock.now();
            let w = window(&mut state, self.id)?;
            if let Some(event) = w.events.pop_front() {
                if event == WindowEvent::Wake {
                    w.wake = false;
                }
                break Ok(Some(event));
            }
            if let Err(error) = live(w) {
                break Err(error);
            }
            if !blocking || deadline.is_some_and(|d| now >= d) {
                break Ok(None);
            }
            if !registered {
                w.event_waiters += 1;
                registered = true;
                self.shared.changed.notify_all();
            }
            state = self.shared.wait(state)?;
        };
        if registered {
            window(&mut state, self.id)?.event_waiters -= 1;
        }
        result
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.lock() {
            if let Ok(w) = window(&mut state, self.id) {
                destroy(w);
                w.endpoint = false;
            }
            if state.windows.iter().all(|w| !w.alive) {
                state.exiting = true;
            }
            self.shared.changed.notify_all();
        }
    }
}
impl WindowEndpoint for Endpoint {
    fn id(&self) -> WindowId {
        self.id
    }
    fn capabilities(&self) -> Capabilities {
        capabilities()
    }
    fn now(&self) -> Duration {
        match self.shared.state.lock() {
            Ok(state) => state.clock.now(),
            Err(poison) => poison.into_inner().clock.now(),
        }
    }
    fn wait_event(&mut self) -> std::result::Result<WindowEvent, Error> {
        self.receive(true, None)?.ok_or(Error::EventChannelClosed)
    }
    fn wait_event_until(
        &mut self,
        deadline: Duration,
    ) -> std::result::Result<Option<WindowEvent>, Error> {
        Ok(self.receive(true, Some(deadline))?)
    }
    fn poll_event(&mut self) -> std::result::Result<Option<WindowEvent>, Error> {
        Ok(self.receive(false, None)?)
    }
    fn waker(&self) -> WindowWaker {
        WindowWaker::new(Arc::new(Wake {
            shared: self.shared.clone(),
            id: self.id,
        }))
    }
    fn request_frame(&mut self) -> std::result::Result<(), Error> {
        let mut state = self.shared.lock()?;
        let now = state.clock.now();
        let interval = state.interval;
        let w = window(&mut state, self.id)?;
        live(w)?;
        if w.frame.is_none() {
            let remainder = now.as_nanos() % interval.as_nanos();
            let delta = interval.as_nanos() - remainder;
            let seconds = u64::try_from(delta / 1_000_000_000)
                .map_err(|_| DriverError::Invalid("vsync overflow"))?;
            let offset = Duration::new(seconds, (delta % 1_000_000_000) as u32);
            w.frame = Some(
                now.checked_add(offset)
                    .ok_or(DriverError::Invalid("vsync overflow"))?,
            );
        }
        Ok(())
    }
    fn set_frame_policy(&mut self, policy: FramePolicy) -> std::result::Result<(), Error> {
        let mut state = self.shared.lock()?;
        live(window(&mut state, self.id)?)?;
        if policy != FramePolicy::OnDemand {
            return Err(Error::UnsupportedFramePolicy(policy));
        }
        Ok(())
    }
    fn acknowledge_resize(&mut self, token: ResizeToken) -> std::result::Result<(), Error> {
        let mut state = self.shared.lock()?;
        let w = window(&mut state, self.id)?;
        let transaction = w
            .resizes
            .iter_mut()
            .find(|r| r.token == token && r.outcome.is_none())
            .ok_or(Error::InvalidResizeToken)?;
        transaction.outcome = Some(ResizeOutcome::Acknowledged);
        w.counters.resize_acknowledgements += 1;
        self.shared.changed.notify_all();
        Ok(())
    }
    fn native_surface_handle(&mut self) -> std::result::Result<Option<NativeSurfaceHandle>, Error> {
        let mut state = self.shared.lock()?;
        live(window(&mut state, self.id)?)?;
        Ok(None)
    }
    fn request_close(&mut self) -> std::result::Result<(), Error> {
        Ok(close(&self.shared, self.id)?)
    }
}
struct Wake {
    shared: Arc<Shared>,
    id: WindowId,
}
impl WindowWake for Wake {
    fn wake(&self) -> std::result::Result<(), Error> {
        let mut state = self.shared.lock()?;
        let w = window(&mut state, self.id)?;
        live(w)?;
        if !w.wake {
            w.wake = true;
            w.events.push_back(WindowEvent::Wake);
        }
        self.shared.changed.notify_all();
        Ok(())
    }
}

impl Driver {
    /// Reads virtual time, reporting poison explicitly.
    pub fn now(&self) -> Result<Duration> {
        Ok(self.shared.lock()?.clock.now())
    }
    /// Advances by a duration; overflow and backwards movement are errors.
    pub fn advance_by(&self, duration: Duration) -> Result<()> {
        let mut state = self.shared.lock()?;
        let target = state
            .clock
            .now()
            .checked_add(duration)
            .ok_or(DriverError::Invalid("clock overflow"))?;
        self.advance(&mut state, target)
    }
    /// Advances to an absolute instant, delivering armed frames and expiring resizes.
    pub fn advance_to(&self, target: Duration) -> Result<()> {
        self.advance(&mut *self.shared.lock()?, target)
    }
    fn advance(&self, state: &mut State, target: Duration) -> Result<()> {
        if target < state.clock.now() {
            return Err(DriverError::Invalid("clock cannot move backwards"));
        }
        state.clock.set(target);
        for w in &mut state.windows {
            if let Some(boundary) = w.frame.filter(|b| *b <= target) {
                w.events.push_back(WindowEvent::Frame {
                    predicted_presentation_timestamp: boundary,
                    refresh_interval: state.interval,
                });
                w.frame = None;
                w.counters.frames += 1;
            }
            for r in &mut w.resizes {
                if r.outcome.is_none() && r.deadline <= target {
                    r.outcome = Some(ResizeOutcome::TimedOut);
                    w.counters.resize_timeouts += 1;
                    tracing::warn!(window = ?w.id, token = ?r.token, timeout = ?w.timeout, "headless resize timed out");
                }
            }
        }
        self.shared.changed.notify_all();
        Ok(())
    }
    fn inject(&self, id: WindowId, make: impl FnOnce(Duration) -> WindowEvent) -> Result<()> {
        let mut state = self.shared.lock()?;
        let now = state.clock.now();
        let w = window(&mut state, id)?;
        live(w)?;
        w.events.push_back(make(now));
        self.shared.changed.notify_all();
        Ok(())
    }
    /// Injects pointer motion; None captures the current virtual time.
    pub fn pointer_moved(
        &self,
        id: WindowId,
        position: Point,
        timestamp: Option<Duration>,
    ) -> Result<()> {
        self.inject(id, |now| WindowEvent::PointerMoved {
            position,
            timestamp: timestamp.unwrap_or(now),
        })
    }
    /// Injects a pointer button transition in FIFO order.
    pub fn pointer_button(
        &self,
        id: WindowId,
        button: PointerButton,
        pressed: bool,
        position: Point,
        timestamp: Option<Duration>,
    ) -> Result<()> {
        self.inject(id, |now| WindowEvent::PointerButton {
            button,
            pressed,
            position,
            timestamp: timestamp.unwrap_or(now),
        })
    }
    /// Injects a raw key transition in FIFO order.
    pub fn key(
        &self,
        id: WindowId,
        code: KeyCode,
        pressed: bool,
        repeat: bool,
        timestamp: Option<Duration>,
    ) -> Result<()> {
        self.inject(id, |now| WindowEvent::Key {
            code,
            pressed,
            repeat,
            timestamp: timestamp.unwrap_or(now),
        })
    }
    /// Requests system repaint without arming a frame.
    pub fn request_redraw(&self, id: WindowId) -> Result<()> {
        self.inject(id, |_| WindowEvent::RedrawRequested)
    }
    /// Delivers user close intent without destroying the window.
    pub fn request_user_close(&self, id: WindowId) -> Result<()> {
        self.inject(id, |_| WindowEvent::CloseRequested)
    }
    /// Changes physical size and scale. A changed scale precedes Resized.
    /// Initial resize publication must have completed before calling this method.
    pub fn resize(&self, id: WindowId, size: PhysicalSize, scale: f64) -> Result<ResizeToken> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(DriverError::Invalid("invalid scale factor"));
        }
        let mut state = self.shared.lock()?;
        if !state.initialized {
            return Err(DriverError::Invalid("initialization has not returned"));
        }
        let now = state.clock.now();
        let w = window(&mut state, id)?;
        live(w)?;
        now.checked_add(w.timeout)
            .ok_or(DriverError::Invalid("resize deadline overflow"))?;
        if w.scale != scale {
            w.events.push_back(WindowEvent::ScaleFactorChanged {
                scale_factor: scale,
            });
        }
        w.size = size;
        w.scale = scale;
        let token = resize(w, now)?;
        self.shared.changed.notify_all();
        Ok(token)
    }
    /// Returns a transaction's current outcome, or None while pending.
    pub fn resize_outcome(
        &self,
        id: WindowId,
        token: ResizeToken,
    ) -> Result<Option<ResizeOutcome>> {
        let mut state = self.shared.lock()?;
        window(&mut state, id)?
            .resizes
            .iter()
            .find(|r| r.token == token)
            .map(|r| r.outcome)
            .ok_or(Error::InvalidResizeToken.into())
    }
    /// Blocks the calling main/controlling thread until acknowledgement,
    /// supersession, or virtual timeout; releases the mutex while sleeping.
    pub fn wait_resize(&self, id: WindowId, token: ResizeToken) -> Result<ResizeOutcome> {
        let mut state = self.shared.lock()?;
        let mut registered = false;
        let outcome = loop {
            let w = window(&mut state, id)?;
            let r = w
                .resizes
                .iter()
                .find(|r| r.token == token)
                .ok_or(Error::InvalidResizeToken)?;
            if let Some(outcome) = r.outcome {
                break outcome;
            }
            if !registered {
                w.resize_waiters += 1;
                registered = true;
                self.shared.changed.notify_all();
            }
            state = self.shared.wait(state)?;
        };
        if registered {
            window(&mut state, id)?.resize_waiters -= 1;
        }
        Ok(outcome)
    }
    /// Reads cumulative counters even after window destruction.
    pub fn counters(&self, id: WindowId) -> Result<Counters> {
        Ok(window(&mut *self.shared.lock()?, id)?.counters)
    }
    /// Deterministic handshake: waits until the specified operation is blocked.
    /// Call before injecting its release; there is deliberately no wall-clock timeout.
    pub fn wait_until_blocked(&self, id: WindowId, kind: WaitKind) -> Result<()> {
        let mut state = self.shared.lock()?;
        loop {
            let w = window(&mut state, id)?;
            let count = match kind {
                WaitKind::Event => w.event_waiters,
                WaitKind::Resize => w.resize_waiters,
            };
            if count > 0 {
                return Ok(());
            }
            live(w)?;
            state = self.shared.wait(state)?;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
