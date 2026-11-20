//! AppKit drives frame opportunities. Metal's completed drawables supply display times.
use crate::{
    HEIGHT, MAX_OBJECTS, Options, Result, WIDTH,
    cadence::{Rates, Submission, Timeline},
    scene::{MotionClock, MotionView, Readout},
};
use oxiview_base::Size;
use oxiview_core::{
    Application, Error, FrameKind, FrameSubmission, FrameTiming, PaintedScene, Scheduler,
};
use oxiview_platform::{KeyCode, PhysicalSize, WindowAttributes, WindowEvent, WindowWaker};
use oxiview_platform_macos::{MacOsControl, MacOsPlatform};
use std::{
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread,
    time::Duration,
};

const WARMUP: Duration = Duration::from_secs(1);
const FEEDBACK_LAG: Duration = Duration::from_millis(100);
const HUD_PERIOD: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
struct Conditions {
    objects: u16,
    pixel_width: u32,
    pixel_height: u32,
    scale: f64,
    fullscreen: bool,
}

struct TimedRun {
    duration: Duration,
    fullscreen: bool,
    capture: Option<Capture>,
    done: Sender<Result<Report, String>>,
}

struct Demo {
    view: MotionView,
    clock: MotionClock,
    control: MacOsControl,
    waker: Option<WindowWaker>,
    measurement: Option<TimedRun>,
    closing: bool,
    conditions: Conditions,
    toggle_fullscreen: bool,
    paused: bool,
    active: bool,
    visible: bool,
    live: Timeline,
    live_since: Option<Duration>,
    label_at: Option<Duration>,
    paint_start: Duration,
    wake_error: Option<Error>,
}

impl Demo {
    fn new(
        control: MacOsControl,
        objects: u16,
        fullscreen: bool,
        measurement: Option<TimedRun>,
    ) -> Result<Self> {
        Ok(Self {
            view: MotionView::new()?,
            clock: MotionClock::default(),
            control,
            waker: None,
            measurement,
            closing: false,
            toggle_fullscreen: fullscreen,
            paused: false,
            active: true,
            visible: true,
            conditions: Conditions {
                objects,
                pixel_width: 0,
                pixel_height: 0,
                scale: 1.0,
                fullscreen: false,
            },
            live: Timeline::default(),
            live_since: None,
            label_at: None,
            paint_start: Duration::ZERO,
            wake_error: None,
        })
    }
    fn running(&self) -> bool {
        !self.closing
            && !self.paused
            && self.active
            && self.visible
            && self.conditions.pixel_width > 0
            && self.conditions.pixel_height > 0
    }
    fn reset_sample(&mut self) -> Result<(), Error> {
        self.live = Timeline::default();
        self.live_since = None;
        self.label_at = None;
        self.clock.hold();
        if let Some(run) = &mut self.measurement
            && run
                .capture
                .take()
                .is_some_and(|sample| sample.latest >= sample.start)
        {
            return Err(Error::Application("animation, load, or viewport changed during the measurement; rerun with stable conditions".into()));
        }
        Ok(())
    }
    fn collect_feedback(&mut self) -> Result<(), Error> {
        let raw = self.control.diagnostic_presented_drawables()?;
        if raw.is_empty() {
            return Ok(());
        }
        let records: Vec<_> = raw
            .into_iter()
            .map(|(id, seconds)| {
                Duration::try_from_secs_f64(seconds)
                    .map(|at| (id, at))
                    .map_err(|e| {
                        Error::Application(format!("invalid Metal presentation time: {e}"))
                    })
            })
            .collect::<Result<_, _>>()?;
        self.live.feedback(&records);
        if let Some(capture) = self
            .measurement
            .as_mut()
            .and_then(|run| run.capture.as_mut())
        {
            capture.frames.feedback(&records);
        }
        Ok(())
    }
    fn update_readout(&mut self, now: Duration, refresh: Duration) -> Result<(), Error> {
        if self
            .label_at
            .is_some_and(|last| now.saturating_sub(last) < HUD_PERIOD)
        {
            return Ok(());
        }
        // Use a trailing second just behind the host clock so in-flight display
        // callbacks have time to arrive. The timed report explicitly drains them.
        let end = now.saturating_sub(FEEDBACK_LAG);
        let start = end.saturating_sub(Duration::from_secs(1));
        let rates = self
            .live_since
            .filter(|since| *since <= start)
            .and_then(|_| self.live.rates(start, end));
        let state = if !self.running() {
            "Paused"
        } else if rates.is_none() {
            "Sampling…"
        } else {
            "Live"
        };
        self.view.set_readout(
            state,
            self.conditions.objects,
            Readout {
                displayed: rates.and_then(|rate| rate.displayed_fps),
                submitted: rates.map(|rate| rate.submitted_fps),
                refresh: (!refresh.is_zero()).then(|| 1.0 / refresh.as_secs_f64()),
                gap_p95_ms: rates.and_then(|rate| rate.display_gap_p95_ms),
            },
        )?;
        self.label_at = Some(now);
        Ok(())
    }
}

impl Application for Demo {
    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        waker: &WindowWaker,
    ) -> Result<(), Error> {
        if matches!(event, WindowEvent::Destroyed | WindowEvent::CloseRequested) {
            self.closing = true;
            self.waker = None;
            return Ok(());
        }
        if let Some(error) = self.wake_error.take() {
            return Err(error);
        }
        if self.waker.is_none() {
            self.waker = Some(waker.clone());
        }
        let before = (self.running(), self.conditions);
        match *event {
            WindowEvent::Resized {
                physical_size,
                scale_factor,
                display_sync,
                ..
            } => {
                self.conditions.pixel_width = physical_size.width;
                self.conditions.pixel_height = physical_size.height;
                self.conditions.scale = scale_factor;
                self.conditions.fullscreen = display_sync;
            }
            WindowEvent::ActivationChanged { active, .. } => self.active = active,
            WindowEvent::VisibilityChanged { visible, .. } => self.visible = visible,
            WindowEvent::Key {
                code,
                pressed: true,
                repeat: false,
                ..
            } => match code {
                KeyCode::Space => self.paused = !self.paused,
                KeyCode::ArrowUp => {
                    self.conditions.objects = (self.conditions.objects * 2).min(MAX_OBJECTS)
                }
                KeyCode::ArrowDown => {
                    self.conditions.objects = (self.conditions.objects / 2).max(1)
                }
                _ => {}
            },
            WindowEvent::Wake if self.running() => scheduler.invalidate(),
            _ => {}
        }
        if before != (self.running(), self.conditions) {
            self.reset_sample()?;
            if self.visible {
                scheduler.invalidate();
            }
        }
        Ok(())
    }
    fn paint(
        &mut self,
        timing: FrameTiming,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, Error> {
        self.paint_start = self.control.diagnostic_now();
        self.collect_feedback()?;
        self.update_readout(self.paint_start, timing.refresh_interval)?;
        let phase = self
            .clock
            .sample(timing.predicted_presentation_timestamp, self.running());
        Ok(self
            .view
            .paint(phase, self.conditions.objects, size, scale)?)
    }
    fn frame_submitted(&mut self, frame: FrameSubmission) {
        if self.toggle_fullscreen {
            self.toggle_fullscreen = false;
            if !self.conditions.fullscreen
                && let Err(error) = self.control.diagnostic_toggle_fullscreen()
            {
                self.wake_error = Some(error.into());
            }
        }
        if self.running() {
            if frame.timing.kind == FrameKind::Vsync {
                let submission = Submission {
                    id: self.control.diagnostic_acquired_drawable(),
                    at: frame.submission_timestamp,
                    paint_to_submit: frame.submission_timestamp.saturating_sub(self.paint_start),
                };
                self.live
                    .trim_before(submission.at.saturating_sub(Duration::from_secs(3)));
                self.live.submit(submission);
                self.live_since.get_or_insert(submission.at);
                if let Some(run) = &mut self.measurement
                    && run.fullscreen == self.conditions.fullscreen
                {
                    let capture = run.capture.get_or_insert_with(|| {
                        Capture::new(submission.at, run.duration, self.conditions)
                    });
                    capture.record(submission, frame.timing.refresh_interval);
                    let result = match capture.finish() {
                        Ok(Some(report)) => Some(Ok(report)),
                        Ok(None) => None,
                        Err(error) => Some(Err(error)),
                    };
                    if let Some(result) = result {
                        // finish() runs after the current submission consumed its
                        // invalidation. Close here before arming the next frame.
                        self.closing = true;
                        let _ = run.done.send(result);
                        if let Err(error) = self.control.request_exit() {
                            self.wake_error = Some(error.into());
                            if let Some(waker) = &self.waker {
                                let _ = waker.wake();
                            }
                        }
                        return;
                    }
                }
            }
            // A successful paint consumes the current invalidation. Request one
            // more display opportunity via the window loop, never a free-running timer.
            if let Some(waker) = &self.waker
                && let Err(error) = waker.wake()
            {
                self.wake_error = Some(error.into());
            }
        }
    }
}

#[derive(serde::Serialize)]
struct Report {
    conditions: Conditions,
    sample_seconds: f64,
    warmup_seconds: f64,
    refresh_hz_min: f64,
    refresh_hz_max: f64,
    #[serde(flatten)]
    rates: Rates,
}

struct Capture {
    start: Duration,
    end: Duration,
    latest: Duration,
    conditions: Conditions,
    frames: Timeline,
    min_interval: Duration,
    max_interval: Duration,
}
impl Capture {
    fn new(at: Duration, duration: Duration, conditions: Conditions) -> Self {
        let start = at + WARMUP;
        Self {
            start,
            end: start + duration,
            latest: at,
            conditions,
            frames: Timeline::default(),
            min_interval: Duration::MAX,
            max_interval: Duration::ZERO,
        }
    }
    fn record(&mut self, frame: Submission, interval: Duration) {
        self.latest = frame.at;
        self.frames.submit(frame);
        if (self.start..self.end).contains(&frame.at) && !interval.is_zero() {
            self.min_interval = self.min_interval.min(interval);
            self.max_interval = self.max_interval.max(interval);
        }
    }
    fn finish(&self) -> Result<Option<Report>, String> {
        if self.latest < self.end + FEEDBACK_LAG {
            return Ok(None);
        }
        if self.frames.has_pending_before(self.end) {
            return if self.latest < self.end + Duration::from_secs(2) {
                Ok(None)
            } else {
                Err("Metal presentation feedback did not arrive for every drawable before the end of the sample".into())
            };
        }
        let rates = self
            .frames
            .rates(self.start, self.end)
            .ok_or("invalid measurement interval")?;
        if rates.submitted_frames == 0
            || self.max_interval.is_zero()
            || rates.displayed_fps.is_none()
        {
            return Err("insufficient submission or Metal feedback samples; measure a longer interval with the window visible".into());
        }
        Ok(Some(Report {
            conditions: self.conditions,
            sample_seconds: (self.end - self.start).as_secs_f64(),
            warmup_seconds: WARMUP.as_secs_f64(),
            refresh_hz_min: 1.0 / self.max_interval.as_secs_f64(),
            refresh_hz_max: 1.0 / self.min_interval.as_secs_f64(),
            rates,
        }))
    }
}

pub(super) fn run(options: Options) -> Result<()> {
    let platform = MacOsPlatform::default().with_diagnostics();
    let control = platform.control();
    let (measurement, worker) = if let Some(seconds) = options.measure {
        let (tx, rx) = mpsc::channel();
        let close = control.clone();
        let duration = Duration::from_secs(u64::from(seconds));
        let worker = thread::Builder::new()
            .name("animation-fps-timeout".into())
            .spawn(
                move || match rx.recv_timeout(duration + Duration::from_secs(12)) {
                    Ok(result) => result,
                    Err(RecvTimeoutError::Disconnected) => {
                        Err("window closed before a complete animation sample".to_string())
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        close.request_exit().map_err(|e| e.to_string())?;
                        Err("timed out waiting for a stable, visible animation window".to_string())
                    }
                },
            )?;
        (
            Some(TimedRun {
                duration,
                fullscreen: options.fullscreen,
                capture: None,
                done: tx,
            }),
            Some(worker),
        )
    } else {
        (None, None)
    };
    let result = oxiview_app::run_macos(
        platform,
        WindowAttributes {
            title: "Continuous motion — oxiview".into(),
            logical_size: Size::new(f64::from(WIDTH), f64::from(HEIGHT)),
            resize_timeout: Duration::from_millis(250),
        },
        move || {
            Demo::new(control, options.objects, options.fullscreen, measurement)
                .map_err(|e| Error::Application(e.to_string()))
        },
    );
    let report = worker.map(|worker| {
        worker
            .join()
            .map_err(|_| "animation measurement worker panicked")
    });
    result?;
    if let Some(report) = report {
        let report = report??;
        if options.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!(
                "{:.1} displayed FPS, {:.1} submitted FPS, p95 display gap {} over {:.1} s ({} objects, {}×{} physical pixels).",
                report.rates.displayed_fps.ok_or("missing display rate")?,
                report.rates.submitted_fps,
                report
                    .rates
                    .display_gap_p95_ms
                    .map_or_else(|| "n/a".to_owned(), |ms| format!("{ms:.2} ms")),
                report.sample_seconds,
                report.conditions.objects,
                report.conditions.pixel_width,
                report.conditions.pixel_height
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxiview_platform::WindowWake;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Default)]
    struct WakeCount(AtomicUsize);

    impl WindowWake for WakeCount {
        fn wake(&self) -> Result<(), oxiview_platform::Error> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[test]
    fn submissions_request_the_next_opportunity_only_during_active_playback() -> Result<()> {
        let count = Arc::new(WakeCount::default());
        let waker = WindowWaker::new(count.clone());
        let mut demo = Demo::new(MacOsPlatform::default().control(), 32, false, None)?;
        demo.conditions.pixel_width = 960;
        demo.conditions.pixel_height = 600;
        let mut scheduler = Scheduler::default();
        let frame = FrameSubmission {
            timing: FrameTiming {
                predicted_presentation_timestamp: Duration::from_millis(10),
                refresh_interval: Duration::from_millis(10),
                kind: FrameKind::Vsync,
            },
            submission_timestamp: Duration::from_millis(9),
            latency: None,
        };
        let space = WindowEvent::Key {
            code: KeyCode::Space,
            logical: KeyCode::Space.us_logical_key(),
            modifiers: Default::default(),
            pressed: true,
            repeat: false,
            timestamp: Duration::ZERO,
        };
        demo.event(&WindowEvent::Wake, &mut scheduler, &waker)?;
        assert!(scheduler.schedule().request_frame);
        demo.frame_submitted(frame);
        assert_eq!(count.0.load(Ordering::Relaxed), 1);
        demo.event(&space, &mut scheduler, &waker)?;
        demo.frame_submitted(frame);
        assert_eq!(count.0.load(Ordering::Relaxed), 1);
        demo.event(&space, &mut scheduler, &waker)?;
        demo.frame_submitted(frame);
        assert_eq!(count.0.load(Ordering::Relaxed), 2);
        demo.event(
            &WindowEvent::VisibilityChanged {
                visible: false,
                timestamp: Duration::ZERO,
            },
            &mut scheduler,
            &waker,
        )?;
        demo.frame_submitted(frame);
        demo.event(&WindowEvent::Destroyed, &mut scheduler, &waker)?;
        demo.frame_submitted(frame);
        assert_eq!(count.0.load(Ordering::Relaxed), 2);
        Ok(())
    }

    #[test]
    fn timed_run_excludes_warmup_and_waits_for_delayed_display_feedback() -> Result<()> {
        let conditions = Conditions {
            objects: 32,
            pixel_width: 1920,
            pixel_height: 1200,
            scale: 2.0,
            fullscreen: false,
        };
        let mut capture = Capture::new(Duration::ZERO, Duration::from_secs(1), conditions);
        let interval = Duration::from_millis(10);
        for i in 0..=210u32 {
            let at = interval * i;
            let id = u64::from(i);
            capture.record(
                Submission {
                    id,
                    at,
                    paint_to_submit: Duration::from_millis(2),
                },
                interval,
            );
            if i != 198 {
                let displayed = if i % 2 == 0 {
                    at + Duration::from_millis(2)
                } else {
                    Duration::ZERO
                };
                capture.frames.feedback(&[(id, displayed)]);
            }
        }
        assert!(
            capture.finish()?.is_none(),
            "one measured drawable is still in flight"
        );
        capture
            .frames
            .feedback(&[(198, Duration::from_millis(1982))]);
        let report = capture.finish()?.ok_or("missing report")?;
        assert_eq!(report.rates.submitted_fps, 100.0);
        assert_eq!(report.rates.displayed_fps, Some(50.0));
        assert_eq!(report.rates.display_gap_p95_ms, Some(20.0));
        assert_eq!(report.rates.submissions_not_displayed, 50);
        Ok(())
    }
}
