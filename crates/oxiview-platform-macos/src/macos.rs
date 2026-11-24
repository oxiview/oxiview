use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEvent, NSEventPhase,
    NSTextInputContext, NSTrackingArea, NSTrackingAreaOptions, NSView,
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

mod desktop_services;
#[cfg(feature = "diagnostics")]
mod display_observation;
#[cfg(feature = "diagnostics")]
pub use display_observation::MetalDisplayUpdate;
mod input_dispatch;
mod keyboard;
mod metal_frames;
pub use metal_frames::{MetalDrawable, MetalFrame, MetalFrames, MetalTiming, MetalTimingSample};
#[cfg(feature = "diagnostics")]
mod m5_input_probe;
#[cfg(feature = "diagnostics")]
mod m6_input_probe;
mod preferences;
mod text_input;
mod text_state;
mod utf16;

#[cfg(feature = "diagnostics")]
pub use m5_input_probe::diagnostic_m5_input_probe;
#[cfg(feature = "diagnostics")]
pub use m6_input_probe::diagnostic_m6_input_probe;

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
    use SystemPreference::*;
    Capabilities {
        frame_policies: vec![FramePolicy::OnDemand],
        native_surface: true,
        system_preferences: vec![Appearance, AccentColor, IncreasedContrast, ReducedMotion],
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
#[cfg(feature = "diagnostics")]
struct DiagnosticInput {
    number: isize,
    position: Point,
    capture: Duration,
    delivered: std::sync::mpsc::SyncSender<Option<Duration>>,
}
struct Queue {
    events: VecDeque<(WindowEvent, Option<u64>)>,
    input_sequence: u64,
    platform_flushes: VecDeque<PlatformFlush>,
    alive: bool,
    endpoint: bool,
    wake: bool,
    frame: bool,
    #[cfg(feature = "diagnostics")]
    fullscreen: bool,
    #[cfg(feature = "diagnostics")]
    input_check: VecDeque<DiagnosticInput>,
    #[cfg(feature = "diagnostics")]
    input_receipts:
        std::collections::BTreeMap<isize, std::sync::mpsc::SyncSender<Option<Duration>>>,
    #[cfg(feature = "diagnostics")]
    next_input_number: isize,
    close: bool,
    resize: Option<(ResizeToken, Duration)>,
}

struct PlatformFlush {
    commands: Vec<PlatformCommand>,
    completed_input: Option<u64>,
}
struct Shared {
    frames: AtomicU64,
    feedback: Arc<presentation_probe::Feedback>,
    metal: Option<MetalFrames>,
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
            q.events.push_back((event, None));
            self.changed.notify_all();
        }
    }
    fn emit_input(&self, events: impl IntoIterator<Item = WindowEvent>) {
        if let Ok(mut q) = self.lock()
            && q.alive
        {
            let mut delivered = false;
            for event in events {
                let Some(sequence) = q.input_sequence.checked_add(1) else {
                    std::process::abort();
                };
                q.input_sequence = sequence;
                q.events.push_back((event, Some(sequence)));
                delivered = true;
            }
            if delivered {
                self.changed.notify_all();
            }
        }
    }
    fn deliver_metal(&self, q: &mut Queue) -> bool {
        if !q.alive || q.close || !q.frame {
            return false;
        }
        let Some((target, interval)) = self.metal.as_ref().and_then(MetalFrames::claim) else {
            return false;
        };
        self.feedback
            .interval
            .store(interval.as_nanos() as u64, Ordering::Release);
        q.frame = false;
        self.frames.fetch_add(1, Ordering::Relaxed);
        q.events.push_back((
            WindowEvent::Frame {
                predicted_presentation_timestamp: target,
                refresh_interval: interval,
            },
            None,
        ));
        self.changed.notify_all();
        true
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
    dispatched_frame: Option<Duration>,
    terminal: bool,
    initial_preferences: SystemPreferences,
    completed_input: Option<u64>,
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
        // A display-link drawable can only be accepted while dispatching its
        // event. A clean scheduler or an application close can leave it unused.
        if let Some(metal) = &self.shared.metal
            && let Some(timestamp) = self.dispatched_frame.take()
        {
            metal.discard(timestamp);
        }
        let mut q = self.shared.lock()?;
        loop {
            if let Some((event, input)) = q.events.pop_front() {
                if let Some(input) = input {
                    self.completed_input = Some(input);
                }
                if event == WindowEvent::Wake {
                    q.wake = false;
                }
                if event == WindowEvent::Destroyed {
                    self.terminal = true;
                }
                if self.shared.metal.is_some()
                    && let WindowEvent::Frame {
                        predicted_presentation_timestamp,
                        ..
                    } = &event
                {
                    self.dispatched_frame = Some(*predicted_presentation_timestamp);
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
    fn presentation_ready(&self) -> Result<(), Error> {
        let mut q = self.lock()?;
        if !q.alive || !q.endpoint {
            return Err(Error::WindowDestroyed);
        }
        q.events.push_back((WindowEvent::PresentationReady, None));
        self.changed.notify_all();
        Ok(())
    }
    fn wake(&self) -> Result<(), Error> {
        let mut q = self.lock()?;
        if !q.alive || !q.endpoint {
            return Err(Error::WindowDestroyed);
        }
        if !q.wake {
            q.wake = true;
            q.events.push_back((WindowEvent::Wake, None));
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
    fn editor_capabilities(&self) -> EditorCapabilities {
        EditorCapabilities::all()
    }
    fn preferences(&self) -> SystemPreferences {
        self.initial_preferences
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
        if !self.shared.deliver_metal(&mut q) {
            drop(q);
            self.shared.signal();
        }
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
    fn flush_platform(&mut self, commands: Vec<PlatformCommand>) -> Result<(), Error> {
        if commands.is_empty() && self.completed_input.is_none() {
            return Ok(());
        }
        if commands.iter().any(|command| {
            matches!(command, PlatformCommand::TextInput(Some(snapshot)) if !snapshot.is_valid())
        }) {
            return Err(failure("invalid text input snapshot"));
        }
        let mut q = self.shared.lock()?;
        if !q.alive {
            return Err(Error::WindowDestroyed);
        }
        q.platform_flushes.push_back(PlatformFlush {
            commands,
            completed_input: self.completed_input.take(),
        });
        drop(q);
        self.shared.signal();
        Ok(())
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
    metal_link: RefCell<Option<metal_frames::Link>>,
    metal_idle: Cell<u8>,
    resizing: Cell<bool>,
    transitioning: Cell<bool>,
    shared: Arc<Shared>,
    timeout: Duration,
    sequence: Cell<u64>,
    scale: Cell<f64>,
    preferences: Cell<SystemPreferences>,
    initial: Cell<bool>,
    interval: Cell<Duration>,
    target: Cell<Duration>,
    used_slot: Cell<Duration>,
    closed: Cell<bool>,
    input: input_dispatch::Dispatch,
    panels: desktop_services::Panels,
}
impl Native {
    fn stop_metal_link(&self) {
        if let Some(frames) = &self.shared.metal {
            frames.detach();
            self.metal_link.borrow_mut().take();
            self.shared
                .feedback
                .link_owned
                .store(false, Ordering::Release);
            self.metal_idle.set(0);
        }
    }

    fn service_metal_link(&self, frame: bool) -> bool {
        let Some(frames) = &self.shared.metal else {
            return false;
        };
        if self.initial.get() || self.resizing.get() || self.transitioning.get() {
            return false;
        }
        if !self.window.isVisible()
            || self.window.isMiniaturized()
            || !self
                .window
                .occlusionState()
                .contains(objc2_app_kit::NSWindowOcclusionState::Visible)
        {
            self.stop_metal_link();
            self.link.setPaused(true);
            return true;
        }
        if frame {
            let mut current = self.metal_link.borrow_mut();
            let link = current.get_or_insert_with(|| {
                self.link.setPaused(true);
                self.target.set(Duration::ZERO);
                self.used_slot.set(Duration::ZERO);
                self.shared.feedback.reset();
                if let Some(screen) = self.window.screen() {
                    let fps = screen.maximumFramesPerSecond();
                    if fps > 0 {
                        self.interval.set(Duration::from_secs_f64(1.0 / fps as f64));
                    }
                }
                self.shared
                    .feedback
                    .link_owned
                    .store(true, Ordering::Release);
                frames.attach();
                metal_frames::Link::new(self)
            });
            self.metal_idle.set(0);
            if link.link.isPaused() {
                link.link.setPaused(false);
            }
        }
        true
    }

    fn metal_update(
        &self,
        link: &objc2_quartz_core::CAMetalDisplayLink,
        update: &objc2_quartz_core::CAMetalDisplayLinkUpdate,
    ) {
        let callback = metal_frames::host_time();
        if self.closed.get()
            || self.resizing.get()
            || self.transitioning.get()
            || !self
                .metal_link
                .borrow()
                .as_ref()
                .is_some_and(|source| &*source.link == link)
        {
            return;
        }
        let Some(frames) = &self.shared.metal else {
            return;
        };
        #[cfg(feature = "diagnostics")]
        if let Some(source) = self
            .metal_link
            .borrow()
            .as_ref()
            .and_then(|link| link.observation.as_ref())
        {
            source.record(&self.shared.feedback, update);
        }
        let Ok(mut q) = self.shared.lock() else {
            return;
        };
        if !q.alive || q.close {
            frames.discard_unclaimed();
            link.setPaused(true);
            return;
        }
        if q.frame {
            self.metal_idle.set(0);
        } else {
            let idle = self.metal_idle.get().saturating_add(1);
            self.metal_idle.set(idle);
            if idle >= 2 {
                frames.discard_unclaimed();
                link.setPaused(true);
                return;
            }
        }
        frames.stage(
            update,
            self.interval.get(),
            callback,
            link.preferredFrameLatency(),
            &self.shared.feedback,
        );
        self.shared.deliver_metal(&mut q);
    }

    fn refresh_preferences(&self) {
        if !self.closed.get() {
            let preferences = preferences::snapshot(&self.view);
            if self.preferences.replace(preferences) != preferences {
                self.shared
                    .emit(WindowEvent::PreferencesChanged { preferences });
            }
        }
    }
}
thread_local! {
    static WINDOWS: RefCell<Vec<Rc<Native>>> = const { RefCell::new(Vec::new()) };
    static RESIZE_DEPTH: Cell<usize> = const { Cell::new(0) };
}
fn natives() -> Vec<Rc<Native>> {
    WINDOWS.with(|w| w.borrow().clone())
}
fn find(id: WindowId) -> Option<Rc<Native>> {
    natives().into_iter().find(|w| w.shared.id == id)
}
fn service() {
    let windows = natives();
    for w in &windows {
        let (close, endpoint, frame, editor_work) = match w.shared.lock() {
            Ok(q) => (q.close, q.endpoint, q.frame, !q.platform_flushes.is_empty()),
            Err(_) => std::process::abort(),
        };
        if !w.closed.get() && RESIZE_DEPTH.get() == 0 && (editor_work || w.input.has_pending()) {
            w.flush_editor_commands();
        }
        #[cfg(feature = "diagnostics")]
        {
            let commands = match w.shared.lock() {
                Ok(mut q) => std::mem::take(&mut q.input_check),
                Err(_) => std::process::abort(),
            };
            for command in commands {
                let point = NSPoint::new(
                    command.position.x(),
                    w.view.bounds().size.height - command.position.y(),
                );
                // A real NSEvent goes through AppKit's event queue and the
                // view callback, exercising capture time and coordinate mapping.
                let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                        objc2_app_kit::NSEventType::MouseMoved, point,
                        objc2_app_kit::NSEventModifierFlags::empty(), command.capture.as_secs_f64(),
                        w.window.windowNumber(), None, command.number, 0, 0.0);
                if let Some(event) = event {
                    match w.shared.lock() {
                        Ok(mut q) => {
                            q.input_receipts.insert(command.number, command.delivered);
                        }
                        Err(_) => std::process::abort(),
                    }
                    NSApplication::sharedApplication(w.view.mtm()).postEvent_atStart(&event, false);
                } else {
                    let _ = command.delivered.send(None);
                }
            }
        }
        if close && !w.closed.replace(true) {
            w.stop_metal_link();
            w.link.setPaused(true);
            if let Ok(mut q) = w.shared.lock() {
                q.alive = false;
                q.frame = false;
                q.wake = false;
                q.resize = None;
                q.events.clear();
                q.platform_flushes.clear();
                q.events.push_back((WindowEvent::Destroyed, None));
                w.shared.changed.notify_all();
            }
            w.input.close();
            w.panels.close();
            // Hide first; keep the content view attached until endpoint teardown.
            w.window.orderOut(None);
        }
        if w.closed.get() && !endpoint {
            w.link.invalidate();
            w.window.setDelegate(None);
            w.window.close();
        } else if !w.closed.get() {
            #[cfg(feature = "diagnostics")]
            if w.shared
                .lock()
                .is_ok_and(|mut q| std::mem::take(&mut q.fullscreen))
            {
                w.window.toggleFullScreen(None);
            }
            if w.service_metal_link(frame) {
                continue;
            }
            if frame {
                let slot = next_slot(w, native_time());
                if w.target.get() != Duration::ZERO {
                    deliver(w, slot);
                } else {
                    w.link.setPaused(false);
                }
            } else {
                w.link.setPaused(true);
            }
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
// Extrapolate the display clock while paused; startup uses one real tick to
// establish its phase. Compositor feedback never changes the slot cadence.
fn unused_slot(slot: Duration, used: Duration, interval: Duration) -> bool {
    // Recalibration timestamps may differ by microseconds for the same vsync.
    used.is_zero() || slot.saturating_sub(used) >= interval / 2
}
fn next_slot(w: &Native, now: Duration) -> Duration {
    let anchor = w.target.get();
    let interval = w.interval.get();
    if anchor > now {
        return anchor;
    }
    anchor
        + interval.mul_f64(
            (now.saturating_sub(anchor).as_secs_f64() / interval.as_secs_f64()).floor() + 1.0,
        )
}
fn prediction(w: &Native, slot: Duration) -> Duration {
    w.shared
        .feedback
        .slot
        .store(slot.as_nanos() as u64, Ordering::Release);
    w.shared
        .feedback
        .interval
        .store(w.interval.get().as_nanos() as u64, Ordering::Release);
    slot + w.shared.feedback.offset()
}
fn deliver(w: &Native, slot: Duration) {
    // A queued tick may arrive after an immediate delivery paused the link.
    // Never re-arm it unless there is still pending work.
    if !w.shared.lock().is_ok_and(|q| q.alive && q.frame) {
        w.link.setPaused(true);
        return;
    }
    if !unused_slot(slot, w.used_slot.get(), w.interval.get()) {
        w.link.setPaused(false);
        return;
    }
    let predicted = prediction(w, slot);
    if let Ok(mut q) = w.shared.lock()
        && q.alive
        && q.frame
    {
        q.frame = false;
        w.used_slot.set(slot);
        w.link.setPaused(true);
        q.events.push_back((
            WindowEvent::Frame {
                predicted_presentation_timestamp: predicted,
                refresh_interval: w.interval.get(),
            },
            None,
        ));
        w.shared.changed.notify_all();
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
    let outer_resize = w.resizing.replace(true);
    w.stop_metal_link();
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
    // Only interactive live resize needs transaction-synchronized presentation.
    // AppKit reparents the layer during fullscreen transitions; presenting their
    // final frame with a transaction can discard it and leave the window blank
    // until new input. Normal asynchronous presentation survives that transition.
    CATransaction::begin();
    RESIZE_DEPTH.set(RESIZE_DEPTH.get() + 1);
    CATransaction::setDisableActions(true);
    w.layer.setPresentsWithTransaction(w.view.inLiveResize());
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
    let predicted = prediction(w, next_slot(w, now));
    w.shared.emit(WindowEvent::Resized {
        display_sync: w.window.styleMask().contains(NSWindowStyleMask::FullScreen),
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
    RESIZE_DEPTH.set(RESIZE_DEPTH.get() - 1);
    w.resizing.set(outer_resize);
    if w.shared.metal.is_some() {
        w.shared.signal();
    }
    w.invalidate_text_coordinates();
    if w.input.has_pending()
        || natives().iter().any(|w| {
            w.shared
                .lock()
                .is_ok_and(|q| !q.platform_flushes.is_empty())
        })
    {
        w.shared.signal();
    }
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
        #[unsafe(method_id(inputContext))]
        fn editor_input_context(&self) -> Option<Retained<NSTextInputContext>> {
            boundary(|| self.ivars().id.get().and_then(find).and_then(|w| w.input.context()))
        }
        #[unsafe(method(setFrameSize:))]
        fn frame_size(&self, size: NSSize) { boundary(|| {
            // SAFETY: Forward the exact NSView method signature to our superclass.
            unsafe { let _: () = msg_send![super(self), setFrameSize: size]; }
            if let Some(w) = self.ivars().id.get().and_then(find) { resize(&w); }
        }); }
        #[unsafe(method(viewDidChangeBackingProperties))]
        fn backing(&self) { boundary(|| { if let Some(w) = self.ivars().id.get().and_then(find) { resize(&w); } }); }
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn changed_appearance(&self) { boundary(|| {
            // SAFETY: Forward this NSView override to the exact superclass method.
            unsafe { let _: () = msg_send![super(self), viewDidChangeEffectiveAppearance]; }
            if let Some(w) = self.ivars().id.get().and_then(find) { w.refresh_preferences(); }
        }); }
        #[unsafe(method(tick:))]
        fn tick(&self, link: &CADisplayLink) { boundary(|| {
            link.setPaused(true);
            if let Some(w) = self.ivars().id.get().and_then(find) {
                if w.service_metal_link(false) { return; }
                let interval = seconds(link.duration()); if !interval.is_zero() { w.interval.set(interval); }
                let target = seconds(link.targetTimestamp()); w.target.set(target);
                w.shared.frames.fetch_add(1, Ordering::Relaxed);
                deliver(&w, target);
            }
        }); }
        #[unsafe(method(mouseMoved:))]
        fn moved(&self, event: &NSEvent) { boundary(|| self.pointer(event, None)); }
        #[unsafe(method(mouseEntered:))]
        fn entered(&self, event: &NSEvent) { boundary(|| self.pointer(event, None)); }
        #[unsafe(method(mouseExited:))]
        fn exited(&self, event: &NSEvent) { boundary(|| {
            if let Some(w) = self.ivars().id.get().and_then(find) {
                w.route_input(WindowEvent::PointerLeft { timestamp: seconds(event.timestamp()) });
            }
        }); }
        #[unsafe(method(cancelOperation:))]
        fn interrupted(&self, _: Option<&objc2::runtime::AnyObject>) { boundary(|| {
            if let Some(w) = self.ivars().id.get().and_then(find) {
                w.route_input(WindowEvent::PointerInterrupted { timestamp: native_time() });
            }
        }); }
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
        #[unsafe(method(scrollWheel:))]
        fn wheel(&self, event: &NSEvent) { boundary(|| self.scroll(event)); }
    }
);
impl ContentView {
    fn pointer(&self, event: &NSEvent, pressed: Option<bool>) {
        if let Some(w) = self.ivars().id.get().and_then(find) {
            let p = event.locationInWindow();
            let position = top_left(p, self.bounds().size.height);
            let timestamp = seconds(event.timestamp());
            #[cfg(feature = "diagnostics")]
            let receipt = w
                .shared
                .lock()
                .ok()
                .and_then(|mut q| q.input_receipts.remove(&event.eventNumber()));
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
            w.route_input(event);
            #[cfg(feature = "diagnostics")]
            if let Some(receipt) = receipt {
                // AppKit's queue can quantize NSEvent's floating-point timestamp
                // again. Acknowledge the exact capture value delivered to core.
                // Capacity one prevents this main-thread callback from blocking.
                let _ = receipt.send(Some(timestamp));
            }
        }
    }
    fn key(&self, event: &NSEvent, pressed: bool) {
        if let Some(w) = self.ivars().id.get().and_then(find) {
            w.route_key(event, pressed);
        }
    }
    fn scroll(&self, event: &NSEvent) {
        if let Some(w) = self.ivars().id.get().and_then(find) {
            let position = self.convertPoint_fromView(event.locationInWindow(), None);
            let line_height = if event.hasPreciseScrollingDeltas() {
                1.0
            } else {
                16.0
            };
            // AppKit's deltas move the content, while the port reports viewport movement.
            let delta_x = -event.scrollingDeltaX() * line_height;
            let delta_y = -event.scrollingDeltaY() * line_height;
            if [position.x, position.y, delta_x, delta_y]
                .into_iter()
                .all(f64::is_finite)
            {
                let position = Point::new(position.x, position.y);
                let timestamp = seconds(event.timestamp());
                let input = match scroll_phase(event.phase(), event.momentumPhase()) {
                    Some(phase) => WindowEvent::Scroll {
                        position,
                        delta_x,
                        delta_y,
                        phase,
                        timestamp,
                    },
                    None => WindowEvent::Wheel {
                        position,
                        delta_x,
                        delta_y,
                        timestamp,
                    },
                };
                w.route_input(input);
            }
        }
    }
}

fn scroll_phase(direct: NSEventPhase, momentum: NSEventPhase) -> Option<ScrollPhase> {
    if momentum.intersects(NSEventPhase::Ended | NSEventPhase::Cancelled) {
        Some(ScrollPhase::MomentumEnd)
    } else if momentum.contains(NSEventPhase::Began) {
        Some(ScrollPhase::MomentumBegin)
    } else if !momentum.is_empty() {
        Some(ScrollPhase::MomentumUpdate)
    } else if direct.intersects(NSEventPhase::Cancelled | NSEventPhase::MayBegin) {
        Some(ScrollPhase::Cancel)
    } else if direct.contains(NSEventPhase::Ended) {
        Some(ScrollPhase::End)
    } else if direct.contains(NSEventPhase::Began) {
        Some(ScrollPhase::Begin(ScrollMomentum::Native))
    } else if !direct.is_empty() {
        Some(ScrollPhase::Update)
    } else {
        None
    }
}
struct DelegateState {
    id: WindowId,
    visible: Cell<bool>,
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
        #[unsafe(method(windowDidBecomeKey:))]
        fn became_key(&self, _: &objc2_foundation::NSNotification) {
            boundary(|| {
                self.activation(true);
            });
        }
        #[unsafe(method(windowDidResignKey:))]
        fn resigned_key(&self, _: &objc2_foundation::NSNotification) {
            boundary(|| {
                self.activation(false);
            });
        }
        #[unsafe(method(windowDidMiniaturize:))]
        fn miniaturized(&self, _: &objc2_foundation::NSNotification) {
            boundary(|| {
                self.visibility();
            });
        }
        #[unsafe(method(windowDidDeminiaturize:))]
        fn deminiaturized(&self, _: &objc2_foundation::NSNotification) {
            boundary(|| {
                self.visibility();
            });
        }
        #[unsafe(method(windowDidChangeOcclusionState:))]
        fn occlusion_changed(&self, _: &objc2_foundation::NSNotification) {
            boundary(|| {
                self.visibility();
            });
        }
        #[unsafe(method(windowDidChangeScreen:))]
        fn changed_screen(&self, _: &objc2_foundation::NSNotification) {
            boundary(|| {
                if let Some(w) = find(self.ivars().id) {
                    w.target.set(Duration::ZERO);
                    w.used_slot.set(Duration::ZERO);
                    w.stop_metal_link();
                    w.shared.feedback.reset();
                    resize(&w);
                    // AppKit can discard drawables while reparenting the layer.
                    // Schedule one normal frame after its transition transaction.
                    w.shared.emit(WindowEvent::RedrawRequested);
                }
            });
        }
        #[unsafe(method(windowDidMove:))]
        fn moved(&self, _: &objc2_foundation::NSNotification) {
            boundary(|| {
                if let Some(w) = find(self.ivars().id) {
                    w.invalidate_text_coordinates();
                }
            });
        }
        #[unsafe(method(windowWillEnterFullScreen:))]
        fn will_enter_fullscreen(&self, _: &objc2_foundation::NSNotification) {
            boundary(|| self.begin_fullscreen());
        }
        #[unsafe(method(windowWillExitFullScreen:))]
        fn will_exit_fullscreen(&self, _: &objc2_foundation::NSNotification) {
            boundary(|| self.begin_fullscreen());
        }
        #[unsafe(method(windowDidFailToEnterFullScreen:))]
        fn failed_to_enter_fullscreen(&self, _: &NSWindow) {
            boundary(|| self.finish_failed_fullscreen());
        }
        #[unsafe(method(windowDidFailToExitFullScreen:))]
        fn failed_to_exit_fullscreen(&self, _: &NSWindow) {
            boundary(|| self.finish_failed_fullscreen());
        }
        #[unsafe(method(windowDidEnterFullScreen:))]
        fn entered_fullscreen(&self, _: &objc2_foundation::NSNotification) {
            boundary(|| {
                if let Some(w) = find(self.ivars().id) {
                    w.transitioning.set(false);
                    resize(&w);
                    // AppKit can discard drawables while reparenting the layer.
                    // Schedule one normal frame after its transition transaction.
                    w.shared.emit(WindowEvent::RedrawRequested);
                }
            });
        }
        #[unsafe(method(windowDidExitFullScreen:))]
        fn exited_fullscreen(&self, _: &objc2_foundation::NSNotification) {
            boundary(|| {
                if let Some(w) = find(self.ivars().id) {
                    w.transitioning.set(false);
                    resize(&w);
                    // AppKit can discard drawables while reparenting the layer.
                    // Schedule one normal frame after its transition transaction.
                    w.shared.emit(WindowEvent::RedrawRequested);
                }
            });
        }
        #[unsafe(method(windowShouldClose:))]
        fn should_close(&self, _: &NSWindow) -> bool {
            boundary(|| {
                if let Some(w) = find(self.ivars().id) {
                    w.route_input(WindowEvent::CloseRequested);
                }
                false
            })
        }
    }
);
impl Delegate {
    fn begin_fullscreen(&self) {
        if let Some(w) = find(self.ivars().id)
            && w.shared.metal.is_some()
        {
            w.transitioning.set(true);
            w.stop_metal_link();
        }
    }

    fn finish_failed_fullscreen(&self) {
        if let Some(w) = find(self.ivars().id)
            && w.shared.metal.is_some()
        {
            w.transitioning.set(false);
            w.shared.emit(WindowEvent::RedrawRequested);
            w.shared.signal();
        }
    }
    fn activation(&self, active: bool) {
        if let Some(w) = find(self.ivars().id) {
            w.input_activation(active);
        }
    }

    fn visibility(&self) {
        if let Some(w) = find(self.ivars().id) {
            // Occlusion notifications are only a cue to query true visibility;
            // another window covering this one must not interrupt a drag.
            let visible = w.window.isVisible()
                && !w.window.isMiniaturized()
                && !NSApplication::sharedApplication(self.mtm()).isHidden();
            if w.shared.metal.is_some() {
                w.shared.signal();
            }
            if self.ivars().visible.replace(visible) != visible {
                w.route_input(WindowEvent::VisibilityChanged {
                    visible,
                    timestamp: native_time(),
                });
            }
        }
    }
}
fn button(code: isize) -> PointerButton {
    match code {
        0 => PointerButton::Primary,
        1 => PointerButton::Secondary,
        2 => PointerButton::Middle,
        n => PointerButton::Other(u16::try_from(n).unwrap_or(u16::MAX)),
    }
}
/// AppKit event-loop entry point. Construct anywhere; run on the process main thread.
#[derive(Default)]
pub struct MacOsPlatform {
    control: MacOsControl,
    metal_drawables: bool,
    metal_telemetry: bool,
    #[cfg(feature = "diagnostics")]
    diagnostics: bool,
}
/// Cross-thread close control and display-link event counters for native runs.
#[derive(Clone, Default)]
pub struct MacOsControl(Arc<Mutex<Vec<Arc<Shared>>>>);
impl MacOsControl {
    /// Drains the system's display-link updates, including callbacks without a frame request.
    #[cfg(feature = "diagnostics")]
    pub fn diagnostic_metal_display_updates(&self) -> Result<Vec<MetalDisplayUpdate>, Error> {
        let windows = self.0.lock().map_err(|_| failure("control poisoned"))?;
        let mut updates = Vec::new();
        for window in windows.iter() {
            let mut observed = window
                .feedback
                .display_updates
                .lock()
                .map_err(|_| failure("display observation poisoned"))?;
            updates.append(&mut observed);
        }
        Ok(updates)
    }
    /// Gets the drawable source for an explicitly configured window.
    pub fn metal_frames(&self, id: WindowId) -> Result<Option<MetalFrames>, Error> {
        let windows = self.0.lock().map_err(|_| failure("control poisoned"))?;
        Ok(windows
            .iter()
            .find(|w| w.id == id)
            .and_then(|w| w.metal.clone()))
    }

    /// Drains opt-in raw CAMetalDisplayLink timing for this run.
    pub fn metal_timing(&self) -> Result<Vec<MetalTiming>, Error> {
        let windows = self.0.lock().map_err(|_| failure("control poisoned"))?;
        Ok(windows
            .iter()
            .filter_map(|w| w.metal.as_ref())
            .flat_map(MetalFrames::drain)
            .collect())
    }

    /// ID of the last drawable acquired by the single-window diagnostics probe.
    #[cfg(feature = "diagnostics")]
    pub fn diagnostic_acquired_drawable(&self) -> u64 {
        self.0
            .lock()
            .ok()
            .and_then(|w| {
                w.first()
                    .map(|w| w.feedback.acquired.load(Ordering::Acquire))
            })
            .unwrap_or(0)
    }

    /// Diagnostics only: slot vsync associated with the most recently acquired
    /// drawable, excluding the production compositor prediction offset.
    #[cfg(feature = "diagnostics")]
    pub fn diagnostic_acquired_slot(&self) -> Duration {
        Duration::from_nanos(
            self.0
                .lock()
                .ok()
                .and_then(|w| {
                    w.first()
                        .map(|w| w.feedback.acquired_slot.load(Ordering::Acquire))
                })
                .unwrap_or(0),
        )
    }
    /// Drains on-screen presentation times (Metal host seconds) and drawable IDs
    /// captured by the opt-in diagnostics layer (also in release builds).
    /// A positive time means Metal reports the drawable was displayed, not merely
    /// submitted. Zero is a distinct not-displayed callback record.
    /// This process-wide probe is intended for the single-window demo.
    #[cfg(feature = "diagnostics")]
    pub fn diagnostic_presented_drawables(&self) -> Result<Vec<(u64, f64)>, Error> {
        let windows = self.0.lock().map_err(|_| failure("control poisoned"))?;
        let mut result = Vec::new();
        for w in windows.iter() {
            if let Ok(mut records) = w.feedback.completed.lock() {
                result.extend(records.drain(..).map(|(id, time, _slot)| (id, time)));
            }
        }
        Ok(result)
    }

    /// Diagnostics only: schedule fullscreen toggle on the main thread.
    #[cfg(feature = "diagnostics")]
    pub fn diagnostic_toggle_fullscreen(&self) -> Result<(), Error> {
        for w in self
            .0
            .lock()
            .map_err(|_| failure("control poisoned"))?
            .iter()
        {
            w.lock()?.fullscreen = true;
            w.signal();
        }
        Ok(())
    }
    /// Current production compositor estimate, in host-clock seconds.
    #[cfg(feature = "diagnostics")]
    pub fn diagnostic_compositor_offset(&self) -> f64 {
        self.0
            .lock()
            .ok()
            .and_then(|w| w.first().map(|w| w.feedback.offset().as_secs_f64()))
            .unwrap_or(0.0)
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
    /// Diagnostics only: posts one pointer move through AppKit from a worker thread.
    /// Waits up to five seconds for delivery and returns the capture timestamp
    /// delivered to core on the backend clock. Capture precedes the command;
    /// it includes in-process command and AppKit queue delay, but bypasses HID
    /// and WindowServer input. Calling on the main thread returns an error.
    #[cfg(feature = "diagnostics")]
    pub fn diagnostic_post_pointer(&self, position: Point) -> Result<Duration, Error> {
        if MainThreadMarker::new().is_some() {
            return Err(failure("diagnostic input posting requires a worker thread"));
        }
        let capture = native_time();
        let (delivered, receipt) = std::sync::mpsc::sync_channel(1);
        {
            let windows = self.0.lock().map_err(|_| failure("control poisoned"))?;
            if windows.len() != 1 {
                return Err(failure("diagnostics require exactly one window"));
            }
            let w = &windows[0];
            {
                let mut q = w.lock()?;
                if !q.alive {
                    return Err(Error::WindowDestroyed);
                }
                let number = q.next_input_number;
                q.next_input_number = number
                    .checked_sub(1)
                    .ok_or_else(|| failure("diagnostic input number exhausted"))?;
                q.input_check.push_back(DiagnosticInput {
                    number,
                    position,
                    capture,
                    delivered,
                });
            }
            w.signal();
        }
        receipt
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| failure("AppKit did not deliver diagnostic input within 5s"))?
            .ok_or_else(|| failure("AppKit could not create diagnostic input"))
    }

    /// Diagnostics only: backend host clock; callable from any thread.
    #[cfg(feature = "diagnostics")]
    pub fn diagnostic_now(&self) -> Duration {
        native_time()
    }

    /// Diagnostics only: process user plus system CPU time, from any thread.
    #[cfg(feature = "diagnostics")]
    pub fn diagnostic_cpu_time(&self) -> Result<Duration, Error> {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: getrusage writes a complete rusage to a correctly aligned
        // writable pointer on success; RUSAGE_SELF includes all process threads.
        let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
        if result != 0 {
            return Err(failure("getrusage(RUSAGE_SELF) failed"));
        }
        // SAFETY: successful getrusage initialized the entire struct above.
        let usage = unsafe { usage.assume_init() };
        let total = usage.ru_utime.tv_sec as f64
            + f64::from(usage.ru_utime.tv_usec) / 1e6
            + usage.ru_stime.tv_sec as f64
            + f64::from(usage.ru_stime.tv_usec) / 1e6;
        Duration::try_from_secs_f64(total).map_err(|_| failure("invalid process CPU time"))
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
    /// Uses update-supplied drawables for synchronized ordinary frames.
    pub fn with_metal_drawables(mut self) -> Self {
        self.metal_drawables = true;
        self
    }

    /// Buffers raw CAMetalDisplayLink times for a diagnostic reader.
    pub fn with_metal_timing(mut self) -> Self {
        self.metal_telemetry = true;
        self
    }

    /// Enables single-window drawable diagnostics and places the window on the
    /// primary display. Call before run, on any thread. Feature-gated, off by
    /// default even when compiled in.
    #[cfg(feature = "diagnostics")]
    pub fn with_diagnostics(mut self) -> Self {
        self.diagnostics = true;
        self
    }
    /// Gets an observation/control handle before entering the main-thread run.
    pub fn control(&self) -> MacOsControl {
        self.control.clone()
    }
}
struct Context {
    metal_drawables: bool,
    metal_telemetry: bool,
    #[cfg(feature = "diagnostics")]
    diagnostics: bool,
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
        #[cfg(feature = "diagnostics")]
        if self.diagnostics {
            // The first NSScreen is the primary display. mainScreen follows
            // keyboard focus and can put successive benchmark windows on a
            // different monitor with an incompatible refresh interval.
            let screen = objc2_app_kit::NSScreen::screens(self.mtm)
                .firstObject()
                .ok_or_else(|| failure("native diagnostics require a primary display"))?;
            let visible = screen.visibleFrame();
            let frame = window.frame();
            window.setFrameOrigin(NSPoint::new(
                visible.origin.x + (visible.size.width - frame.size.width) * 0.5,
                visible.origin.y + (visible.size.height - frame.size.height) * 0.5,
            ));
            eprintln!(
                "Native diagnostics: primary display {}, maximum {} Hz",
                screen.localizedName(),
                screen.maximumFramesPerSecond()
            );
        }
        let view = ContentView::alloc(self.mtm).set_ivars(ViewState::default());
        // SAFETY: Correct NSView initializer and finite frame for a fresh allocation.
        let view: Retained<ContentView> = unsafe { msg_send![super(view), initWithFrame: rect] };
        // SAFETY: The main-thread ContentView implements the exact mouse-entered
        // and mouse-exited selectors, is retained for the view's lifetime, and no
        // userInfo pointer is supplied. NSView retains the added tracking area.
        let tracking = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                rect,
                NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::ActiveAlways
                    | NSTrackingAreaOptions::InVisibleRect
                    | NSTrackingAreaOptions::EnabledDuringMouseDrag,
                Some(&view),
                None,
            )
        };
        view.addTrackingArea(&tracking);
        let feedback = Arc::new(presentation_probe::Feedback::default());
        #[cfg(feature = "diagnostics")]
        feedback
            .diagnostics
            .store(self.diagnostics, Ordering::Relaxed);
        let layer = presentation_probe::layer(feedback.clone());
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
        // Only interactive live-resize transactions temporarily enable this flag.
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
            feedback,
            metal: self
                .metal_drawables
                .then(|| MetalFrames::new(self.metal_telemetry)),
            id,
            handle,
            source: self.source.clone(),
            changed: Condvar::new(),
            queue: Mutex::new(Queue {
                events: VecDeque::new(),
                input_sequence: 0,
                platform_flushes: VecDeque::new(),
                alive: true,
                endpoint: true,
                wake: false,
                frame: false,
                #[cfg(feature = "diagnostics")]
                fullscreen: false,
                #[cfg(feature = "diagnostics")]
                input_check: VecDeque::new(),
                #[cfg(feature = "diagnostics")]
                input_receipts: std::collections::BTreeMap::new(),
                #[cfg(feature = "diagnostics")]
                next_input_number: -1,
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
        let delegate = Delegate::alloc(self.mtm).set_ivars(DelegateState {
            id,
            visible: Cell::new(false),
        });
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
        let initial_preferences = preferences::snapshot(&view);
        let native = Rc::new(Native {
            window,
            view,
            _delegate: delegate,
            layer,
            link,
            metal_link: RefCell::new(None),
            metal_idle: Cell::new(0),
            resizing: Cell::new(false),
            transitioning: Cell::new(false),
            shared: shared.clone(),
            timeout: attributes.resize_timeout,
            sequence: Cell::new(0),
            scale: Cell::new(1.0),
            preferences: Cell::new(initial_preferences),
            initial: Cell::new(true),
            interval: Cell::new(Duration::from_nanos(16_666_667)),
            target: Cell::new(Duration::ZERO),
            used_slot: Cell::new(Duration::ZERO),
            closed: Cell::new(false),
            input: input_dispatch::Dispatch::default(),
            panels: desktop_services::Panels::default(),
        });
        WINDOWS.with(|w| w.borrow_mut().push(native.clone()));
        #[cfg(not(feature = "diagnostics"))]
        native.window.center();
        #[cfg(feature = "diagnostics")]
        if !self.diagnostics {
            native.window.center();
        }
        native.window.makeKeyAndOrderFront(None);
        native.window.makeFirstResponder(Some(&native.view));
        Ok(CreatedWindow {
            main: Box::new(Main {
                shared: shared.clone(),
                _affinity: std::marker::PhantomData,
            }),
            endpoint: Box::new(Endpoint {
                shared,
                dispatched_frame: None,
                terminal: false,
                initial_preferences,
                completed_input: None,
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
            let _preference_observers = preferences::observe(mtm);
            let mut context = Context {
                metal_drawables: self.metal_drawables,
                metal_telemetry: self.metal_telemetry,
                #[cfg(feature = "diagnostics")]
                diagnostics: self.diagnostics,
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
                // Sample before the first resize even if AppKit has not yet
                // drained main-queue notifications posted during initialization.
                w.refresh_preferences();
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
    fn capabilities_report_observable_preferences() {
        let fields = capabilities().system_preferences;
        assert_eq!(
            fields,
            [
                SystemPreference::Appearance,
                SystemPreference::AccentColor,
                SystemPreference::IncreasedContrast,
                SystemPreference::ReducedMotion,
            ]
        );
    }
    #[test]
    fn slot_recalibration_does_not_deliver_twice() {
        let interval = Duration::from_millis(10);
        let used = Duration::from_millis(100);
        assert!(!unused_slot(used, used, interval));
        assert!(!unused_slot(
            used + Duration::from_micros(10),
            used,
            interval
        ));
        assert!(!unused_slot(
            used - Duration::from_micros(10),
            used,
            interval
        ));
        assert!(unused_slot(used + interval, used, interval));
        let feedback = presentation_probe::Feedback::default();
        assert_eq!(feedback.offset(), Duration::from_nanos(16_666_667));
    }
    #[test]
    fn pointer_buttons() {
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

// Production presentation feedback, per window. Metal reports completed frames;
// no polling or timers keep an idle application awake.
mod presentation_probe {
    use super::*;
    use block2::RcBlock;
    use objc2::runtime::ProtocolObject;
    use objc2::{AnyThread, runtime::AnyObject};
    use objc2_metal::MTLDrawable;
    use std::ptr::NonNull;
    use std::sync::atomic::AtomicBool;

    // Metal may invoke handlers on arbitrary threads. Every field accessed by
    // handlers is atomic or mutex-protected; no AppKit/window-thread state is used.
    pub(super) struct Feedback {
        pub completed: Mutex<Vec<(u64, f64, Duration)>>,
        #[cfg(feature = "diagnostics")]
        pub(super) display_updates: Mutex<Vec<MetalDisplayUpdate>>,
        pub acquired: AtomicU64,
        pub acquired_slot: AtomicU64,
        pub slot: AtomicU64,
        pub interval: AtomicU64,
        estimate: AtomicU64,
        pub diagnostics: AtomicBool,
        pub link_owned: AtomicBool,
    }
    impl Default for Feedback {
        fn default() -> Self {
            Self {
                completed: Mutex::new(Vec::new()),
                #[cfg(feature = "diagnostics")]
                display_updates: Mutex::new(Vec::new()),
                acquired: AtomicU64::new(0),
                acquired_slot: AtomicU64::new(0),
                slot: AtomicU64::new(0),
                interval: AtomicU64::new(16_666_667),
                estimate: AtomicU64::new(0),
                diagnostics: AtomicBool::new(false),
                link_owned: AtomicBool::new(false),
            }
        }
    }
    impl Feedback {
        fn acquired(&self, id: u64, slot: u64) {
            self.acquired.store(id, Ordering::Release);
            self.acquired_slot.store(slot, Ordering::Release);
        }
        pub fn reset(&self) {
            self.estimate.store(0, Ordering::Release);
        }
        pub fn offset(&self) -> Duration {
            let interval = self.interval.load(Ordering::Acquire);
            let estimate = self.estimate.load(Ordering::Acquire);
            Duration::from_nanos(if estimate == 0 {
                interval
            } else {
                estimate.min(interval * 4)
            })
        }
        fn presented(&self, id: u64, time: f64, slot: Duration, legacy: bool) {
            // Feedback calibrates the display clock. Surface acquisition supplies backpressure.
            if legacy && time.is_finite() && time > 0.0 {
                let offset = seconds(time).saturating_sub(slot);
                let bound = self.interval.load(Ordering::Acquire) * 4;
                let sample = (offset.as_nanos() as u64).clamp(1, bound);
                // Concurrent callbacks must not overwrite each other's EWMA updates.
                let _ = self
                    .estimate
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                        Some(if old == 0 {
                            sample
                        } else {
                            (old * 7 + sample) / 8
                        })
                    });
            }
            if self.diagnostics.load(Ordering::Relaxed)
                && let Ok(mut done) = self.completed.lock()
            {
                // Diagnostics must drain records; bound forgotten consumers.
                if done.len() >= 4096 {
                    done.remove(0);
                }
                // Zero explicitly reports not displayed and never changes the estimate.
                done.push((id, time, slot));
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn delayed_feedback_keeps_original_slot_and_reports_drops() -> Result<(), String> {
            let feedback = Arc::new(Feedback::default());
            feedback.diagnostics.store(true, Ordering::Relaxed);
            feedback.interval.store(10_000_000, Ordering::Release);
            // A later acquisition must not invalidate an earlier callback's slot.
            feedback.slot.store(2_000_000_000, Ordering::Release);
            feedback.acquired(1, 1_000_000_000);
            feedback.acquired(2, 2_000_000_000);
            let worker = Arc::clone(&feedback);
            std::thread::spawn(move || {
                worker.presented(1, 1.02, Duration::from_secs(1), true);
                worker.presented(2, 0.0, Duration::from_secs(2), true);
            })
            .join()
            .map_err(|_| "feedback worker panicked")?;
            assert_eq!(feedback.offset(), Duration::from_millis(20));
            feedback.acquired(3, 2_000_000_000);
            feedback.presented(3, 2.028, Duration::from_secs(2), true);
            assert_eq!(feedback.offset(), Duration::from_millis(21));
            let records = feedback.completed.lock().map_err(|e| e.to_string())?;
            assert_eq!(
                records.as_slice(),
                &[
                    (1, 1.02, Duration::from_secs(1)),
                    (2, 0.0, Duration::from_secs(2)),
                    (3, 2.028, Duration::from_secs(2)),
                ]
            );
            Ok(())
        }

        #[test]
        fn production_feedback_does_not_collect_diagnostics() -> Result<(), String> {
            let feedback = Feedback::default();
            feedback.acquired(1, 1_000_000_000);
            feedback.presented(1, 1.02, Duration::from_secs(1), true);
            assert_eq!(feedback.offset(), Duration::from_millis(20));
            assert!(
                feedback
                    .completed
                    .lock()
                    .map_err(|e| e.to_string())?
                    .is_empty()
            );
            Ok(())
        }
    }

    define_class!(
        // SAFETY: NSObject subclass with no state; window can be queried on any
        // thread and always returns nil.
        #[unsafe(super = NSObject)]
        struct HostingDelegate;
        // SAFETY: NSObjectProtocol adds no invariants.
        unsafe impl NSObjectProtocol for HostingDelegate {}
        impl HostingDelegate {
            #[unsafe(method(window))]
            fn window(&self) -> *mut AnyObject { std::ptr::null_mut() }
        }
    );
    define_class!(
        // SAFETY: CAMetalLayer supports subclassing and off-main nextDrawable.
        // All added state is synchronized.
        #[unsafe(super = CAMetalLayer)]
        #[ivars = Arc<Feedback>]
        struct ProbeLayer;
        impl ProbeLayer {
            #[unsafe(method_id(nextDrawable))]
            fn next_drawable(&self) -> Option<Retained<ProtocolObject<dyn MTLDrawable>>> {
                boundary(|| {
                    // SAFETY: Exact superclass object-returning method signature;
                    // CAMetalDrawable conforms to MTLDrawable.
                    let drawable: Option<Retained<ProtocolObject<dyn MTLDrawable>>> = unsafe { msg_send![super(self), nextDrawable] };
                    if let Some(drawable) = &drawable
                        && !self.ivars().link_owned.load(Ordering::Acquire) {
                        let slot = self.ivars().slot.load(Ordering::Acquire);
                        track(self.ivars(), drawable, Duration::from_nanos(slot), true);
                    }
                    drawable
                })
            }
        }
    );
    pub(super) fn track(
        feedback: &Arc<Feedback>,
        drawable: &ProtocolObject<dyn MTLDrawable>,
        slot: Duration,
        legacy: bool,
    ) {
        feedback.acquired(drawable.drawableID() as u64, slot.as_nanos() as u64);
        let feedback = Arc::clone(feedback);
        // Capture only shared feedback and the acquisition slot, never
        // the drawable: tracking cannot hold the drawable pool hostage.
        let handler = RcBlock::new(move |drawable: NonNull<ProtocolObject<dyn MTLDrawable>>| {
            boundary(|| {
                // SAFETY: Metal supplies a live drawable for the callback.
                let drawable = unsafe { drawable.as_ref() };
                feedback.presented(
                    drawable.drawableID() as u64,
                    drawable.presentedTime(),
                    slot,
                    legacy,
                );
            });
        });
        // SAFETY: Valid heap block with the required ABI. Metal copies
        // it for later invocation; its captures are thread-safe and
        // boundary prevents unwinding across the Objective-C boundary.
        unsafe {
            drawable.addPresentedHandler(RcBlock::as_ptr(&handler));
        }
    }

    pub(super) fn layer(feedback: Arc<Feedback>) -> Retained<CAMetalLayer> {
        let layer = ProbeLayer::alloc().set_ivars(feedback);
        // SAFETY: Standard superclass initializer for fresh CAMetalLayer allocation.
        let layer: Retained<ProbeLayer> = unsafe { msg_send![super(layer), init] };
        // SAFETY: NSObject initializer on a fresh stateless allocation.
        let delegate: Retained<HostingDelegate> =
            unsafe { msg_send![HostingDelegate::alloc(), init] };
        // SAFETY: CALayer delegate is weak. Intentionally retain for the same
        // process lifetime as the exported layer (ADR-0025). wgpu-hal 30.0.2
        // metal/surface.rs acquire_texture/hosting_window (gfx-rs/wgpu#8309)
        // stops at this delegate's nil window, bypassing false Occluded results
        // after AppKit fullscreen reparenting. Truly occluded windows can block
        // in nextDrawable until its timeout; wgpu maps nil to Timeout and our
        // renderer maps that to FrameOutcome::Skipped, retaining work for retry.
        unsafe {
            let _: () = msg_send![&*layer, setDelegate: &*delegate];
        }
        std::mem::forget(delegate);
        Retained::into_super(layer)
    }
}

#[cfg(test)]
mod scroll_tests {
    use super::*;

    #[test]
    fn maps_appkit_wheel_direct_and_momentum_phases() {
        use NSEventPhase as NativePhase;

        for (direct, momentum, expected) in [
            (NativePhase::None, NativePhase::None, None),
            (
                NativePhase::MayBegin,
                NativePhase::None,
                Some(ScrollPhase::Cancel),
            ),
            (
                NativePhase::Began,
                NativePhase::None,
                Some(ScrollPhase::Begin(ScrollMomentum::Native)),
            ),
            (
                NativePhase::Changed,
                NativePhase::None,
                Some(ScrollPhase::Update),
            ),
            (
                NativePhase::Stationary,
                NativePhase::None,
                Some(ScrollPhase::Update),
            ),
            (
                NativePhase::Ended,
                NativePhase::None,
                Some(ScrollPhase::End),
            ),
            (
                NativePhase::Cancelled,
                NativePhase::None,
                Some(ScrollPhase::Cancel),
            ),
            (
                NativePhase::None,
                NativePhase::Began,
                Some(ScrollPhase::MomentumBegin),
            ),
            (
                NativePhase::None,
                NativePhase::Changed,
                Some(ScrollPhase::MomentumUpdate),
            ),
            (
                NativePhase::None,
                NativePhase::Ended,
                Some(ScrollPhase::MomentumEnd),
            ),
        ] {
            assert_eq!(scroll_phase(direct, momentum), expected);
        }
    }
}
