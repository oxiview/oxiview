use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEvent, NSView,
    NSViewLayerContentsRedrawPolicy, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSSize, NSString,
    ns_string,
};
use objc2_quartz_core::{CADisplayLink, CAMetalLayer, CATransaction};
use oxiview_base::{NativeSurfaceHandle, Point};
use oxiview_platform::*;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    ffi::c_void,
    num::NonZeroUsize,
    rc::Rc,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

// Core Foundation's source is explicitly registered in both the ordinary common
// modes and our resize-only mode. The latter services commands/display links
// without recursively dispatching AppKit input or committing its resize transaction.
#[repr(C)]
struct SourceContext {
    version: isize,
    info: *mut c_void,
    retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<unsafe extern "C" fn(*const c_void)>,
    description: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    equal: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> u8>,
    hash: Option<unsafe extern "C" fn(*const c_void) -> usize>,
    schedule: Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void)>,
    cancel: Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void)>,
    perform: Option<unsafe extern "C" fn(*mut c_void)>,
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopSourceCreate(
        allocator: *const c_void,
        order: isize,
        context: *mut SourceContext,
    ) -> *mut c_void;
    fn CFRunLoopAddSource(runloop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopSourceInvalidate(source: *mut c_void);
    fn CFRelease(object: *const c_void);
    fn CFRunLoopSourceSignal(source: *mut c_void);
    fn CFRunLoopWakeUp(runloop: *mut c_void);
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, once: u8) -> i32;
    static kCFRunLoopCommonModes: *const c_void;
}
#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    fn CACurrentMediaTime() -> f64;
}
fn native_time() -> Duration {
    // SAFETY: CACurrentMediaTime is a thread-safe, argument-free monotonic clock.
    seconds(unsafe { CACurrentMediaTime() })
}
fn seconds(value: f64) -> Duration {
    Duration::try_from_secs_f64(value).unwrap_or_default()
}
// CACurrentMediaTime, NSEvent.timestamp, and CADisplayLink timestamps all use
// seconds of mach absolute uptime. Keep that origin (not process-relative time),
// so input capture timestamps require only seconds-to-Duration conversion.
fn capabilities() -> Capabilities {
    Capabilities {
        frame_policies: vec![FramePolicy::OnDemand],
        native_surface: true,
    }
}
fn failure(message: &str) -> Error {
    Error::OperationFailed(message.into())
}
fn boundary<T>(f: impl FnOnce() -> T) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => value,
        // No callback state is resumed after an unexpected unwind.
        Err(_) => std::process::abort(),
    }
}
struct Queue {
    events: VecDeque<WindowEvent>,
    alive: bool,
    endpoint: bool,
    wake: bool,
    frame: bool,
    #[cfg(debug_assertions)]
    input_check: VecDeque<Option<Point>>,
    close: bool,
    resize: Option<(ResizeToken, Duration)>,
}
struct Shared {
    frames: AtomicU64,
    queue: Mutex<Queue>,
    changed: Condvar,
    source: Arc<RunSource>,
    handle: NativeSurfaceHandle,
    id: WindowId,
}
impl Shared {
    fn lock(&self) -> Result<MutexGuard<'_, Queue>, Error> {
        self.queue
            .lock()
            .map_err(|_| failure("window queue poisoned"))
    }
    fn signal(&self) {
        // SAFETY: The source is retained by this shared Arc and
        // exposed with provenance at construction. Both CF operations are thread-safe.
        unsafe {
            CFRunLoopSourceSignal(std::ptr::with_exposed_provenance_mut(self.source.0));
            CFRunLoopWakeUp(CFRunLoopGetMain());
        }
    }
    fn emit(&self, event: WindowEvent) {
        if let Ok(mut q) = self.lock()
            && q.alive
        {
            q.events.push_back(event);
            self.changed.notify_all();
        }
    }
    fn close(&self) -> Result<(), Error> {
        let mut q = self.lock()?;
        q.close = true;
        drop(q);
        self.signal();
        Ok(())
    }
}
struct Endpoint {
    shared: Arc<Shared>,
    terminal: bool,
}
impl Endpoint {
    fn receive(
        &mut self,
        block: bool,
        deadline: Option<Duration>,
    ) -> Result<Option<WindowEvent>, Error> {
        if self.terminal {
            return Err(Error::WindowDestroyed);
        }
        let mut q = self.shared.lock()?;
        loop {
            if let Some(event) = q.events.pop_front() {
                if event == WindowEvent::Wake {
                    q.wake = false;
                }
                if event == WindowEvent::Destroyed {
                    self.terminal = true;
                }
                return Ok(Some(event));
            }
            if !q.alive {
                return Err(Error::WindowDestroyed);
            }
            if !block {
                return Ok(None);
            }
            q = if let Some(deadline) = deadline {
                let now = native_time();
                if now >= deadline {
                    return Ok(None);
                }
                self.shared
                    .changed
                    .wait_timeout(q, deadline - now)
                    .map_err(|_| failure("window queue poisoned"))?
                    .0
            } else {
                self.shared
                    .changed
                    .wait(q)
                    .map_err(|_| failure("window queue poisoned"))?
            };
        }
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        if let Ok(mut q) = self.shared.lock() {
            q.endpoint = false;
            q.close = true;
        }
        self.shared.signal();
    }
}
impl WindowWake for Shared {
    fn wake(&self) -> Result<(), Error> {
        let mut q = self.lock()?;
        if !q.alive || !q.endpoint {
            return Err(Error::WindowDestroyed);
        }
        if !q.wake {
            q.wake = true;
            q.events.push_back(WindowEvent::Wake);
            self.changed.notify_all();
        }
        Ok(())
    }
}
impl WindowEndpoint for Endpoint {
    fn id(&self) -> WindowId {
        self.shared.id
    }
    fn capabilities(&self) -> Capabilities {
        capabilities()
    }
    fn now(&self) -> Duration {
        native_time()
    }
    fn wait_event(&mut self) -> Result<WindowEvent, Error> {
        self.receive(true, None)?.ok_or(Error::EventChannelClosed)
    }
    fn wait_event_until(&mut self, deadline: Duration) -> Result<Option<WindowEvent>, Error> {
        self.receive(true, Some(deadline))
    }
    fn poll_event(&mut self) -> Result<Option<WindowEvent>, Error> {
        self.receive(false, None)
    }
    fn waker(&self) -> WindowWaker {
        WindowWaker::new(self.shared.clone())
    }
    fn request_frame(&mut self) -> Result<(), Error> {
        let mut q = self.shared.lock()?;
        if !q.alive {
            return Err(Error::WindowDestroyed);
        }
        q.frame = true;
        drop(q);
        self.shared.signal();
        Ok(())
    }
    fn set_frame_policy(&mut self, policy: FramePolicy) -> Result<(), Error> {
        if !self.shared.lock()?.alive {
            return Err(Error::WindowDestroyed);
        }
        if policy != FramePolicy::OnDemand {
            return Err(Error::UnsupportedFramePolicy(policy));
        }
        Ok(())
    }
    fn acknowledge_resize(&mut self, token: ResizeToken) -> Result<(), Error> {
        let mut q = self.shared.lock()?;
        if !q
            .resize
            .is_some_and(|(pending, deadline)| pending == token && native_time() < deadline)
        {
            return Err(Error::InvalidResizeToken);
        }
        q.resize = None;
        drop(q);
        self.shared.signal();
        Ok(())
    }
    fn native_surface_handle(&mut self) -> Result<Option<NativeSurfaceHandle>, Error> {
        if !self.shared.lock()?.alive {
            return Err(Error::WindowDestroyed);
        }
        Ok(Some(self.shared.handle))
    }
    fn request_close(&mut self) -> Result<(), Error> {
        self.shared.close()
    }
}
struct Main {
    shared: Arc<Shared>,
    _affinity: std::marker::PhantomData<Rc<()>>,
}
impl MainThreadWindow for Main {
    fn id(&self) -> WindowId {
        self.shared.id
    }
    fn request_close(&mut self) -> Result<(), Error> {
        self.shared.close()?;
        service();
        Ok(())
    }
}
struct Native {
    window: Retained<NSWindow>,
    view: Retained<ContentView>,
    _delegate: Retained<Delegate>,
    layer: Retained<CAMetalLayer>,
    link: Retained<CADisplayLink>,
    shared: Arc<Shared>,
    timeout: Duration,
    sequence: Cell<u64>,
    scale: Cell<f64>,
    initial: Cell<bool>,
    interval: Cell<Duration>,
    target: Cell<Duration>,
    closed: Cell<bool>,
}
thread_local! { static WINDOWS: RefCell<Vec<Rc<Native>>> = const { RefCell::new(Vec::new()) }; }
fn natives() -> Vec<Rc<Native>> {
    WINDOWS.with(|w| w.borrow().clone())
}
fn find(id: WindowId) -> Option<Rc<Native>> {
    natives().into_iter().find(|w| w.shared.id == id)
}
fn service() {
    let windows = natives();
    for w in &windows {
        let (close, endpoint, frame) = match w.shared.lock() {
            Ok(q) => (q.close, q.endpoint, q.frame),
            Err(_) => std::process::abort(),
        };
        #[cfg(debug_assertions)]
        {
            let commands = match w.shared.lock() {
                Ok(mut q) => std::mem::take(&mut q.input_check),
                Err(_) => std::process::abort(),
            };
            for position in commands {
                if let Some(position) = position {
                    let point =
                        NSPoint::new(position.x(), w.view.bounds().size.height - position.y());
                    // A real NSEvent goes through AppKit's event queue and the
                    // view callback, exercising capture time and coordinate mapping.
                    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                        objc2_app_kit::NSEventType::MouseMoved, point,
                        objc2_app_kit::NSEventModifierFlags::empty(), native_time().as_secs_f64(),
                        w.window.windowNumber(), None, 0, 0, 0.0);
                    if let Some(event) = event {
                        NSApplication::sharedApplication(w.view.mtm())
                            .postEvent_atStart(&event, false);
                    }
                } else {
                    w.window.performClose(None);
                }
            }
        }
        if close && !w.closed.replace(true) {
            w.link.setPaused(true);
            if let Ok(mut q) = w.shared.lock() {
                q.alive = false;
                q.frame = false;
                q.wake = false;
                q.resize = None;
                q.events.clear();
                q.events.push_back(WindowEvent::Destroyed);
                w.shared.changed.notify_all();
            }
            // Hide first; keep the content view attached until endpoint teardown.
            w.window.orderOut(None);
        }
        if w.closed.get() && !endpoint {
            w.link.invalidate();
            w.window.setDelegate(None);
            w.window.close();
        } else if !w.closed.get() {
            w.link.setPaused(!frame);
        }
    }
    if !windows.is_empty()
        && windows
            .iter()
            .all(|w| w.closed.get() && w.shared.lock().is_ok_and(|q| !q.endpoint))
    {
        // Stop is observed after this source returns; a posted event wakes AppKit's
        // nextEvent wait without polling or an idle timer.
        if let Some(mtm) = MainThreadMarker::new() {
            let app = NSApplication::sharedApplication(mtm);
            app.stop(None);
            // SAFETY: Application-defined events carry no native pointers, use
            // documented scalar fields, and are posted only on the main thread.
            let event = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(objc2_app_kit::NSEventType::ApplicationDefined, NSPoint::new(0.0,0.0), objc2_app_kit::NSEventModifierFlags::empty(), 0.0, 0, None, 0, 0, 0);
            if let Some(event) = event {
                app.postEvent_atStart(&event, true);
            }
        }
    }
}
/// # Safety
/// Called by Core Foundation on the registered main run loop; info is unused.
unsafe extern "C" fn perform(_: *mut c_void) {
    boundary(service);
}
fn resize_mode() -> &'static NSString {
    ns_string!("oxiview.resize")
}
struct RunSource(usize);
impl Drop for RunSource {
    fn drop(&mut self) {
        // SAFETY: This is the last Arc owning the source's create retain. CF source
        // invalidation/removal and release are thread-safe; no context pointer is used.
        unsafe {
            let pointer = std::ptr::with_exposed_provenance_mut(self.0);
            CFRunLoopSourceInvalidate(pointer);
            CFRelease(pointer);
        }
    }
}
fn source() -> Result<Arc<RunSource>, Error> {
    let mut context = SourceContext {
        version: 0,
        info: std::ptr::null_mut(),
        retain: None,
        release: None,
        description: None,
        equal: None,
        hash: None,
        schedule: None,
        cancel: None,
        perform: Some(perform),
    };
    // SAFETY: Exact CFRunLoopSourceContext ABI; callback has process lifetime and
    // no context pointer. CF copies the context. Main run loop and constant modes
    // live for the process lifetime. The shared Arc retains the source through all endpoint/control uses;
    // the callback only consults main-thread local state.
    unsafe {
        let source = CFRunLoopSourceCreate(std::ptr::null(), 0, &mut context);
        if source.is_null() {
            return Err(failure("run-loop source allocation failed"));
        }
        CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
        CFRunLoopAddSource(
            CFRunLoopGetMain(),
            source,
            (resize_mode() as *const NSString).cast(),
        );
        Ok(Arc::new(RunSource(source.expose_provenance())))
    }
}
fn top_left(point: NSPoint, height: f64) -> Point {
    Point::new(point.x, height - point.y)
}
fn physical(size: NSSize, scale: f64) -> PhysicalSize {
    PhysicalSize {
        width: (size.width * scale).round().max(0.0) as u32,
        height: (size.height * scale).round().max(0.0) as u32,
    }
}
fn resize(w: &Native) {
    if w.initial.get() || w.closed.get() {
        return;
    }
    let scale = w.window.backingScaleFactor();
    let bounds = w.view.bounds();
    let size = physical(bounds.size, scale);
    // Core Animation synchronizes individual native property accesses. Geometry
    // changes precede publication of the resize event; worker configure follows
    // that event. Each window-thread drawable acquisition follows an event
    // published by this main thread (Resized or a display-link Frame), observing
    // the flag set before publication. wgpu-hal 30.0.2 metal/surface.rs,
    // acquire_texture, snapshots presentsWithTransaction after nextDrawable.
    // Exception: after a resize timeout, a drawable acquired with the flag on
    // may present late, after this transaction commits; trace that risk below.
    // A hosting view owns layer geometry. Keep this nested transaction open
    // through acknowledgement, merging it into AppKit's enclosing transaction.
    // Disable implicit animations so old contents never animate between sizes.
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    w.layer.setPresentsWithTransaction(true);
    w.layer.setFrame(bounds);
    w.layer.setContentsScale(scale);
    w.layer
        .setDrawableSize(NSSize::new(f64::from(size.width), f64::from(size.height)));
    if w.scale.replace(scale) != scale {
        w.shared.emit(WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });
    }
    let Some(sequence) = w.sequence.get().checked_add(1) else {
        std::process::abort()
    };
    w.sequence.set(sequence);
    let token = ResizeToken::new(w.shared.id, sequence);
    let now = native_time();
    let Some(deadline) = now.checked_add(w.timeout) else {
        std::process::abort()
    };
    if let Ok(mut q) = w.shared.lock() {
        q.resize = Some((token, deadline));
    }
    // A paused link has no fresh target; use now + last known refresh interval.
    let predicted = if w.target.get() > now {
        w.target.get()
    } else {
        now.saturating_add(w.interval.get())
    };
    w.shared.emit(WindowEvent::Resized {
        physical_size: size,
        scale_factor: scale,
        token,
        predicted_presentation_timestamp: predicted,
        refresh_interval: w.interval.get(),
    });
    loop {
        service();
        let pending = w
            .shared
            .lock()
            .is_ok_and(|q| q.resize.is_some_and(|(t, _)| t == token));
        if !pending {
            break;
        }
        let now = native_time();
        if now >= deadline {
            if let Ok(mut q) = w.shared.lock()
                && q.resize.is_some_and(|(t, _)| t == token)
            {
                q.resize = None;
            }
            tracing::warn!(window = ?w.shared.id, ?token, timeout = ?w.timeout, "macOS resize timed out; a transaction-enabled drawable may present late");
            break;
        }
        // SAFETY: Called only on the main thread, with a process-lifetime mode
        // string toll-free bridged to CFString. This mode contains our source and
        // display link, keeping commands and retry frames serviceable while waiting.
        unsafe {
            CFRunLoopRunInMode(
                (resize_mode() as *const NSString).cast(),
                (deadline - now).as_secs_f64(),
                1,
            );
        }
    }
    w.layer.setPresentsWithTransaction(false);
    CATransaction::commit();
}
#[derive(Default)]
struct ViewState {
    id: Cell<Option<WindowId>>,
}
define_class!(
    // SAFETY: NSView supports subclassing; all AppKit access stays on the main thread.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ViewState]
    struct ContentView;
    // SAFETY: NSObjectProtocol imposes no additional invariants.
    unsafe impl NSObjectProtocol for ContentView {}
    impl ContentView {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first(&self) -> bool { true }
        #[unsafe(method(setFrameSize:))]
        fn frame_size(&self, size: NSSize) { boundary(|| {
            // SAFETY: Forward the exact NSView method signature to our superclass.
            unsafe { let _: () = msg_send![super(self), setFrameSize: size]; }
            if let Some(w) = self.ivars().id.get().and_then(find) { resize(&w); }
        }); }
        #[unsafe(method(viewDidChangeBackingProperties))]
        fn backing(&self) { boundary(|| { if let Some(w) = self.ivars().id.get().and_then(find) { resize(&w); } }); }
        #[unsafe(method(tick:))]
        fn tick(&self, link: &CADisplayLink) { boundary(|| {
            link.setPaused(true);
            if let Some(w) = self.ivars().id.get().and_then(find) {
                let interval = seconds(link.duration()); if !interval.is_zero() { w.interval.set(interval); }
                let target = seconds(link.targetTimestamp()); w.target.set(target);
                if let Ok(mut q) = w.shared.lock() && q.alive && q.frame {
                    q.frame = false; w.shared.frames.fetch_add(1, Ordering::Relaxed); q.events.push_back(WindowEvent::Frame { predicted_presentation_timestamp: target, refresh_interval: w.interval.get() }); w.shared.changed.notify_all();
                }
            }
        }); }
        #[unsafe(method(mouseMoved:))]
        fn moved(&self, event: &NSEvent) { boundary(|| self.pointer(event, None)); }
        #[unsafe(method(mouseDragged:))]
        fn dragged(&self, event: &NSEvent) { boundary(|| self.pointer(event, None)); }
        #[unsafe(method(rightMouseDragged:))]
        fn right_dragged(&self, event: &NSEvent) { boundary(|| self.pointer(event, None)); }
        #[unsafe(method(otherMouseDragged:))]
        fn other_dragged(&self, event: &NSEvent) { boundary(|| self.pointer(event, None)); }
        #[unsafe(method(mouseDown:))]
        fn down(&self, event: &NSEvent) { boundary(|| self.pointer(event, Some(true))); }
        #[unsafe(method(mouseUp:))]
        fn up(&self, event: &NSEvent) { boundary(|| self.pointer(event, Some(false))); }
        #[unsafe(method(rightMouseDown:))]
        fn right_down(&self, event: &NSEvent) { boundary(|| self.pointer(event, Some(true))); }
        #[unsafe(method(rightMouseUp:))]
        fn right_up(&self, event: &NSEvent) { boundary(|| self.pointer(event, Some(false))); }
        #[unsafe(method(otherMouseDown:))]
        fn other_down(&self, event: &NSEvent) { boundary(|| self.pointer(event, Some(true))); }
        #[unsafe(method(otherMouseUp:))]
        fn other_up(&self, event: &NSEvent) { boundary(|| self.pointer(event, Some(false))); }
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) { boundary(|| self.key(event, true)); }
        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) { boundary(|| self.key(event, false)); }
    }
);
impl ContentView {
    fn pointer(&self, event: &NSEvent, pressed: Option<bool>) {
        if let Some(w) = self.ivars().id.get().and_then(find) {
            let p = event.locationInWindow();
            let position = top_left(p, self.bounds().size.height);
            let timestamp = seconds(event.timestamp());
            let event = if let Some(pressed) = pressed {
                WindowEvent::PointerButton {
                    button: button(event.buttonNumber()),
                    pressed,
                    position,
                    timestamp,
                }
            } else {
                WindowEvent::PointerMoved {
                    position,
                    timestamp,
                }
            };
            w.shared.emit(event);
        }
    }
    fn key(&self, event: &NSEvent, pressed: bool) {
        if let Some(w) = self.ivars().id.get().and_then(find) {
            w.shared.emit(WindowEvent::Key {
                code: key(event.keyCode()),
                pressed,
                repeat: pressed && event.isARepeat(),
                timestamp: seconds(event.timestamp()),
            });
        }
    }
}
struct DelegateState {
    id: WindowId,
}
define_class!(
    // SAFETY: NSObject subclass with main-thread-only callbacks and owned ivars.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateState]
    struct Delegate;
    // SAFETY: NSObjectProtocol adds no additional invariants.
    unsafe impl NSObjectProtocol for Delegate {}
    // SAFETY: Exact NSWindowDelegate signatures, dispatched on AppKit's main thread.
    unsafe impl NSWindowDelegate for Delegate {
        #[unsafe(method(windowShouldClose:))]
        fn should_close(&self, _: &NSWindow) -> bool {
            boundary(|| {
                if let Some(w) = find(self.ivars().id) {
                    w.shared.emit(WindowEvent::CloseRequested);
                }
                false
            })
        }
    }
);
fn button(code: isize) -> PointerButton {
    match code {
        0 => PointerButton::Primary,
        1 => PointerButton::Secondary,
        2 => PointerButton::Middle,
        n => PointerButton::Other(u16::try_from(n).unwrap_or(u16::MAX)),
    }
}
fn key(code: u16) -> KeyCode {
    match code {
        53 => KeyCode::Escape,
        36 | 76 => KeyCode::Enter,
        49 => KeyCode::Space,
        48 => KeyCode::Tab,
        51 => KeyCode::Backspace,
        123 => KeyCode::ArrowLeft,
        124 => KeyCode::ArrowRight,
        125 => KeyCode::ArrowDown,
        126 => KeyCode::ArrowUp,
        0 => KeyCode::KeyA,
        2 => KeyCode::KeyD,
        1 => KeyCode::KeyS,
        13 => KeyCode::KeyW,
        _ => KeyCode::Unidentified,
    }
}

/// AppKit event-loop entry point. Construct anywhere; run on the process main thread.
#[derive(Default)]
pub struct MacOsPlatform {
    control: MacOsControl,
}
/// Cross-thread close control and display-link event counters for native runs.
#[derive(Clone, Default)]
pub struct MacOsControl(Arc<Mutex<Vec<Arc<Shared>>>>);
impl MacOsControl {
    /// ID of the last drawable acquired by the single-window debug probe.
    #[cfg(debug_assertions)]
    pub fn test_acquired_drawable(&self) -> u64 {
        presentation_probe::ACQUIRED.load(Ordering::Acquire)
    }

    /// Drains on-screen presentation times (Metal host seconds) and drawable IDs
    /// captured by the opt-in OXIVIEW_WINDOW_INPUT_CHECK debug layer.
    /// A positive time means Metal reports the drawable was displayed, not merely
    /// submitted. This process-wide probe is intended for the single-window demo.
    #[cfg(debug_assertions)]
    pub fn test_presented_drawables(&self) -> Result<Vec<(u64, f64)>, Error> {
        presentation_probe::completed()
    }

    /// Requests orderly destruction of all windows from any thread.
    pub fn request_exit(&self) -> Result<(), Error> {
        for window in self
            .0
            .lock()
            .map_err(|_| failure("control poisoned"))?
            .iter()
        {
            window.close()?;
        }
        Ok(())
    }
    /// Posts a synthetic pointer event (Some) or native close-button intent (None).
    /// Debug builds only, for non-interactive native acceptance checks.
    #[cfg(debug_assertions)]
    pub fn post_test_input(&self, position: Option<Point>) -> Result<(), Error> {
        for w in self
            .0
            .lock()
            .map_err(|_| failure("control poisoned"))?
            .iter()
        {
            w.lock()?.input_check.push_back(position);
            w.signal();
        }
        Ok(())
    }
    /// Total display-link frame events delivered in this run.
    pub fn frame_events(&self) -> Result<u64, Error> {
        Ok(self
            .0
            .lock()
            .map_err(|_| failure("control poisoned"))?
            .iter()
            .map(|w| w.frames.load(Ordering::Relaxed))
            .sum())
    }
}
impl MacOsPlatform {
    /// Gets an observation/control handle before entering the main-thread run.
    pub fn control(&self) -> MacOsControl {
        self.control.clone()
    }
}
struct Context {
    control: MacOsControl,
    mtm: MainThreadMarker,
    source: Arc<RunSource>,
    exiting: bool,
}
impl EventLoopContext for Context {
    fn capabilities(&self) -> Capabilities {
        capabilities()
    }
    fn create_window(&mut self, attributes: WindowAttributes) -> Result<CreatedWindow, Error> {
        let existing = natives();
        if self.exiting || (!existing.is_empty() && existing.iter().all(|w| w.closed.get())) {
            return Err(failure("event loop exiting"));
        }
        let size = attributes.logical_size;
        if attributes.title.contains('\0')
            || !size.width().is_finite()
            || !size.height().is_finite()
            || size.width() <= 0.0
            || size.height() <= 0.0
            || size.width() > f64::from(u32::MAX)
            || size.height() > f64::from(u32::MAX)
            || attributes.resize_timeout.is_zero()
            || native_time()
                .checked_add(attributes.resize_timeout)
                .is_none()
        {
            return Err(Error::InvalidWindowAttributes(
                "invalid title, size, or resize timeout".into(),
            ));
        }
        let id = WindowId::new(
            u64::try_from(natives().len())
                .ok()
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| failure("window identities exhausted"))?,
        );
        let rect = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(size.width(), size.height()),
        );
        // SAFETY: Main-thread allocation, validated finite rect, correct initializer;
        // close-time release is disabled because Retained owns the window.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(self.mtm),
                rect,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Resizable
                    | NSWindowStyleMask::Miniaturizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: Retained owns the window; no NSWindowController manages it.
        unsafe {
            window.setReleasedWhenClosed(false);
        }
        let view = ContentView::alloc(self.mtm).set_ivars(ViewState::default());
        // SAFETY: Correct NSView initializer and finite frame for a fresh allocation.
        let view: Retained<ContentView> = unsafe { msg_send![super(view), initWithFrame: rect] };
        #[cfg(debug_assertions)]
        let layer = if std::env::var_os("OXIVIEW_WINDOW_INPUT_CHECK").is_some() {
            presentation_probe::layer()
        } else {
            CAMetalLayer::new()
        };
        #[cfg(not(debug_assertions))]
        let layer = CAMetalLayer::new();
        // Layer-hosting: supply the layer before wantsLayer, so AppKit never
        // replaces it. Never stretch cached content; resize paints physical pixels.
        view.setLayer(Some(&layer));
        view.setWantsLayer(true);
        view.setLayerContentsPlacement(objc2_app_kit::NSViewLayerContentsPlacement::TopLeft);
        view.setLayerContentsRedrawPolicy(NSViewLayerContentsRedrawPolicy::DuringViewResize);
        layer.setFrame(rect);
        // SAFETY: QuartzCore's constant gravity string is process-lifetime, and
        // this layer is still confined to its constructing main thread.
        unsafe {
            layer.setContentsGravity(objc2_quartz_core::kCAGravityTopLeft);
        }
        layer.setContentsScale(window.backingScaleFactor());
        // The window thread has no run loop to commit implicit CA transactions.
        // Only the main-thread resize transaction temporarily enables this flag.
        layer.setPresentsWithTransaction(false);
        window.setContentView(Some(&view));
        window.setAcceptsMouseMovedEvents(true);
        window.setTitle(&NSString::from_str(&attributes.title));
        let pointer = Retained::as_ptr(&layer);
        let address = NonZeroUsize::new(pointer.expose_provenance())
            .ok_or_else(|| failure("null Metal layer"))?;
        // One intentional layer retain per window for the rest of the process;
        // M9 must redesign handle ownership before reclaiming these objects.
        std::mem::forget(layer.clone());
        // SAFETY: Allocated initialized CAMetalLayer, exposed pointer provenance.
        // CAMetalLayer supports off-main-thread drawable creation/presentation and
        // internally synchronized property access, including copied handles. All
        // AppKit operations stay main-thread-only. The window/view remain retained
        // and attached through Destroyed delivery AND endpoint drop (which follows
        // surface release). The intentional retain above keeps even stale copies
        // valid for surface creation/presentation for the entire process lifetime.
        let handle = unsafe { NativeSurfaceHandle::from_macos_metal_layer(address) };
        let shared = Arc::new(Shared {
            frames: AtomicU64::new(0),
            id,
            handle,
            source: self.source.clone(),
            changed: Condvar::new(),
            queue: Mutex::new(Queue {
                events: VecDeque::new(),
                alive: true,
                endpoint: true,
                wake: false,
                frame: false,
                #[cfg(debug_assertions)]
                input_check: VecDeque::new(),
                close: false,
                resize: None,
            }),
        });
        self.control
            .0
            .lock()
            .map_err(|_| failure("control poisoned"))?
            .push(shared.clone());
        view.ivars().id.set(Some(id));
        let delegate = Delegate::alloc(self.mtm).set_ivars(DelegateState { id });
        // SAFETY: NSObject init initializes a fresh delegate allocation.
        let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // SAFETY: macOS 14 is checked before initialization; target implements tick:
        // with the CADisplayLink argument. Registry retains view until invalidation.
        let link = unsafe { view.displayLinkWithTarget_selector(&view, sel!(tick:)) };
        link.setPaused(true);
        // SAFETY: Main run loop, constant NSString modes, link retained by Native.
        unsafe {
            link.addToRunLoop_forMode(
                &NSRunLoop::mainRunLoop(),
                objc2_foundation::NSRunLoopCommonModes,
            );
            link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), resize_mode());
        }
        let native = Rc::new(Native {
            window,
            view,
            _delegate: delegate,
            layer,
            link,
            shared: shared.clone(),
            timeout: attributes.resize_timeout,
            sequence: Cell::new(0),
            scale: Cell::new(1.0),
            initial: Cell::new(true),
            interval: Cell::new(Duration::from_nanos(16_666_667)),
            target: Cell::new(Duration::ZERO),
            closed: Cell::new(false),
        });
        WINDOWS.with(|w| w.borrow_mut().push(native.clone()));
        native.window.center();
        native.window.makeKeyAndOrderFront(None);
        native.window.makeFirstResponder(Some(&native.view));
        Ok(CreatedWindow {
            main: Box::new(Main {
                shared: shared.clone(),
                _affinity: std::marker::PhantomData,
            }),
            endpoint: Box::new(Endpoint {
                shared,
                terminal: false,
            }),
        })
    }
    fn request_exit(&mut self) -> Result<(), Error> {
        self.exiting = true;
        for w in natives() {
            w.shared.close()?;
        }
        service();
        Ok(())
    }
}
impl Platform for MacOsPlatform {
    fn run(
        self,
        initialize: impl FnOnce(&mut dyn EventLoopContext) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mtm = MainThreadMarker::new().ok_or(Error::MainThreadRequired)?;
        if !objc2::available!(macos = 14.0) {
            return Err(failure("macOS 14 or later is required"));
        }
        if !natives().is_empty() {
            return Err(failure("AppKit event loop already running"));
        }
        autoreleasepool(|_| {
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            app.finishLaunching();
            let mut context = Context {
                control: self.control,
                mtm,
                source: source()?,
                exiting: false,
            };
            let result = initialize(&mut context);
            if result.is_err() {
                context.request_exit()?;
            }
            service();
            for w in natives() {
                w.initial.set(false);
                resize(&w);
            }
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
            if natives()
                .iter()
                .any(|w| w.shared.lock().is_ok_and(|q| q.alive || q.endpoint))
            {
                app.run();
            }
            WINDOWS.with(|w| w.borrow_mut().clear());
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_key_subset_and_buttons() {
        for (code, expected) in [
            (53, KeyCode::Escape),
            (36, KeyCode::Enter),
            (49, KeyCode::Space),
            (48, KeyCode::Tab),
            (51, KeyCode::Backspace),
            (123, KeyCode::ArrowLeft),
            (124, KeyCode::ArrowRight),
            (125, KeyCode::ArrowDown),
            (126, KeyCode::ArrowUp),
            (0, KeyCode::KeyA),
            (2, KeyCode::KeyD),
            (1, KeyCode::KeyS),
            (13, KeyCode::KeyW),
            (999, KeyCode::Unidentified),
        ] {
            assert_eq!(key(code), expected);
        }
        assert_eq!(button(0), PointerButton::Primary);
        assert_eq!(button(1), PointerButton::Secondary);
        assert_eq!(button(2), PointerButton::Middle);
        assert_eq!(button(4), PointerButton::Other(4));
    }
    #[test]
    fn coordinates_scale_and_clock_conversion() {
        assert_eq!(
            top_left(NSPoint::new(12.0, 30.0), 100.0),
            Point::new(12.0, 70.0)
        );
        assert_eq!(
            physical(NSSize::new(100.0, 50.0), 2.0),
            PhysicalSize {
                width: 200,
                height: 100
            }
        );
        assert_eq!(
            physical(NSSize::new(0.0, 50.0), 1.5),
            PhysicalSize {
                width: 0,
                height: 75
            }
        );
        assert_eq!(seconds(1234.125), Duration::from_millis(1234125));
        assert_eq!(seconds(f64::NAN), Duration::ZERO);
        assert!(native_time() <= native_time());
    }
    #[test]
    fn rejects_worker_before_appkit_access() {
        assert!(matches!(
            MacOsPlatform::default().run(|_| Ok(())),
            Err(Error::MainThreadRequired)
        ));
    }
}

// Capture the exact drawable returned to wgpu, without acquiring an extra one or
// relying on private wgpu fields. Only the opt-in debug demo uses this subclass.
#[cfg(debug_assertions)]
mod presentation_probe {
    use super::*;
    use objc2::{AnyThread, runtime::AnyObject};

    struct Drawable(Retained<AnyObject>);
    // SAFETY: Only CAMetalDrawable objects enter this wrapper. Metal drawables
    // support cross-thread retention and presentedTime queries; the mutex
    // serializes our accesses and ownership transfers.
    unsafe impl Send for Drawable {}
    static PENDING: Mutex<Vec<Drawable>> = Mutex::new(Vec::new());
    pub(super) static ACQUIRED: AtomicU64 = AtomicU64::new(0);

    define_class!(
        // SAFETY: CAMetalLayer supports subclassing and off-main-thread
        // nextDrawable. No AppKit operations or unsynchronized ivars are added.
        #[unsafe(super = CAMetalLayer)]
        struct ProbeLayer;
        impl ProbeLayer {
            #[unsafe(method_id(nextDrawable))]
            fn next_drawable(&self) -> Option<Retained<AnyObject>> {
                boundary(|| {
                    // SAFETY: Forward the exact object-returning superclass
                    // method; objc2 retains the autoreleased CAMetalDrawable.
                    let drawable: Option<Retained<AnyObject>> = unsafe {
                        msg_send![super(self), nextDrawable]
                    };
                    if let Some(drawable) = &drawable
                        && let Ok(mut pending) = PENDING.lock()
                    {
                        // SAFETY: nextDrawable returns a CAMetalDrawable;
                        // drawableID has NSUInteger ABI and is available on macOS 14.
                        let id: usize = unsafe { msg_send![&**drawable, drawableID] };
                        ACQUIRED.store(id as u64, Ordering::Release);
                        pending.push(Drawable(drawable.clone()));
                    }
                    drawable
                })
            }
        }
    );

    pub(super) fn layer() -> Retained<CAMetalLayer> {
        // SAFETY: Standard NSObject init on a freshly allocated CAMetalLayer
        // subclass with no ivars or additional initialization requirements.
        let layer: Retained<ProbeLayer> = unsafe { msg_send![ProbeLayer::alloc(), init] };
        Retained::into_super(layer)
    }

    pub(super) fn completed() -> Result<Vec<(u64, f64)>, Error> {
        let mut pending = PENDING
            .lock()
            .map_err(|_| failure("presentation probe poisoned"))?;
        let mut completed = Vec::new();
        pending.retain(|drawable| {
            // SAFETY: Objects captured from CAMetalLayer.nextDrawable conform
            // to MTLDrawable; these selectors have NSUInteger and CFTimeInterval
            // return types. macOS 14 supports both. Zero means not displayed
            // (or skipped), so only a finite positive time passes the check.
            let (id, time): (usize, f64) = unsafe {
                (
                    msg_send![&*drawable.0, drawableID],
                    msg_send![&*drawable.0, presentedTime],
                )
            };
            if time.is_finite() && time > 0.0 {
                completed.push((id as u64, time));
                false
            } else {
                true
            }
        });
        Ok(completed)
    }
}
