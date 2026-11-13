use crate::Error;
use oxiview_base::{NativeSurfaceHandle, Point, Size};
use std::sync::Arc;
use std::time::Duration;

/// Opaque identity, unique for the lifetime of an event loop; never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(u64);
impl WindowId {
    /// Creates an identity on the main thread. Backends allocate unique values.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

/// Correlates a resize with presentation at its new size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResizeToken {
    window: WindowId,
    sequence: u64,
}
impl ResizeToken {
    /// Creates a token on the main thread. Never reuse a sequence for this window.
    pub const fn new(window: WindowId, sequence: u64) -> Self {
        Self { window, sequence }
    }
}

/// Window dimensions in physical pixels; zero dimensions mean no drawable area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalSize {
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
}

/// Requested window properties, validated by the backend before creation.
#[derive(Clone, Debug)]
pub struct WindowAttributes {
    /// Window title, without embedded NUL characters.
    pub title: String,
    /// Initial content size in logical units; dimensions must be finite and positive.
    pub logical_size: Size,
    /// Maximum main-thread wait for resize presentation; must be nonzero and finite.
    /// Headless backends measure this against their virtual clock.
    pub resize_timeout: Duration,
}

/// When a window produces frames. Only on-demand is supported in M1.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FramePolicy {
    /// One frame on the next vsync after a request, then silence.
    #[default]
    OnDemand,
    /// Reserved for M10: a whole-window frame every vsync.
    ContinuousWindow,
    /// Reserved for M10: one compositing layer every vsync.
    ContinuousLayer,
}

/// Window capabilities reported explicitly by a backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Supported policies; M1 backends report only `OnDemand`.
    pub frame_policies: Vec<FramePolicy>,
    /// Whether this backend supplies a native presentation object.
    /// Headless backends report false and return no native surface handle.
    pub native_surface: bool,
}

/// A platform-neutral pointer button.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    /// Primary button.
    Primary,
    /// Secondary button.
    Secondary,
    /// Middle button.
    Middle,
    /// Another button, numbered consistently within the backend.
    Other(u16),
}

/// Physical key positions for the M1 demo, without text input or key mapping.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    /// Escape key.
    Escape,
    /// Enter/return key.
    Enter,
    /// Space bar.
    Space,
    /// Tab key.
    Tab,
    /// Backspace key.
    Backspace,
    /// Left arrow.
    ArrowLeft,
    /// Right arrow.
    ArrowRight,
    /// Up arrow.
    ArrowUp,
    /// Down arrow.
    ArrowDown,
    /// Physical A position on a US keyboard.
    KeyA,
    /// Physical D position on a US keyboard.
    KeyD,
    /// Physical S position on a US keyboard.
    KeyS,
    /// Physical W position on a US keyboard.
    KeyW,
    /// A key outside the demo subset, not a native key code.
    Unidentified,
}

/// Ordered event delivered by one window's endpoint.
///
/// All timestamps use the same monotonic backend clock origin as
/// `WindowEndpoint::now`, including virtual vsync in headless execution. Input
/// timestamps describe capture, not delivery. A replay seek starts a new run.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum WindowEvent {
    /// Pointer motion in logical content coordinates.
    PointerMoved {
        /// Position in logical units.
        position: Point,
        /// Monotonic input capture time.
        timestamp: Duration,
    },
    /// Pointer button transition in logical content coordinates.
    PointerButton {
        /// Changed button.
        button: PointerButton,
        /// True for a press, false for a release.
        pressed: bool,
        /// Position in logical units.
        position: Point,
        /// Monotonic input capture time.
        timestamp: Duration,
    },
    /// Raw physical key transition, without IME or key mapping.
    Key {
        /// Physical key.
        code: KeyCode,
        /// True for a press, false for a release.
        pressed: bool,
        /// True only for repeated key presses.
        repeat: bool,
        /// Monotonic input capture time.
        timestamp: Duration,
    },
    /// Scale changed; precedes the corresponding `Resized` event.
    ScaleFactorChanged {
        /// Finite positive physical pixels per logical unit.
        scale_factor: f64,
    },
    /// Resize transaction. Present at this size, then acknowledge its token.
    /// The main thread waits at most `WindowAttributes::resize_timeout` while
    /// keeping the frame/command path serviceable, including during modal resize.
    /// On timeout it proceeds and traces the window, token, and timeout. The
    /// token expires; late or duplicate acknowledgements return an explicit error.
    /// An expired resize remains valid size information. A subsequent resize
    /// supersedes it with a new token. Zero size must be acknowledged after
    /// suspending presentation, without waiting for an impossible drawable.
    Resized {
        /// New content dimensions in physical pixels.
        physical_size: PhysicalSize,
        /// Finite positive physical pixels per logical unit.
        scale_factor: f64,
        /// Acknowledgement identity for this transaction.
        token: ResizeToken,
    },
    /// One requested vsync opportunity. No unsolicited idle frame events.
    Frame {
        /// Predicted display presentation time, not callback or CPU start time.
        predicted_presentation_timestamp: Duration,
        /// Positive display refresh interval for this opportunity; may vary.
        refresh_interval: Duration,
    },
    /// The system needs repainting; application code requests a frame explicitly.
    RedrawRequested,
    /// Cross-thread notification from a `WindowWaker`, delivered on the assigned
    /// window thread. Repeated wakes before delivery coalesce into one event.
    /// Delivery clears the pending wake so a subsequent wake can notify again.
    Wake,
    /// User/system close intent; the application may defer or request close.
    CloseRequested,
    /// Terminal event. Release every renderer surface before dropping the endpoint.
    /// The backend retains native resources until that release and endpoint drop.
    Destroyed,
}

/// Main-thread entry point. Implementations enforce process-main-thread placement.
/// No `Send` bound: native event-loop state may be thread-affine.
pub trait Platform {
    /// Runs and pumps system events on the calling main thread until exit.
    /// Invokes `initialize` exactly once after startup, on that thread. Application
    /// code creates windows there and moves endpoints to their window threads.
    /// Neither the callback nor context needs to be Send. An initialization error
    /// requests orderly shutdown and is returned after resource cleanup.
    fn run(
        self,
        initialize: impl FnOnce(&mut dyn EventLoopContext) -> Result<(), Error>,
    ) -> Result<(), Error>;
}

/// Window creation and event-loop control, accessible only on the main thread.
pub trait EventLoopContext {
    /// Queries support on the main thread; no missing capability is silently emulated.
    fn capabilities(&self) -> Capabilities;

    /// Creates a window on the main thread, returning both thread-specific sides.
    /// The initial size/scale is delivered as `Resized` after initialization returns,
    /// so endpoint transfer cannot deadlock behind the initial resize wait.
    /// Each window starts on-demand, without an automatically requested frame.
    fn create_window(&mut self, attributes: WindowAttributes) -> Result<CreatedWindow, Error>;

    /// Requests orderly event-loop exit on the main thread. Delivers destruction
    /// to all windows and services teardown until their endpoints are dropped.
    /// Closing the last window also exits the loop, so window threads can end a run.
    fn request_exit(&mut self) -> Result<(), Error>;
}

/// Window creation result: retain `main` on the main thread and transfer `endpoint`.
pub struct CreatedWindow {
    /// Native ownership/control side; dropping it alone does not close the window.
    pub main: Box<dyn MainThreadWindow>,
    /// Channel/control side moved once to its window thread before use.
    pub endpoint: Box<dyn WindowEndpoint>,
}

/// Main-thread window handle. Not required to be Send or Sync.
pub trait MainThreadWindow {
    /// Returns identity on the main thread.
    fn id(&self) -> WindowId;
    /// Requests destruction on the main thread; repeated requests are harmless.
    /// The backend delivers `Destroyed` before releasing surface resources.
    fn request_close(&mut self) -> Result<(), Error>;
}

/// Backend implementation of a window's cross-thread wake notification.
///
/// Implementations share pending-wake and lifetime state with the endpoint.
/// All wakers for one window must coalesce together, including concurrent calls.
/// Wake delivery through any endpoint receive method atomically clears the pending
/// notification, so a concurrent wake is either covered by that delivery or queues
/// the next one. Wakes must unblock both event waits without depending on the main
/// thread's resize wait. Headless implementations use the same notification
/// contract without requiring native events or wall-clock time.
pub trait WindowWake: Send + Sync {
    /// Requests one `WindowEvent::Wake` from any thread, without waiting for delivery.
    /// Returns `Error::WindowDestroyed` after window destruction or endpoint drop.
    /// A wake racing destruction may succeed, but destruction may discard pending
    /// wakes; no events are delivered after the terminal `Destroyed` event.
    fn wake(&self) -> Result<(), Error>;
}

/// Owned, cloneable cross-thread notification handle for one window endpoint.
///
/// May be cloned, shared, used, and dropped on any thread (`Send + Sync + Clone`).
/// Clones share one backend implementation and do not keep the window alive.
#[derive(Clone)]
pub struct WindowWaker {
    backend: Arc<dyn WindowWake>,
}

impl WindowWaker {
    /// Wraps a backend implementation on any thread. Backends must associate it
    /// with the endpoint's shared wake and lifetime state; see `WindowWake`.
    pub fn new(backend: Arc<dyn WindowWake>) -> Self {
        Self { backend }
    }

    /// Requests one `WindowEvent::Wake` from any thread, without waiting for delivery.
    /// Repeated wakes before delivery coalesce. Returns `Error::WindowDestroyed`
    /// after window destruction or endpoint drop. Destruction may discard a wake
    /// that succeeded before or concurrently with destruction.
    pub fn wake(&self) -> Result<(), Error> {
        self.backend.wake()
    }
}

/// Backend-provided event receiver and commands, moved to one window thread.
///
/// Send enables the initial transfer only; use all methods on its assigned window
/// thread afterward. No Sync bound is imposed. UI state is created on that thread
/// and may be !Send; it is never stored in this transferable endpoint. Drop it on
/// that thread after releasing all surfaces. Drop also initiates close if needed.
/// Backends must keep frame requests and acknowledgements serviceable while the
/// main thread waits during resize, using a channel independent of that wait.
pub trait WindowEndpoint: Send {
    /// Returns identity on the assigned window thread.
    fn id(&self) -> WindowId;
    /// Queries window support on the assigned window thread.
    fn capabilities(&self) -> Capabilities;
    /// Samples the backend's monotonic clock on the assigned window thread.
    fn now(&self) -> Duration;
    /// Blocks on the assigned window thread until an event arrives, without polling.
    /// After the terminal `Destroyed` event, returns `Error::WindowDestroyed`.
    /// Premature channel disconnection returns `Error::EventChannelClosed`.
    fn wait_event(&mut self) -> Result<WindowEvent, Error>;
    /// Waits on the assigned window thread for an event or an absolute deadline
    /// on the same monotonic backend clock as `now()`, without polling.
    /// Returns an available event first, even if the deadline has already passed;
    /// returns None when `now() >= deadline` and no event is available.
    /// Termination and disconnection follow `wait_event`'s error contract, taking
    /// precedence over timeout. Headless deadlines are reached only by advancing
    /// the virtual clock; never sleep or use a wall-clock timeout to reach them.
    /// Virtual-clock advancement must unblock a wait whose deadline is reached.
    fn wait_event_until(&mut self, deadline: Duration) -> Result<Option<WindowEvent>, Error>;
    /// Obtains a cross-thread waker on the assigned window thread. Every handle
    /// returned for this endpoint shares the same pending notification. Obtaining
    /// a handle after destruction is allowed; waking it returns WindowDestroyed.
    fn waker(&self) -> WindowWaker;
    /// Polls once on the assigned window thread. None means an empty live channel;
    /// termination and disconnection follow `wait_event`'s error contract.
    fn poll_event(&mut self) -> Result<Option<WindowEvent>, Error>;
    /// Arms one frame on the next display (or virtual) vsync, from the window thread.
    /// Repeated requests before delivery coalesce. Delivery disarms the request;
    /// no further frames arrive until rearmed. A resize wait must not block delivery.
    fn request_frame(&mut self) -> Result<(), Error>;
    /// Sets policy on the window thread. Both continuous variants return
    /// `Error::UnsupportedFramePolicy` in M1 without changing state. Reapplying
    /// OnDemand preserves any pending frame request and does not arm a new one.
    fn set_frame_policy(&mut self, policy: FramePolicy) -> Result<(), Error>;
    /// Acknowledges on the window thread only after presenting at the token's size
    /// with the resize transaction (or suspending presentation for zero size).
    /// Returns `Error::InvalidResizeToken` for expired, foreign, or duplicate tokens.
    fn acknowledge_resize(&mut self, token: ResizeToken) -> Result<(), Error>;
    /// Gets the owned surface description on the window thread. Headless returns
    /// None. After destruction returns `Error::WindowDestroyed`.
    fn native_surface_handle(&mut self) -> Result<Option<NativeSurfaceHandle>, Error>;
    /// Requests destruction on the window thread; repeated requests are harmless.
    /// Continue receiving until `Destroyed`, release surfaces, then drop this side.
    fn request_close(&mut self) -> Result<(), Error>;
}
