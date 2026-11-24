//! Native scroll capture around the ordinary viewer. Screen feedback comes from Metal.

use crate::{
    app::{PaintedListPosition, ViewerApp},
    cadence::{Display, Frame, Report, STRIDE, Window},
};
use oxiview_core::{
    Application, Error, FrameKind, FrameSubmission, FrameTiming, PaintedScene, Scheduler,
};
use oxiview_platform::{
    PhysicalSize, PlatformCommand, SystemPreferences, WindowEvent, WindowWaker,
};
use oxiview_platform_macos::{MacOsControl, MacOsPlatform};
use std::{sync::mpsc, thread, time::Duration};

fn nanos(t: Duration) -> u64 {
    u64::try_from(t.as_nanos()).unwrap_or(u64::MAX)
}
struct Probe {
    viewer: ViewerApp,
    control: MacOsControl,
    send: Option<mpsc::Sender<Report>>,
    waker: Option<WindowWaker>,
    frames: Vec<Frame>,
    sample: Option<Window>,
    paint_sequence: u64,
    painted_row: Option<PaintedListPosition>,
    size: PhysicalSize,
    scale: f64,
    fail: Option<Error>,
}
impl Probe {
    fn collect(&mut self) -> Result<(), Error> {
        for (id, time) in self.control.diagnostic_presented_drawables()? {
            let at = nanos(
                Duration::try_from_secs_f64(time).map_err(|e| Error::Application(e.to_string()))?,
            );
            if let Some(f) = self
                .frames
                .iter_mut()
                .rev()
                .find(|f| f.drawable == id && f.presented_ns.is_none())
            {
                f.presented_ns = Some(at);
                if self.sample.is_none() && at > 0 {
                    self.sample = Window::new(at, f.refresh_ns);
                }
            }
        }
        Ok(())
    }
}
impl Application for Probe {
    fn initialize_preferences(&mut self, p: SystemPreferences) -> Result<(), Error> {
        self.viewer.initialize_preferences(p)
    }
    fn event(&mut self, e: &WindowEvent, s: &mut Scheduler, w: &WindowWaker) -> Result<(), Error> {
        if let Some(err) = self.fail.take() {
            return Err(err);
        }
        self.waker.get_or_insert_with(|| w.clone());
        let interruption = match e {
            WindowEvent::Wheel { .. } => Some("wheel input"),
            WindowEvent::Scroll { .. } => Some("trackpad input"),
            WindowEvent::PointerButton { .. } => Some("pointer button"),
            WindowEvent::Key { .. } => Some("keyboard input"),
            WindowEvent::TextInput(_) => Some("text input"),
            WindowEvent::VisibilityChanged { visible: false, .. } => Some("window hidden"),
            _ => None,
        };
        if let (Some(reason), Some(sample)) = (interruption, self.sample)
            && self.send.is_some()
            && (sample.start..sample.end).contains(&nanos(self.control.diagnostic_now()))
        {
            return Err(Error::Application(format!(
                "Fixed scroll interrupted by {reason} during measurement"
            )));
        }
        self.viewer.event(e, s, w)?;
        if matches!(e, WindowEvent::Wake) && self.send.is_some() {
            s.invalidate();
        }
        Ok(())
    }
    fn flush_platform(&mut self, c: &mut Vec<PlatformCommand>) -> Result<(), Error> {
        self.viewer.flush_platform(c)
    }
    fn paint(
        &mut self,
        t: FrameTiming,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, Error> {
        self.collect()?;
        if self.sample.is_some() && (self.size != size || self.scale != scale) {
            return Err(Error::Application(
                "window geometry changed during scroll capture".into(),
            ));
        }
        self.size = size;
        self.scale = scale;
        if t.kind == FrameKind::Vsync {
            self.viewer.scroll_sample(STRIDE)?;
        }
        let scene = self.viewer.paint(t, size, scale)?;
        self.painted_row = if t.kind == FrameKind::Vsync {
            self.paint_sequence = self.paint_sequence.saturating_add(1);
            Some(self.viewer.painted_list_position(&scene)?)
        } else {
            None
        };
        Ok(scene)
    }
    fn frame_submitted(&mut self, f: FrameSubmission) {
        self.viewer.frame_submitted(f);
        if self.send.is_none() {
            return;
        }
        let now = nanos(f.submission_timestamp);
        if f.timing.kind == FrameKind::Vsync {
            let id = self.control.diagnostic_acquired_drawable();
            match self.painted_row.take() {
                Some(PaintedListPosition {
                    first_key,
                    offset_y,
                    row_top_y,
                    scene_fingerprint,
                }) => self.frames.push(Frame {
                    drawable: id,
                    paint_sequence: self.paint_sequence,
                    target_ns: nanos(f.timing.predicted_presentation_timestamp),
                    scene_fingerprint,
                    submitted_ns: now,
                    refresh_ns: nanos(f.timing.refresh_interval),
                    first_key,
                    offset_y,
                    row_top_y,
                    presented_ns: None,
                }),
                None => {
                    self.fail = Some(Error::Application(
                        "Submitted drawable has no painted row".into(),
                    ))
                }
            }
        }
        if let Some(sample) = self.sample
            && now >= sample.end + 200_000_000
        {
            let missing = self
                .frames
                .iter()
                .any(|f| f.submitted_ns < sample.end && f.presented_ns.is_none());
            if !missing || now >= sample.end + 1_000_000_000 {
                let report = sample.report(
                    std::mem::take(&mut self.frames),
                    self.size.width,
                    self.size.height,
                    self.scale,
                    self.viewer.book.total(),
                );
                if let Some(tx) = self.send.take() {
                    let _ = tx.send(report);
                }
                if let Err(e) = self.control.request_exit() {
                    self.fail = Some(e.into());
                }
                return;
            }
        }
        if let Some(w) = &self.waker
            && let Err(e) = w.wake()
        {
            self.fail = Some(e.into());
        }
    }
}
pub(crate) fn run() -> Result<Report, Box<dyn std::error::Error + Send + Sync>> {
    let timing_path = std::env::var_os("OXIVIEW_METAL_TIMING_FILE");
    let mut platform = MacOsPlatform::default().with_diagnostics();
    if timing_path.is_some() {
        platform = platform.with_metal_timing();
    }
    let control = platform.control();
    let exit = control.clone();
    let timing_control = control.clone();
    let (tx, rx) = mpsc::channel();
    let guard = thread::Builder::new()
        .name("scroll-capture-timeout".into())
        .spawn(move || {
            rx.recv_timeout(Duration::from_secs(40)).map_err(|e| {
                let _ = exit.request_exit();
                format!("incomplete native scroll capture: {e}")
            })
        })?;
    let run = crate::run_native_viewer(platform, move || {
        Ok(Probe {
            viewer: ViewerApp::new(None)?,
            control,
            send: Some(tx),
            waker: None,
            frames: Vec::new(),
            sample: None,
            paint_sequence: 0,
            painted_row: None,
            size: PhysicalSize {
                width: 0,
                height: 0,
            },
            scale: 1.,
            fail: None,
        })
    });
    let report = guard.join().map_err(|_| "scroll capture guard panicked")?;
    if let Some(path) = timing_path {
        let samples: Vec<_> = timing_control
            .metal_timing()?
            .into_iter()
            .map(|t| {
                serde_json::json!({
                    "drawable_id": t.drawable_id,
                    "callback_time": t.callback_time,
                    "target_timestamp": t.target_timestamp,
                    "target_presentation_timestamp": t.target_presentation_timestamp,
                    "preferred_frame_latency": t.preferred_frame_latency,
                    "offered": t.offered,
                    "taken_time": t.taken_time,
                    "scene_encoded_time": t.scene_encoded_time,
                    "scene_submitted_time": t.scene_submitted_time,
                    "commit_time": t.commit_time,
                    "scheduled_time": t.scheduled_time,
                    "gpu_start_time": t.gpu_start_time,
                    "gpu_end_time": t.gpu_end_time,
                    "command_status": t.command_status,
                })
            })
            .collect();
        std::fs::write(path, serde_json::to_vec(&samples)?)?;
    }
    run?;
    let mut report = report?;
    let ticks = timing_control.diagnostic_metal_display_updates()?;
    if let Some(first) = ticks.first() {
        if ticks.iter().any(|u| {
            u.display_id != first.display_id
                || u.built_in != first.built_in
                || u.max_hz != first.max_hz
        }) {
            return Err("Display configuration changed during native scroll capture".into());
        }
        report.display = Some(Display {
            id: first.display_id,
            built_in: first.built_in,
        });
        report.display_ticks_ns = ticks.iter().map(|u| nanos(u.target)).collect();
    }
    Ok(report)
}
