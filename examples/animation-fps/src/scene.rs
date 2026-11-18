//! Retained native scene and time-based motion used by both render paths.
use crate::{HEIGHT, Result, WIDTH};
use oxiview_base::{Color, CornerRadii, Point, Rect, Size};
use oxiview_core::{NodeId, PaintedScene, UiError, UiTree, text};
use oxiview_platform::PhysicalSize;
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_scene::{Brush, PixelSize, Primitive};
use oxiview_style::{Alignment, Edges, FlexDirection, LayoutStyle, Length, Overflow, Style};
use oxiview_text::{TextStyle, TextSystem};
use std::{
    cell::RefCell, f64::consts::PI, fs::File, io::BufWriter, path::Path, rc::Rc, time::Duration,
};

type Rgb = [u8; 3];
const INK: Rgb = [26, 46, 44];
const MUTED: Rgb = [81, 101, 96];
fn color([r, g, b]: Rgb, alpha: u8) -> Color {
    Color::from_rgba8(r, g, b, alpha)
}
fn px(value: f64) -> Length {
    Length::Points(value)
}
fn style(layout: LayoutStyle) -> Style {
    Style {
        layout,
        hit_test: false,
        ..Default::default()
    }
}
fn column() -> LayoutStyle {
    LayoutStyle {
        flex_direction: FlexDirection::Column,
        min_width: px(0.0),
        min_height: px(0.0),
        ..Default::default()
    }
}

/// Positions are sampled from the backend's predicted display clock. Pausing
/// discards the anchor so inactive time cannot cause a jump on resumption.
#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
pub(super) struct MotionClock {
    elapsed: Duration,
    previous: Option<Duration>,
}
#[cfg(any(target_os = "macos", test))]
impl MotionClock {
    pub fn hold(&mut self) {
        self.previous = None;
    }
    pub fn sample(&mut self, display_time: Duration, running: bool) -> Duration {
        if running {
            if let Some(previous) = self.previous {
                self.elapsed = self
                    .elapsed
                    .saturating_add(display_time.saturating_sub(previous));
                self.previous = Some(previous.max(display_time));
            } else {
                self.previous = Some(display_time);
            }
        } else {
            self.hold();
        }
        self.elapsed
    }
}

#[derive(Default)]
pub(super) struct Readout {
    pub displayed: Option<f64>,
    pub submitted: Option<f64>,
    pub refresh: Option<f64>,
    pub gap_p95_ms: Option<f64>,
}

pub(super) struct MotionView {
    tree: UiTree,
    canvas: NodeId,
    status: NodeId,
    displayed: NodeId,
    submitted: NodeId,
    refresh: NodeId,
    gap: NodeId,
}

fn label(
    tree: &mut UiTree,
    parent: NodeId,
    fonts: &Rc<RefCell<TextSystem>>,
    value: &str,
    font_size: f64,
    weight: u16,
    ink: Rgb,
) -> Result<NodeId> {
    let font = TextStyle {
        font_size,
        weight,
        color: color(ink, 255),
        line_height: 1.18,
        ..Default::default()
    };
    Ok(tree.mount(
        parent,
        text(Rc::clone(fonts), value, font).style(style(column())),
    )?)
}

fn metric(
    tree: &mut UiTree,
    header: NodeId,
    fonts: &Rc<RefCell<TextSystem>>,
    caption: &str,
    ink: Rgb,
) -> Result<NodeId> {
    let group = tree.append(
        header,
        style(LayoutStyle {
            width: px(110.0),
            row_gap: px(4.0),
            ..column()
        }),
    )?;
    label(tree, group, fonts, caption, 13.0, 550, MUTED)?;
    label(tree, group, fonts, "—", 29.0, 700, ink)
}

impl MotionView {
    pub fn new() -> Result<Self> {
        let fonts = Rc::new(RefCell::new(TextSystem::new()));
        let mut tree = UiTree::new()?;
        let root = tree.root();
        let mut root_style = style(LayoutStyle {
            width: Length::Percent(1.0),
            height: Length::Percent(1.0),
            padding: Edges::all(px(24.0)),
            row_gap: px(16.0),
            ..column()
        });
        root_style.background = color([242, 245, 238], 255);
        tree.set_style(root, root_style)?;
        tree.layout(Size::new(f64::from(WIDTH), f64::from(HEIGHT)))?;
        let header = tree.append(
            root,
            style(LayoutStyle {
                height: px(72.0),
                flex_shrink: 0.0,
                column_gap: px(14.0),
                align_items: Alignment::Center,
                ..Default::default()
            }),
        )?;
        let title = tree.append(
            header,
            style(LayoutStyle {
                flex_grow: 1.0,
                flex_basis: px(0.0),
                row_gap: px(4.0),
                ..column()
            }),
        )?;
        label(
            &mut tree,
            title,
            &fonts,
            "Continuous motion",
            28.0,
            720,
            INK,
        )?;
        let status = label(&mut tree, title, &fonts, "Sampling", 14.0, 450, MUTED)?;
        let displayed = metric(&mut tree, header, &fonts, "Displayed FPS", [17, 108, 89])?;
        let submitted = metric(&mut tree, header, &fonts, "Submitted FPS", INK)?;
        let refresh = metric(&mut tree, header, &fonts, "Refresh Hz", INK)?;
        let mut field = style(LayoutStyle {
            flex_grow: 1.0,
            min_height: px(0.0),
            ..column()
        });
        field.background = color([20, 40, 42], 255);
        field.border_radii = CornerRadii::uniform(18.0);
        field.overflow = Overflow::Clip;
        let canvas = tree.append(root, field)?;
        let footer = tree.append(
            root,
            style(LayoutStyle {
                height: px(28.0),
                flex_shrink: 0.0,
                column_gap: px(12.0),
                align_items: Alignment::Center,
                ..Default::default()
            }),
        )?;
        let keys = tree.append(
            footer,
            style(LayoutStyle {
                flex_grow: 1.0,
                flex_basis: px(0.0),
                ..column()
            }),
        )?;
        label(
            &mut tree,
            keys,
            &fonts,
            "Space  Pause / resume      ↑ ↓  Object count",
            14.0,
            500,
            MUTED,
        )?;
        let gap = label(
            &mut tree,
            footer,
            &fonts,
            "P95 display gap   —",
            14.0,
            600,
            INK,
        )?;
        Ok(Self {
            tree,
            canvas,
            status,
            displayed,
            submitted,
            refresh,
            gap,
        })
    }

    pub fn set_readout(
        &mut self,
        status: &str,
        objects: u16,
        data: Readout,
    ) -> Result<(), UiError> {
        self.tree.set_text(
            self.status,
            format!("{objects} moving objects   ·   {status}"),
        )?;
        let rate = |number: Option<f64>| number.map_or_else(|| "—".into(), |n| format!("{n:.1}"));
        self.tree.set_text(self.displayed, rate(data.displayed))?;
        self.tree.set_text(self.submitted, rate(data.submitted))?;
        self.tree.set_text(
            self.refresh,
            data.refresh
                .map_or_else(|| "—".into(), |n| format!("{n:.0}")),
        )?;
        self.tree.set_text(
            self.gap,
            data.gap_p95_ms.map_or_else(
                || "P95 display gap   —".into(),
                |ms| format!("P95 display gap   {ms:.2} ms"),
            ),
        )
    }

    pub fn paint(
        &mut self,
        phase: Duration,
        objects: u16,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, UiError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(UiError::InvalidViewport);
        }
        let viewport = Size::new(
            f64::from(size.width) / scale,
            f64::from(size.height) / scale,
        );
        self.tree.layout(viewport)?;
        let bounds = self.tree.bounds(self.canvas)?;
        self.tree
            .set_content(self.canvas, tracks(bounds.size(), phase, objects))?;
        self.tree.paint(viewport, scale)
    }
}

// Constant distance per second along two straights and two semicircles.
fn on_track(x: f64, y: f64, straight: f64, r: f64, distance: f64) -> Point {
    let arc = PI * r;
    let s = distance.rem_euclid(2.0 * (straight + arc));
    if s < straight {
        Point::new(x + s, y - r)
    } else if s < straight + arc {
        let a = (s - straight) / r - PI / 2.0;
        Point::new(x + straight + r * a.cos(), y + r * a.sin())
    } else if s < 2.0 * straight + arc {
        Point::new(x + 2.0 * straight + arc - s, y + r)
    } else {
        let a = (s - 2.0 * straight - arc) / r + PI / 2.0;
        Point::new(x + r * a.cos(), y + r * a.sin())
    }
}

fn dot(at: Point, radius: f64, tint: Rgb, alpha: u8) -> Primitive {
    Primitive::Fill(
        Rect::new(
            at.x() - radius,
            at.y() - radius,
            at.x() + radius,
            at.y() + radius,
        ),
        CornerRadii::uniform(radius),
        Brush::Solid(color(tint, alpha)),
    )
}

fn tracks(size: Size, phase: Duration, count: u16) -> Vec<Primitive> {
    let (w, h) = (size.width(), size.height());
    if w < 48.0 || h < 48.0 {
        return Vec::new();
    }
    let margin = (w * 0.06).min(48.0);
    let lane_height = h / 3.0;
    let radius = (lane_height * 0.28).min((w - 2.0 * margin) / 4.0);
    let straight = w - 2.0 * (margin + radius);
    let lap = 2.0 * (straight + PI * radius);
    let bead = (lane_height * 0.075).clamp(2.0, 9.0);
    let mut out = Vec::with_capacity(3 + usize::from(count) * 2);
    for (lane, (tint, period)) in [
        ([126, 224, 188], 4.1),
        ([244, 181, 112], -5.3),
        ([168, 186, 247], 6.7),
    ]
    .into_iter()
    .enumerate()
    {
        let center = lane_height * (lane as f64 + 0.5);
        let rect = Rect::new(margin, center - radius, w - margin, center + radius);
        out.push(Primitive::Stroke(
            rect,
            CornerRadii::uniform(radius),
            2.0,
            Brush::Solid(color(tint, 58)),
        ));
        let per_lane = (usize::from(count) + 2 - lane) / 3;
        for i in 0..per_lane {
            let fraction =
                i as f64 / per_lane as f64 + lane as f64 * 0.08 + phase.as_secs_f64() / period;
            let distance = fraction * lap;
            let at = |s| on_track(margin + radius, center, straight, radius, s);
            out.push(dot(
                at(distance - period.signum() * bead * 1.65),
                bead * 0.65,
                tint,
                94,
            ));
            out.push(dot(at(distance), bead, tint, 255));
        }
    }
    out
}

pub(super) fn export(objects: u16) -> Result<()> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/xtask/animation-fps");
    std::fs::create_dir_all(&dir)?;
    let mut view = MotionView::new()?;
    view.set_readout("CPU preview", objects, Readout::default())?;
    for (ms, name) in [(0, "phase-0.png"), (125, "phase-125ms.png")] {
        let scene = view.paint(
            Duration::from_millis(ms),
            objects,
            PhysicalSize {
                width: WIDTH,
                height: HEIGHT,
            },
            1.0,
        )?;
        let image = CpuRenderer.render(
            &scene.scene,
            scene.root,
            PixelSize {
                width: WIDTH,
                height: HEIGHT,
            },
        )?;
        let mut encoder =
            png::Encoder::new(BufWriter::new(File::create(dir.join(name))?), WIDTH, HEIGHT);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut file = encoder.write_header()?;
        file.write_image_data(image.pixels())?;
        file.finish()?;
    }
    println!("Two animation phases written to {}", dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sampling_at_different_refresh_rates_reaches_the_same_geometry_and_pause_freezes_it() {
        let mut slow = MotionClock::default();
        let mut fast = MotionClock::default();
        for i in 0..=50 {
            slow.sample(Duration::from_millis(i * 20), true);
        }
        for i in 0..=100 {
            fast.sample(Duration::from_millis(i * 10), true);
        }
        let size = Size::new(800.0, 400.0);
        let t = slow.sample(Duration::from_secs(1), true);
        let moving = tracks(size, t, 32);
        assert_ne!(moving, tracks(size, Duration::ZERO, 32));
        assert_eq!(
            moving,
            tracks(size, fast.sample(Duration::from_secs(1), true), 32)
        );
        assert_eq!(fast.sample(Duration::from_secs(40), false), t);
        assert_eq!(fast.sample(Duration::from_secs(60), true), t);
        assert_eq!(
            fast.sample(Duration::from_millis(60_010), true),
            t + Duration::from_millis(10)
        );
    }
}
