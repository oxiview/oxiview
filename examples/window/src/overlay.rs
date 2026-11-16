//! Optional example-only visualization; never requests frames or starts timers.
use oxiview_base::{Color, CornerRadii, Rect};
use oxiview_core::{LatencyRecord, PaintedScene};
use oxiview_platform::PhysicalSize;
use oxiview_scene::{Brush, NodeKind, Primitive};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub(super) struct Overlay {
    start: Instant,
    printed: Instant,
    interval: Duration,
    recent: VecDeque<(f64, Option<f64>)>,
    frames: u64,
    paint_sum: f64,
    paint_max: f64,
    input_count: u64,
    input_sum: f64,
    input_max: f64,
}
impl Overlay {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            printed: Instant::now(),
            interval: Duration::ZERO,
            recent: VecDeque::with_capacity(60),
            frames: 0,
            paint_sum: 0.0,
            paint_max: 0.0,
            input_count: 0,
            input_sum: 0.0,
            input_max: 0.0,
        }
    }
    pub fn begin(&mut self, interval: Duration) {
        self.start = Instant::now();
        self.interval = interval;
    }
    pub fn submitted(&mut self, record: Option<LatencyRecord>) {
        let paint = self.start.elapsed().as_secs_f64() * 1000.0;
        let input = record
            .and_then(|r| r.submit_latency())
            .map(|d| d.as_secs_f64() * 1000.0);
        if self.recent.len() == 60 {
            self.recent.pop_front();
        }
        self.recent.push_back((paint, input));
        self.frames += 1;
        self.paint_sum += paint;
        self.paint_max = self.paint_max.max(paint);
        if let Some(input) = input {
            self.input_count += 1;
            self.input_sum += input;
            self.input_max = self.input_max.max(input);
        }
        if self.printed.elapsed() >= Duration::from_secs(1) {
            eprintln!(
                "frames={} paint→submit avg/max={:.3}/{:.3} ms input→submit avg/max={:.3}/{:.3} ms refresh={:.3} ms",
                self.frames,
                self.paint_sum / self.frames as f64,
                self.paint_max,
                self.input_sum / self.input_count.max(1) as f64,
                self.input_max,
                self.interval.as_secs_f64() * 1000.0
            );
            self.frames = 0;
            self.paint_sum = 0.0;
            self.paint_max = 0.0;
            self.input_count = 0;
            self.input_sum = 0.0;
            self.input_max = 0.0;
            self.printed = Instant::now();
        }
    }
    pub fn draw(
        &self,
        scene: &mut PaintedScene,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<(), oxiview_scene::Error> {
        let left = f64::from(size.width) - 260.0 * scale;
        let bottom = 120.0 * scale;
        let mut index = 2;
        let mut rectangle = |x: f64, y: f64, width: f64, height: f64, color| {
            let result = scene.scene.insert(
                Some(scene.root),
                index,
                NodeKind::Draw(Primitive::Fill(
                    Rect::new(x, y, x + width, y + height),
                    CornerRadii::uniform(0.0),
                    Brush::Solid(color),
                )),
            );
            index += 1;
            result.map(|_| ())
        };
        rectangle(
            left,
            10.0 * scale,
            250.0 * scale,
            115.0 * scale,
            Color::from_rgba8(8, 12, 20, 255),
        )?;
        let refresh = self.interval.as_secs_f64() * 1000.0;
        let height = |ms: f64| (ms / refresh.max(0.001) * 50.0).min(100.0) * scale;
        for (i, (paint, input)) in self.recent.iter().enumerate() {
            let x = left + (5.0 + i as f64 * 4.0) * scale;
            rectangle(
                x,
                bottom - height(*paint),
                1.5 * scale,
                height(*paint),
                Color::from_rgba8(45, 212, 191, 255),
            )?;
            if let Some(input) = input {
                rectangle(
                    x + 1.5 * scale,
                    bottom - height(*input),
                    1.5 * scale,
                    height(*input),
                    Color::from_rgba8(251, 191, 36, 255),
                )?;
            }
        }
        rectangle(
            left + 4.0 * scale,
            bottom - 50.0 * scale,
            242.0 * scale,
            scale,
            Color::from_rgba8(240, 240, 240, 255),
        )
    }
}
