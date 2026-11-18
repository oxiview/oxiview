//! Opt-in diagnostics drawn after the game, without participating in hit testing.

use oxiview_base::{Affine, Color, CornerRadii, Point, Rect, Size};
use oxiview_core::{FrameTiming, PaintedScene, UiDiagnostics};
use oxiview_platform::WindowWaker;
use oxiview_scene::{Brush, Clip, NodeKind, Primitive};
use oxiview_text::{TextLayout, TextStyle, TextSystem};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const FPS_WINDOW: Duration = Duration::from_secs(1);
const LABEL_INTERVAL: Duration = Duration::from_millis(200);
const MAX_SAMPLES: usize = 4096;

#[derive(Default)]
struct FrameMeter {
    submissions: VecDeque<Duration>,
    deadline: Option<Duration>,
    application_pending: bool,
    refresh_pending: bool,
    idle: bool,
}
impl FrameMeter {
    fn application_work(&mut self) {
        self.application_pending = true;
        self.idle = false;
    }
    fn expire(&mut self, backend_now: Duration) -> bool {
        if !self
            .deadline
            .is_some_and(|deadline| deadline <= backend_now)
        {
            return false;
        }
        self.deadline = None;
        if self.application_pending {
            return false;
        }
        self.idle = true;
        self.refresh_pending = true;
        self.submissions.clear();
        true
    }
    /// Returns whether this presentation belongs to application work.
    fn presented(&mut self, submitted_at: Duration) -> bool {
        let application_frame = self.application_pending || !self.refresh_pending;
        self.application_pending = false;
        self.refresh_pending = false;
        if application_frame {
            while self
                .submissions
                .front()
                .is_some_and(|time| submitted_at.saturating_sub(*time) >= FPS_WINDOW)
            {
                self.submissions.pop_front();
            }
            if self.submissions.len() == MAX_SAMPLES {
                self.submissions.pop_front();
            }
            self.submissions.push_back(submitted_at);
            self.deadline = Some(submitted_at.saturating_add(FPS_WINDOW));
            self.idle = false;
        }
        application_frame
    }
    fn fps(&self, backend_now: Duration) -> f64 {
        if self.idle {
            return 0.0;
        }
        self.submissions
            .iter()
            .filter(|time| backend_now.saturating_sub(**time) < FPS_WINDOW)
            .count() as f64
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SceneInfo {
    pub viewport: Size,
    pub scale: f64,
    pub nodes: usize,
    pub draws: usize,
    pub work: UiDiagnostics,
}

pub(crate) struct DebugOverlay {
    epoch: Instant,
    meter: FrameMeter,
    paint_started: Option<Instant>,
    last_submission: Duration,
    event_ms: Option<f64>,
    frame_ms: Option<f64>,
    refresh_hz: Option<f64>,
    waker: Option<WindowWaker>,
    text: TextSystem,
    layout: Option<TextLayout>,
    label: String,
    label_at: Option<Duration>,
    label_width: f64,
    last_viewport: Option<(Size, f64)>,
    force_label: bool,
    suspended: bool,
}
impl DebugOverlay {
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            meter: FrameMeter::default(),
            paint_started: None,
            last_submission: Duration::ZERO,
            event_ms: None,
            frame_ms: None,
            refresh_hz: None,
            waker: None,
            text: TextSystem::new(),
            layout: None,
            label: String::new(),
            label_at: None,
            label_width: 0.0,
            last_viewport: None,
            force_label: true,
            suspended: false,
        }
    }
    pub fn set_waker(&mut self, waker: &WindowWaker) {
        self.waker = Some(waker.clone());
    }
    pub fn deadline(&self) -> Option<Duration> {
        self.meter.deadline
    }
    pub fn fps(&self) -> f64 {
        self.meter.fps(self.last_submission)
    }
    pub fn suspend(&mut self) {
        self.suspended = true;
        self.meter.deadline = None;
        self.meter.submissions.clear();
        self.meter.idle = true;
        self.force_label = true;
    }
    pub fn resume(&mut self) {
        self.suspended = false;
    }
    pub fn event_finished(
        &mut self,
        backend_now: Duration,
        app_work: bool,
        elapsed: Duration,
        record_cost: bool,
    ) -> bool {
        if app_work {
            if self.suspended {
                self.meter.application_pending = true;
            } else {
                self.meter.application_work();
            }
        }
        let expired = !self.suspended && self.meter.expire(backend_now);
        if record_cost {
            self.event_ms = Some(elapsed.as_secs_f64() * 1000.0);
        }
        if expired {
            self.force_label = true;
        }
        expired
    }
    pub fn begin_paint(&mut self, timing: FrameTiming) {
        self.paint_started = Some(Instant::now());
        self.refresh_hz = if timing.refresh_interval.is_zero() {
            None
        } else {
            Some(1.0 / timing.refresh_interval.as_secs_f64())
        };
    }
    pub fn presented(&mut self, submitted_at: Duration) {
        let Some(started) = self.paint_started.take() else {
            return;
        };
        let elapsed = started.elapsed();
        if self.suspended {
            self.meter.application_pending = false;
            self.meter.refresh_pending = false;
            return;
        }
        if self.meter.presented(submitted_at) {
            self.last_submission = submitted_at;
            self.frame_ms = Some(elapsed.as_secs_f64() * 1000.0);
            // Scheduling is owned by the application event callback. One wake lets
            // it arm the final idle refresh after a successful presentation.
            if let Some(waker) = &self.waker {
                let _ = waker.wake();
            }
        }
    }
    pub fn close(&mut self) {
        self.meter.deadline = None;
        self.paint_started = None;
        self.waker = None;
    }
    pub fn draw(&mut self, painted: &mut PaintedScene, info: SceneInfo) -> Result<(), String> {
        if info.viewport.width() < 56.0 || info.viewport.height() < 40.0 {
            return Ok(());
        }
        let width = (info.viewport.width() - 24.0).min(320.0);
        let inner_width = width - 24.0;
        let now = self.epoch.elapsed();
        let view_changed = self.last_viewport != Some((info.viewport, info.scale));
        if self.force_label
            || view_changed
            || self
                .label_at
                .is_none_or(|last| now.saturating_sub(last) >= LABEL_INTERVAL)
        {
            let milliseconds = |value: Option<f64>| {
                value.map_or_else(|| "--".into(), |value| format!("{value:.2}"))
            };
            let refresh = self
                .refresh_hz
                .map_or_else(|| "--".into(), |value| format!("{value:.0}"));
            let idle = if self.meter.idle { " IDLE" } else { "" };
            let label = format!(
                "APP FPS {:>5.1}{}   VSYNC {} Hz\nEVENT CPU       {} ms\nPAINT → SUBMIT  {} ms\nNODES {}   DRAW {}\nLAYOUT {}   MEASURE {}   CACHE {}\nVIEW {:.0}×{:.0}  @{:.2}×",
                self.fps(),
                idle,
                refresh,
                milliseconds(self.event_ms),
                milliseconds(self.frame_ms),
                info.nodes,
                info.draws,
                info.work.layout_passes,
                info.work.measured_nodes,
                info.work.layout_cache_hits,
                info.viewport.width(),
                info.viewport.height(),
                info.scale,
            );
            if label != self.label || self.label_width != inner_width || self.layout.is_none() {
                let style = TextStyle {
                    font_size: 12.0,
                    weight: 500,
                    line_height: 1.45,
                    color: Color::from_rgba8(224, 239, 230, 255),
                    ..Default::default()
                };
                self.layout = Some(
                    self.text
                        .layout(&label, &style, Some(inner_width))
                        .map_err(|error| error.to_string())?,
                );
                self.label = label;
                self.label_width = inner_width;
            }
            self.label_at = Some(now);
            self.last_viewport = Some((info.viewport, info.scale));
            self.force_label = false;
        }
        let Some(layout) = &self.layout else {
            return Ok(());
        };
        let x = info.viewport.width() - width - 12.0;
        let y = 12.0;
        let height = (layout.size().height() + 24.0).min(info.viewport.height() - 24.0);
        let rect = Rect::new(x, y, x + width, y + height);
        let index = painted
            .scene
            .children(Some(painted.root))
            .map_err(|error| error.to_string())?
            .len();
        let root = painted
            .scene
            .insert(
                Some(painted.root),
                index,
                NodeKind::Transform(Affine::IDENTITY),
            )
            .map_err(|error| error.to_string())?;
        painted
            .scene
            .insert(
                Some(root),
                0,
                NodeKind::Draw(Primitive::Fill(
                    rect,
                    CornerRadii::uniform(11.0),
                    Brush::Solid(Color::from_rgba8(15, 25, 29, 244)),
                )),
            )
            .map_err(|error| error.to_string())?;
        let text_root = painted
            .scene
            .insert(
                Some(root),
                1,
                NodeKind::Clip(Clip::Rect(Rect::new(
                    x + 12.0,
                    y + 10.0,
                    x + width - 12.0,
                    y + height - 10.0,
                ))),
            )
            .map_err(|error| error.to_string())?;
        for (index, glyph) in self
            .text
            .glyph_images(layout, Point::new(x + 12.0, y + 12.0), info.scale)
            .map_err(|error| error.to_string())?
            .into_iter()
            .enumerate()
        {
            painted
                .scene
                .insert(
                    Some(text_root),
                    index,
                    NodeKind::Draw(Primitive::Image(glyph.rect, glyph.image)),
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rolling_fps_counts_submissions_not_monitor_ticks() {
        let mut meter = FrameMeter::default();
        for frame in 0..8 {
            let now = Duration::from_millis(frame * 125);
            meter.application_work();
            assert!(meter.presented(now));
        }
        assert_eq!(meter.fps(Duration::from_millis(999)), 8.0);
        assert_eq!(meter.fps(Duration::from_millis(1125)), 6.0);
    }
    #[test]
    fn final_idle_refresh_does_not_count_itself_or_rearm_a_timer() {
        let mut meter = FrameMeter::default();
        meter.application_work();
        assert!(meter.presented(Duration::ZERO));
        assert_eq!(meter.deadline, Some(FPS_WINDOW));
        assert!(!meter.expire(Duration::from_millis(999)));
        assert!(meter.expire(FPS_WINDOW));
        assert_eq!(meter.fps(FPS_WINDOW), 0.0);
        assert!(!meter.presented(FPS_WINDOW));
        assert_eq!(meter.deadline, None);
        assert!(!meter.expire(Duration::from_secs(1000)));
        assert_eq!(meter.fps(Duration::from_secs(1000)), 0.0);
    }
    #[test]
    fn new_work_can_replace_an_idle_refresh_and_retry_without_losing_its_identity() {
        let mut meter = FrameMeter::default();
        meter.application_work();
        meter.presented(Duration::ZERO);
        assert!(meter.expire(FPS_WINDOW));
        meter.application_work();
        // A skipped paint has no presentation callback and does not clear pending work.
        assert!(meter.application_pending);
        assert!(meter.presented(Duration::from_millis(1100)));
        assert_eq!(meter.deadline, Some(Duration::from_millis(2100)));
        assert_eq!(meter.fps(Duration::from_millis(1100)), 1.0);
    }
}
