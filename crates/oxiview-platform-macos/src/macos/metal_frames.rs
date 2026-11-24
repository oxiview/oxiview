use super::*;

use objc2_metal::MTLDrawable;
use objc2_quartz_core::{
    CAMetalDisplayLink, CAMetalDisplayLinkDelegate, CAMetalDisplayLinkUpdate, CAMetalDrawable,
};
use std::any::Any;

/// One pass through the window thread for a drawable supplied by Core Animation.
pub struct MetalDrawable {
    offered: Offered,
    frames: MetalFrames,
    epoch: u64,
}

/// Raw host-clock timing for a display-link drawable, in seconds.
#[derive(Clone, Copy, Debug)]
pub struct MetalTiming {
    /// OS drawable identifier.
    pub drawable_id: u64,
    /// Host clock sampled at the delegate callback.
    pub callback_time: f64,
    /// Apple CPU presentation deadline.
    pub target_timestamp: f64,
    /// Apple predicted display time.
    pub target_presentation_timestamp: f64,
    /// Requested driver time for rendering, in frames.
    pub preferred_frame_latency: f32,
    /// Whether the opportunity was offered to the window thread.
    pub offered: bool,
    /// Host clock sampled when the window thread takes the drawable.
    pub taken_time: Option<f64>,
    /// Host clock sampled after application paint and CPU scene encoding.
    pub scene_encoded_time: Option<f64>,
    /// Host clock sampled after submitting the wgpu scene.
    pub scene_submitted_time: Option<f64>,
    /// Host clock sampled immediately before presentation and commit.
    pub commit_time: Option<f64>,
    /// Host clock sampled in the Metal scheduled handler.
    pub scheduled_time: Option<f64>,
    /// Metal's GPU start time for the final output pass.
    pub gpu_start_time: Option<f64>,
    /// Metal's GPU end time for the final output pass.
    pub gpu_end_time: Option<f64>,
    /// Native MTLCommandBufferStatus value when the output pass completed.
    pub command_status: Option<usize>,
}

/// Write access for the Metal adapter's schedule and completion handlers.
#[derive(Clone)]
pub struct MetalTimingSample(Arc<Mutex<MetalTiming>>);

impl MetalTimingSample {
    /// Samples CPU scene encoding completion.
    pub fn scene_encoded(&self) {
        let now = host_time();
        if let Ok(mut timing) = self.0.lock() {
            timing.scene_encoded_time = Some(now);
        }
    }

    /// Samples the host clock after the wgpu scene submit returns.
    pub fn scene_submitted(&self) {
        let now = host_time();
        if let Ok(mut timing) = self.0.lock() {
            timing.scene_submitted_time = Some(now);
        }
    }

    /// Samples the host clock from the scheduled handler.
    pub fn scheduled(&self) {
        let now = host_time();
        if let Ok(mut timing) = self.0.lock() {
            timing.scheduled_time = Some(now);
        }
    }

    /// Records the raw command-buffer timestamps and status from Metal.
    pub fn completed(&self, start: f64, end: f64, status: usize) {
        if let Ok(mut timing) = self.0.lock() {
            timing.gpu_start_time = Some(start);
            timing.gpu_end_time = Some(end);
            timing.command_status = Some(status);
        }
    }
}

impl MetalDrawable {
    /// Borrows the retained display-link resource for the compatible renderer.
    /// The adapter downcasts it using the same pinned native bindings.
    pub fn resource(&self) -> &dyn Any {
        &self.offered.raw
    }

    /// Returns whether this frame belongs to the current layer configuration
    /// and is still within Apple's CPU deadline.
    pub fn can_submit(&self) -> bool {
        self.frames.0.lock().is_ok_and(|state| {
            state.active && state.epoch == self.epoch && host_time() < self.offered.deadline
        })
    }

    /// Returns the optional Metal command-buffer observer for this drawable.
    pub fn timing_sample(&self) -> Option<MetalTimingSample> {
        self.offered.sample.clone()
    }

    /// Consumes the drawable, running the final render submission only if
    /// its deadline and layer generation are still current.
    pub fn submit<T>(self, submit: impl FnOnce(&dyn Any) -> T) -> Option<T> {
        autoreleasepool(|_| {
            let state = self.frames.0.lock().ok()?;
            let now = host_time();
            if !state.active || state.epoch != self.epoch || now >= self.offered.deadline {
                return None;
            }
            if self.offered.feedback.diagnostics.load(Ordering::Relaxed) {
                presentation_probe::track(
                    &self.offered.feedback,
                    ProtocolObject::from_ref(&*self.offered.raw),
                    self.offered.slot,
                    false,
                );
            }
            if let Some(sample) = &self.offered.sample
                && let Ok(mut timing) = sample.0.lock()
            {
                timing.commit_time = Some(host_time());
            }
            let result = submit(self.resource());
            // Serialize presentation against layer mutations and link detachment.
            drop(state);
            Some(result)
        })
    }
}

/// The source selected for a particular ordinary frame event.
pub enum MetalFrame {
    /// The delegate supplied this exact frame's drawable.
    Drawable(MetalDrawable),
    /// The Metal opportunity expired, was canceled, or was already consumed.
    Skipped,
    /// The regular clock drives this frame, including resize retries.
    Fallback,
}

#[derive(Default)]
struct Mailbox {
    active: bool,
    epoch: u64,
    offered_at: Option<Duration>,
    awaiting_take: bool,
    pending: Option<Offered>,
    telemetry: Option<Vec<MetalTimingSample>>,
}

impl Mailbox {
    fn has_unclaimed(&self) -> bool {
        !self.awaiting_take && self.pending.is_some()
    }

    fn can_stage(&mut self, slot: Duration, deadline: f64, now: f64) -> bool {
        if self.has_unclaimed()
            && self
                .pending
                .as_ref()
                .is_some_and(|pending| now >= pending.deadline)
        {
            self.pending = None;
        }
        self.active
            // A detach releases the drawable immediately. Its queued event
            // still needs to resolve as Skipped before another can publish.
            && !self.awaiting_take
            && !slot.is_zero()
            && deadline.is_finite()
            && now < deadline
            && self.offered_at.is_none_or(|last| last < slot)
            && self
                .pending
                .as_ref()
                .is_none_or(|pending| self.offered_at != Some(pending.slot) && pending.slot < slot)
    }
}

struct Offered {
    raw: Retained<ProtocolObject<dyn CAMetalDrawable>>,
    deadline: f64,
    slot: Duration,
    interval: Duration,
    sample: Option<MetalTimingSample>,
    feedback: Arc<presentation_probe::Feedback>,
}

// SAFETY: The mailbox transfers the only application-owned drawable reference
// from the display-link callback to the window thread. Metal permits encoding
// and presentation on that thread. No AppKit object is transferred, and this
// wrapper does not confer Sync on the drawable.
unsafe impl Send for Offered {}

/// Per-window, single-slot handoff from the display-link delegate to the renderer.
#[derive(Clone, Default)]
pub struct MetalFrames(Arc<Mutex<Mailbox>>);

impl MetalFrames {
    pub(super) fn new(telemetry: bool) -> Self {
        Self(Arc::new(Mutex::new(Mailbox {
            telemetry: telemetry.then(|| Vec::with_capacity(8192)),
            ..Mailbox::default()
        })))
    }

    /// Takes the drawable belonging to this exact frame event at most once.
    pub fn take(&self, timestamp: Duration) -> MetalFrame {
        let Ok(mut state) = self.0.lock() else {
            return MetalFrame::Skipped;
        };
        if state.offered_at == Some(timestamp) {
            if !state.awaiting_take {
                return MetalFrame::Skipped;
            }
            state.awaiting_take = false;
            if state.active
                && let Some(offered) = state.pending.take()
            {
                let now = host_time();
                if now >= offered.deadline {
                    return MetalFrame::Skipped;
                }
                if let Some(sample) = &offered.sample
                    && let Ok(mut timing) = sample.0.lock()
                {
                    timing.taken_time = Some(now);
                }
                return MetalFrame::Drawable(MetalDrawable {
                    offered,
                    frames: self.clone(),
                    epoch: state.epoch,
                });
            }
            return MetalFrame::Skipped;
        }
        if state.active {
            MetalFrame::Skipped
        } else {
            MetalFrame::Fallback
        }
    }

    pub(super) fn discard(&self, timestamp: Duration) {
        if let Ok(mut state) = self.0.lock()
            && state.offered_at == Some(timestamp)
            && state.awaiting_take
        {
            state.awaiting_take = false;
            state.pending = None;
        }
    }

    pub(super) fn attach(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.active = true;
        }
    }

    pub(super) fn detach(&self) {
        if let Ok(mut state) = self.0.lock()
            && state.active
        {
            state.active = false;
            state.epoch = state.epoch.wrapping_add(1);
            state.pending = None;
        }
    }

    pub(super) fn drain(&self) -> Vec<MetalTiming> {
        self.0.lock().map_or_else(
            |_| Vec::new(),
            |mut state| {
                state.telemetry.as_mut().map_or_else(Vec::new, |samples| {
                    samples
                        .drain(..)
                        .filter_map(|sample| sample.0.lock().ok().map(|timing| *timing))
                        .collect()
                })
            },
        )
    }

    pub(super) fn discard_unclaimed(&self) {
        if let Ok(mut state) = self.0.lock()
            && state.has_unclaimed()
        {
            state.pending = None;
        }
    }

    /// Dispatches a staged drawable only in response to demand. The caller
    /// holds the window queue lock across claim and publication of the event.
    pub(super) fn claim(&self) -> Option<(Duration, Duration)> {
        let mut state = self.0.lock().ok()?;
        if !state.active || !state.has_unclaimed() {
            return None;
        }
        let offered = state.pending.as_ref()?;
        if host_time() >= offered.deadline {
            state.pending = None;
            return None;
        }
        let (slot, interval) = (offered.slot, offered.interval);
        if let Some(sample) = &offered.sample
            && let Ok(mut timing) = sample.0.lock()
        {
            timing.offered = true;
        }
        state.offered_at = Some(slot);
        state.awaiting_take = true;
        Some((slot, interval))
    }

    pub(super) fn stage(
        &self,
        update: &CAMetalDisplayLinkUpdate,
        interval: Duration,
        callback_time: f64,
        preferred_frame_latency: f32,
        feedback: &Arc<presentation_probe::Feedback>,
    ) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        let target_presentation_timestamp = update.targetPresentationTimestamp();
        let slot = seconds(target_presentation_timestamp);
        let deadline = update.targetTimestamp();
        let staged = state.can_stage(slot, deadline, host_time());
        if !staged && state.telemetry.is_none() {
            return;
        }
        let raw = update.drawable();
        let sample = state.telemetry.as_mut().and_then(|samples| {
            if samples.len() >= 8192 {
                return None;
            }
            let timing = MetalTiming {
                drawable_id: raw.drawableID() as u64,
                callback_time,
                target_timestamp: deadline,
                target_presentation_timestamp,
                preferred_frame_latency,
                offered: false,
                taken_time: None,
                scene_encoded_time: None,
                scene_submitted_time: None,
                commit_time: None,
                scheduled_time: None,
                gpu_start_time: None,
                gpu_end_time: None,
                command_status: None,
            };
            let sample = MetalTimingSample(Arc::new(Mutex::new(timing)));
            samples.push(sample.clone());
            Some(sample)
        });
        if !staged {
            return;
        }
        state.pending = Some(Offered {
            raw,
            deadline,
            slot,
            interval,
            sample,
            feedback: Arc::clone(feedback),
        });
    }
}

pub(super) fn host_time() -> f64 {
    // SAFETY: CACurrentMediaTime is a thread-safe host clock with no arguments.
    unsafe { CACurrentMediaTime() }
}

define_class!(
    // SAFETY: NSObject supports subclassing and the ivar is a value.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = WindowId]
    struct MetalDelegate;

    // SAFETY: NSObjectProtocol adds no requirements.
    unsafe impl NSObjectProtocol for MetalDelegate {}
    // SAFETY: The exact callback signature is registered only on the main loop.
    unsafe impl CAMetalDisplayLinkDelegate for MetalDelegate {
        #[unsafe(method(metalDisplayLink:needsUpdate:))]
        fn metal_update(&self, link: &CAMetalDisplayLink, update: &CAMetalDisplayLinkUpdate) {
            boundary(|| {
                if let Some(window) = find(*self.ivars()) {
                    window.metal_update(link, update);
                }
            });
        }
    }
);

pub(super) struct Link {
    pub link: Retained<CAMetalDisplayLink>,
    _delegate: Retained<MetalDelegate>,
    #[cfg(feature = "diagnostics")]
    pub(super) observation: Option<display_observation::Source>,
}

impl Link {
    pub(super) fn new(window: &Native) -> Self {
        let delegate = MetalDelegate::alloc(window.view.mtm()).set_ivars(window.shared.id);
        // SAFETY: Standard initializer for an allocated NSObject subclass.
        let delegate: Retained<MetalDelegate> = unsafe { msg_send![super(delegate), init] };
        let link =
            CAMetalDisplayLink::initWithMetalLayer(CAMetalDisplayLink::alloc(), &window.layer);
        link.setPaused(true);
        link.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // SAFETY: Constant run-loop mode and registration on the main thread.
        unsafe {
            link.addToRunLoop_forMode(
                &NSRunLoop::mainRunLoop(),
                objc2_foundation::NSRunLoopCommonModes,
            );
        }
        Self {
            link,
            _delegate: delegate,
            #[cfg(feature = "diagnostics")]
            observation: window
                .shared
                .feedback
                .diagnostics
                .load(Ordering::Relaxed)
                .then(|| display_observation::Source::new(window))
                .flatten(),
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.link.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unclaimed_drawable_waits_for_demand_and_never_overwrites_dispatched_frame()
    -> Result<(), Box<dyn std::error::Error>> {
        autoreleasepool(|_| {
            let device =
                objc2_metal::MTLCreateSystemDefaultDevice().ok_or("missing Metal device")?;
            let layer = CAMetalLayer::new();
            layer.setDevice(Some(&device));
            layer.setDrawableSize(NSSize::new(16.0, 16.0));
            let raw = layer.nextDrawable().ok_or("detached layer drawable")?;
            let id = raw.drawableID();
            let frames = MetalFrames::default();
            let feedback = Arc::new(presentation_probe::Feedback::default());
            let first = Duration::from_secs(1);
            let later = Duration::from_secs(2);
            let interval = Duration::from_nanos(8_333_333);
            let stage = |slot, deadline, now| {
                let mut state = frames.0.lock().map_err(|e| e.to_string())?;
                let staged = state.can_stage(slot, deadline, now);
                if staged {
                    state.pending = Some(Offered {
                        raw: raw.clone(),
                        deadline,
                        slot,
                        interval,
                        sample: None,
                        feedback: feedback.clone(),
                    });
                }
                Ok::<_, String>(staged)
            };
            let now = host_time();
            let live = now + 60.0;
            assert!(!stage(first, live, now)?);
            frames.attach();
            assert!(!stage(first, now, now)?);
            assert!(!stage(first, f64::INFINITY, now)?);
            assert!(!stage(Duration::ZERO, live, now)?);

            assert!(stage(first, live, now)?);
            assert!(matches!(frames.take(first), MetalFrame::Skipped));
            assert!(!stage(first, live, now)?);
            assert!(stage(later, live, now)?);
            assert_eq!(frames.claim(), Some((later, interval)));
            assert_eq!(frames.claim(), None);
            assert!(!stage(later + interval, live, now)?);
            frames.discard_unclaimed();
            assert!(matches!(frames.take(first), MetalFrame::Skipped));
            let MetalFrame::Drawable(mut drawable) = frames.take(later) else {
                return Err("late demand lost its drawable".into());
            };
            assert_eq!(drawable.offered.raw.drawableID(), id);
            assert!(matches!(frames.take(later), MetalFrame::Skipped));
            assert!(drawable.can_submit());
            drawable.offered.deadline = now;
            assert!(!drawable.can_submit());
            assert_eq!(drawable.submit(|_| id), None);

            let next = later + interval;
            assert!(stage(next, now, now - 1.0)?);
            assert_eq!(frames.claim(), None);
            assert!(stage(next, live, now)?);
            frames.discard_unclaimed();
            assert_eq!(frames.claim(), None);
            assert!(stage(next, live, now)?);
            frames.detach();
            frames.attach();
            assert_eq!(frames.claim(), None);
            assert!(matches!(frames.take(next), MetalFrame::Skipped));

            assert!(stage(next, live, now)?);
            assert_eq!(frames.claim(), Some((next, interval)));
            frames.detach();
            frames.attach();
            assert!(!stage(next + interval, live, now)?);
            frames.detach();
            assert!(matches!(frames.take(next), MetalFrame::Skipped));
            frames.attach();
            assert!(stage(next + interval, live, now)?);
            // Rechecking the retired event cannot take its successor.
            assert!(matches!(frames.take(next), MetalFrame::Skipped));
            assert_eq!(frames.claim(), Some((next + interval, interval)));
            Ok(())
        })
    }

    #[test]
    fn actual_drawables_keep_identity_and_only_current_leases_submit()
    -> Result<(), Box<dyn std::error::Error>> {
        autoreleasepool(|_| {
            let device =
                objc2_metal::MTLCreateSystemDefaultDevice().ok_or("missing Metal device")?;
            let layer = CAMetalLayer::new();
            layer.setDevice(Some(&device));
            layer.setDrawableSize(NSSize::new(16.0, 16.0));
            let first = layer
                .nextDrawable()
                .ok_or("first detached layer drawable")?;
            let second = layer
                .nextDrawable()
                .ok_or("second detached layer drawable")?;
            let first_id = first.drawableID();
            let second_id = second.drawableID();
            assert_ne!(first_id, second_id);

            let frames = MetalFrames::new(true);
            let feedback = Arc::new(presentation_probe::Feedback::default());
            feedback.diagnostics.store(true, Ordering::Relaxed);
            let put = |raw, at, deadline| {
                let mut state = frames.0.lock().map_err(|e| e.to_string())?;
                state.offered_at = Some(at);
                state.awaiting_take = true;
                state.pending = Some(Offered {
                    raw,
                    deadline,
                    slot: at,
                    interval: Duration::from_micros(8333),
                    sample: None,
                    feedback: Arc::clone(&feedback),
                });
                Ok::<_, String>(())
            };
            let t1 = Duration::from_secs(1);
            let t2 = Duration::from_secs(2);
            let t3 = Duration::from_secs(3);
            let t4 = Duration::from_secs(4);
            let future = host_time() + 60.0;

            assert!(matches!(frames.take(t1), MetalFrame::Fallback));
            frames.attach();
            put(first.clone(), t1, future)?;
            assert!(matches!(frames.take(t2), MetalFrame::Skipped));
            let MetalFrame::Drawable(drawable) = frames.take(t1) else {
                return Err("first opportunity lost its drawable".into());
            };
            assert!(drawable.can_submit());
            assert!(matches!(frames.take(t1), MetalFrame::Skipped));
            let id = |resource: &dyn Any| {
                resource
                    .downcast_ref::<Retained<ProtocolObject<dyn CAMetalDrawable>>>()
                    .map(|drawable| drawable.drawableID())
            };
            assert_eq!(id(drawable.resource()), Some(first_id));
            assert_eq!(drawable.submit(id), Some(Some(first_id)));
            assert_eq!(feedback.acquired.load(Ordering::Acquire), first_id as u64);
            assert_eq!(
                feedback.acquired_slot.load(Ordering::Acquire),
                t1.as_nanos() as u64
            );

            put(first, t2, 0.0)?;
            assert!(matches!(frames.take(t2), MetalFrame::Skipped));

            put(second.clone(), t3, future)?;
            let MetalFrame::Drawable(stale) = frames.take(t3) else {
                return Err("second opportunity lost its drawable".into());
            };
            assert_eq!(id(stale.resource()), Some(second_id));
            put(second, t4, future)?;
            frames.discard(t4);
            assert!(matches!(frames.take(t4), MetalFrame::Skipped));
            frames.detach();
            frames.attach();
            assert!(!stale.can_submit());
            assert_eq!(stale.submit(id), None);
            assert_eq!(feedback.acquired.load(Ordering::Acquire), first_id as u64);
            Ok(())
        })
    }
}
