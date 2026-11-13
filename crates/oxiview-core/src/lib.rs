//! Unstable UI-core scheduling and window-thread orchestration (M1 S3).
//! Scenes use physical pixel coordinates. No renderer or clock is selected here.

use oxiview_platform::{PhysicalSize, WindowEndpoint, WindowEvent, WindowWaker};
use oxiview_scene::{NodeId, Scene};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    time::Duration,
};

/// Failures terminate this window loop; affected state is never resumed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Platform command or event reception failed.
    #[error("platform operation failed: {0}")]
    Platform(#[from] oxiview_platform::Error),
    /// Application event handling or painting failed.
    #[error("application failed: {0}")]
    Application(String),
    /// Presenter configuration or presentation failed.
    #[error("presentation failed: {0}")]
    Present(String),
    /// Unexpected unwind at the window-thread boundary.
    #[error("window thread panicked")]
    Panicked,
}

/// Why a frame was produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    /// Requested display opportunity.
    Vsync,
    /// Immediate resize transaction.
    Resize,
}
/// Timing supplied by the backend, never sampled from a clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTiming {
    /// Expected visibility time on the backend clock.
    pub predicted_presentation_timestamp: Duration,
    /// Display interval for this frame.
    pub refresh_interval: Duration,
    /// Vsync or immediate resize.
    pub kind: FrameKind,
}
/// Input-to-predicted-presentation data for one successfully presented frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LatencyRecord {
    /// Earliest input capture consumed since the preceding produced frame.
    pub input_capture_timestamp: Duration,
    /// Backend timing, including the frame kind.
    pub timing: FrameTiming,
}
impl LatencyRecord {
    /// Returns None for inconsistent future-dated input, rather than underflowing.
    pub fn latency(&self) -> Option<Duration> {
        self.timing
            .predicted_presentation_timestamp
            .checked_sub(self.input_capture_timestamp)
    }
}
/// Commands to the endpoint. Taking these outputs coalesces frame arming.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Schedule {
    /// Arm one display opportunity.
    pub request_frame: bool,
    /// Absolute backend-clock deadline, or an indefinite event wait.
    pub deadline: Option<Duration>,
}
/// Pure on-demand state machine. No I/O or clock reads.
#[derive(Debug, Default)]
pub struct Scheduler {
    dirty: bool,
    armed: bool,
    suspended: bool,
    timed: Option<Duration>,
    input: Option<Duration>,
}
impl Scheduler {
    /// Marks application state dirty.
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }
    /// Requests a future frame; multiple requests coalesce to the earliest instant.
    pub fn request_at(&mut self, instant: Duration) {
        self.timed = Some(self.timed.map_or(instant, |old| old.min(instant)));
    }
    /// System repaint request.
    pub fn redraw(&mut self) {
        self.invalidate();
    }
    /// Attributes input to the pending frame. Call after the application handled
    /// the input; input that left the scheduler clean is not attributed, so it
    /// cannot inflate the latency of a later, unrelated frame.
    pub fn input(&mut self, timestamp: Duration) {
        if self.dirty {
            self.input = Some(self.input.map_or(timestamp, |old| old.min(timestamp)));
        }
    }
    /// A deadline wait expired. The caller supplies the expired deadline.
    pub fn deadline_reached(&mut self, instant: Duration) {
        if self.timed.is_some_and(|deadline| deadline <= instant) {
            self.timed = None;
            self.invalidate();
        }
    }
    /// Updates drawable availability. A nonzero resize requires immediate painting.
    pub fn resize(&mut self, size: PhysicalSize) -> bool {
        self.suspended = size.width == 0 || size.height == 0;
        self.dirty = true;
        !self.suspended
    }
    /// Consumes a display opportunity; stale opportunities do not paint twice.
    pub fn frame(&mut self) -> bool {
        self.armed = false;
        self.dirty && !self.suspended
    }
    /// Commits a produced frame and returns its input record, if any.
    /// A resize does not disarm a previously requested backend opportunity.
    pub fn produced(&mut self, timing: FrameTiming) -> Option<LatencyRecord> {
        self.dirty = false;
        self.input
            .take()
            .map(|input_capture_timestamp| LatencyRecord {
                input_capture_timestamp,
                timing,
            })
    }
    /// Takes commands once. Suspended windows retain work until a nonzero resize.
    pub fn schedule(&mut self) -> Schedule {
        let request_frame = self.dirty && !self.armed && !self.suspended;
        self.armed |= request_frame;
        Schedule {
            request_frame,
            deadline: if self.suspended { None } else { self.timed },
        }
    }
}
/// Scene and its selected root, in physical pixel coordinates.
pub struct PaintedScene {
    /// Retained scene instructions.
    pub scene: Scene,
    /// Subtree to present.
    pub root: NodeId,
}
/// Application code runs only on the window thread and need not be Send.
pub trait Application {
    /// Handles events and requests invalidation or future frames through the scheduler.
    /// The waker can be shared with cross-thread work producers.
    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        waker: &WindowWaker,
    ) -> Result<(), Error>;
    /// Paints a complete scene at the supplied backend timing, size, and scale.
    fn paint(
        &mut self,
        timing: FrameTiming,
        size: PhysicalSize,
        scale_factor: f64,
    ) -> Result<PaintedScene, Error>;
    /// Whether user close intent should close this window. Defaults to true.
    fn should_close(&mut self) -> bool {
        true
    }
    /// Receives per-frame latency data after successful presentation.
    fn presented(&mut self, _record: Option<LatencyRecord>) {}
}
/// Renderer-independent presentation boundary, used only on the window thread.
pub trait Presenter {
    /// Reconfigures or suspends for zero size before painting a resize.
    fn configure(&mut self, size: PhysicalSize, scale_factor: f64) -> Result<(), Error>;
    /// Presents the complete selected scene and maps failures into core errors.
    fn present(&mut self, scene: &PaintedScene) -> Result<(), Error>;
}

/// Runs a window thread to destruction, releasing the presenter before the endpoint.
/// The factory constructs thread-local application/presenter state inside the panic
/// boundary. Unexpected unwinds terminate the loop; no captured state is reused.
/// As with all unwind boundaries, aborts and double panics cannot be recovered.
pub fn run_window<A: Application, P: Presenter>(
    endpoint: Box<dyn WindowEndpoint>,
    initialize: impl FnOnce() -> Result<(A, P), Error>,
) -> Result<(), Error> {
    // This boundary takes exclusive ownership of the endpoint and factory. The
    // assertion permits trait objects without UnwindSafe; it does not promise
    // recovery of their invariants. Everything is dropped inside the boundary,
    // presenter before endpoint, and no state is accessed after an unwind.
    let result = catch_unwind(AssertUnwindSafe(move || {
        let mut endpoint = endpoint;
        let (mut application, mut presenter) = initialize()?;
        window_loop(&mut *endpoint, &mut application, &mut presenter)
    }));
    match result {
        Ok(result) => {
            if let Err(error) = &result {
                tracing::error!(%error, "window loop terminated");
            }
            result
        }
        Err(_) => {
            tracing::error!("unexpected panic terminated window loop");
            eprintln!("oxiview: unexpected panic terminated window loop");
            Err(Error::Panicked)
        }
    }
}
fn window_loop(
    endpoint: &mut dyn WindowEndpoint,
    application: &mut impl Application,
    presenter: &mut impl Presenter,
) -> Result<(), Error> {
    let mut scheduler = Scheduler::default();
    let mut size = PhysicalSize {
        width: 0,
        height: 0,
    };
    let mut scale = 1.0;
    scheduler.resize(size);
    let waker = endpoint.waker();
    loop {
        let output = scheduler.schedule();
        if output.request_frame {
            endpoint.request_frame()?;
        }
        let event = if let Some(deadline) = output.deadline {
            match endpoint.wait_event_until(deadline)? {
                Some(event) => event,
                None => {
                    scheduler.deadline_reached(deadline);
                    continue;
                }
            }
        } else {
            endpoint.wait_event()?
        };
        if matches!(event, WindowEvent::Destroyed) {
            application.event(&event, &mut scheduler, &waker)?;
            return Ok(());
        }
        match &event {
            WindowEvent::RedrawRequested => scheduler.redraw(),
            WindowEvent::ScaleFactorChanged { scale_factor } => scale = *scale_factor,
            _ => {}
        }
        application.event(&event, &mut scheduler, &waker)?;
        if let WindowEvent::PointerMoved { timestamp, .. }
        | WindowEvent::PointerButton { timestamp, .. }
        | WindowEvent::Key { timestamp, .. } = &event
        {
            scheduler.input(*timestamp);
        }
        match event {
            WindowEvent::Resized {
                physical_size,
                scale_factor,
                token,
                predicted_presentation_timestamp,
                refresh_interval,
            } => {
                let _span = tracing::info_span!("resize", ?physical_size, ?token).entered();
                size = physical_size;
                scale = scale_factor;
                presenter.configure(size, scale)?;
                if scheduler.resize(size) {
                    produce(
                        application,
                        presenter,
                        &mut scheduler,
                        FrameTiming {
                            predicted_presentation_timestamp,
                            refresh_interval,
                            kind: FrameKind::Resize,
                        },
                        size,
                        scale,
                    )?;
                }
                match endpoint.acknowledge_resize(token) {
                    // Expired/superseded sizes remain valid, but their token cannot
                    // be acknowledged. Continue to the newest queued transaction.
                    Err(oxiview_platform::Error::InvalidResizeToken) => {
                        tracing::warn!(?token, "resize token expired before acknowledgement")
                    }
                    result => result?,
                }
            }
            WindowEvent::Frame {
                predicted_presentation_timestamp,
                refresh_interval,
            } => {
                if scheduler.frame() {
                    produce(
                        application,
                        presenter,
                        &mut scheduler,
                        FrameTiming {
                            predicted_presentation_timestamp,
                            refresh_interval,
                            kind: FrameKind::Vsync,
                        },
                        size,
                        scale,
                    )?;
                }
            }
            WindowEvent::CloseRequested if application.should_close() => {
                endpoint.request_close()?
            }
            _ => {}
        }
    }
}
fn produce(
    application: &mut impl Application,
    presenter: &mut impl Presenter,
    scheduler: &mut Scheduler,
    timing: FrameTiming,
    size: PhysicalSize,
    scale: f64,
) -> Result<(), Error> {
    let _frame = tracing::info_span!("frame", ?timing).entered();
    let scene = application.paint(timing, size, scale)?;
    {
        let _present = tracing::info_span!("present").entered();
        presenter.present(&scene)?;
    }
    application.presented(scheduler.produced(timing));
    Ok(())
}

#[cfg(test)]
mod tests;
