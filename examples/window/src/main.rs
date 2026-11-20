//! Native on-demand M1 diagnostics and the interactive M2 layout preview.
use oxiview_base::{Affine, Color, CornerRadii, Point, Rect, Size};
use oxiview_core::{Application, FrameTiming, LatencyRecord, PaintedScene, Scheduler};
use oxiview_platform::{PhysicalSize, WindowAttributes, WindowEvent, WindowWaker};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_scene::{Brush, NodeKind, Primitive, Scene};
use std::{
    error::Error,
    fs::File,
    io::BufWriter,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

mod layout_demo;
#[cfg(target_os = "macos")]
mod measurement;
mod overlay;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 420;
fn scene(
    size: PhysicalSize,
    scale: f64,
    pointer: Point,
) -> Result<PaintedScene, oxiview_scene::Error> {
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Transform(Affine::IDENTITY))?;
    let x = pointer.x() * scale;
    let y = pointer.y() * scale;
    for (index, (rect, radius, color)) in [
        (
            Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
            0.0,
            Color::from_rgba8(19, 27, 44, 255),
        ),
        (
            Rect::new(
                x - 64.0 * scale,
                y - 44.0 * scale,
                x + 64.0 * scale,
                y + 44.0 * scale,
            ),
            20.0 * scale,
            Color::from_rgba8(45, 212, 191, 255),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        scene.insert(
            Some(root),
            index,
            NodeKind::Draw(Primitive::Fill(
                rect,
                CornerRadii::uniform(radius),
                Brush::Solid(color),
            )),
        )?;
    }
    Ok(PaintedScene { scene, root })
}
struct Demo {
    pointer: Point,
    presents: Arc<AtomicU64>,
    first: Option<mpsc::Sender<()>>,
    #[cfg(target_os = "macos")]
    probe: Option<(
        oxiview_platform_macos::MacOsControl,
        mpsc::Sender<measurement::Frame>,
    )>,
    timing: Option<FrameTiming>,
    #[cfg(target_os = "macos")]
    paint_started: Duration,
    input_events: Arc<AtomicU64>,
    fullscreen: bool,
    overlay: Option<overlay::Overlay>,
}
impl Application for Demo {
    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        _: &WindowWaker,
    ) -> Result<(), oxiview_core::Error> {
        if matches!(
            event,
            WindowEvent::PointerMoved { .. }
                | WindowEvent::PointerButton { .. }
                | WindowEvent::Key { .. }
        ) {
            self.input_events.fetch_add(1, Ordering::SeqCst);
        }
        if let WindowEvent::Resized { display_sync, .. } = event {
            self.fullscreen = *display_sync;
        }
        if let WindowEvent::PointerMoved { position, .. } = event
            && self.pointer != *position
        {
            self.pointer = *position;
            scheduler.invalidate();
        }
        Ok(())
    }
    fn paint(
        &mut self,
        timing: FrameTiming,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, oxiview_core::Error> {
        #[cfg(target_os = "macos")]
        if let Some((control, _)) = &self.probe {
            self.paint_started = control.diagnostic_now();
        }
        self.timing = Some(timing);
        if let Some(overlay) = &mut self.overlay {
            overlay.begin(timing.refresh_interval);
        }
        let mut painted = scene(size, scale, self.pointer)
            .map_err(|e| oxiview_core::Error::Application(e.to_string()))?;
        if let Some(overlay) = &self.overlay {
            overlay
                .draw(&mut painted, size, scale)
                .map_err(|e| oxiview_core::Error::Application(e.to_string()))?;
        }
        Ok(painted)
    }
    fn presented(&mut self, record: Option<LatencyRecord>) {
        if let Some(overlay) = &mut self.overlay {
            overlay.submitted(record);
        }
        self.presents.fetch_add(1, Ordering::SeqCst);
        #[cfg(target_os = "macos")]
        if let Some((control, sender)) = &self.probe
            && let Some(timing) = self.timing
        {
            let _ = sender.send(measurement::Frame {
                pointer: self.pointer,
                fullscreen: self.fullscreen,
                drawable: control.diagnostic_acquired_drawable(),
                slot: control.diagnostic_acquired_slot(),
                paint_started: self.paint_started,
                timing,
                record,
            });
        }
        if let Some(first) = self.first.take() {
            let _ = first.send(());
        }
    }
}
fn run(presents: Arc<AtomicU64>) -> Result<bool, Box<dyn Error>> {
    let mut headless = false;
    let mut idle = None;
    let mut samples = None;
    let mut json = false;
    let mut debug_overlay = false;
    let mut fullscreen_check = false;
    let mut trace_input = false;
    // The standalone preview bundle uses this executable name as its default mode.
    let mut layout_demo = std::env::current_exe().ok().is_some_and(|path| {
        path.file_name()
            .is_some_and(|name| name == "oxiview-layout-lab")
    });
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--debug-overlay" => debug_overlay = true,
            "--fullscreen-check" => fullscreen_check = true,
            "--layout-demo" => layout_demo = true,
            "--trace-input" => trace_input = true,
            "--measure-latency" => {
                let count: usize = args.next().ok_or("missing sample count")?.parse()?;
                if count == 0 || count > 100_000 {
                    return Err("samples must be in 1..=100000".into());
                }
                samples = Some(count);
            }
            "--headless" => headless = true,
            "--idle-check" => {
                let seconds: f64 = args.next().ok_or("missing idle duration")?.parse()?;
                let duration = Duration::try_from_secs_f64(seconds)?;
                if duration.is_zero() {
                    return Err("idle duration must be positive".into());
                }
                idle = Some(duration);
            }
            _ => {
                return Err(format!(
                    "unknown argument: {arg}; usage: window [--layout-demo [--headless] [--trace-input] | --headless | --idle-check seconds [--json] | --measure-latency samples [--json] | --fullscreen-check [--debug-overlay] [--json]]"
                )
                .into());
            }
        }
    }
    if layout_demo {
        if idle.is_some() || samples.is_some() || json || fullscreen_check || debug_overlay {
            return Err("--layout-demo supports --headless and --trace-input".into());
        }
        return layout_demo::run(headless, trace_input, presents);
    }
    if trace_input {
        return Err("--trace-input requires --layout-demo".into());
    }
    if debug_overlay && (idle.is_some() || samples.is_some() || headless) {
        return Err("--debug-overlay requires a native window and is supported only interactively or with --fullscreen-check".into());
    }
    if fullscreen_check && (idle.is_some() || samples.is_some() || headless) {
        return Err("select exactly one measurement mode".into());
    }
    if idle.is_some() && samples.is_some() {
        return Err("select exactly one measurement mode".into());
    }
    if json && idle.is_none() && samples.is_none() && !fullscreen_check {
        return Err("--json requires a measurement mode".into());
    }
    if headless {
        if idle.is_some() || samples.is_some() {
            return Err("--idle-check requires a native window".into());
        }
        let painted = scene(
            PhysicalSize {
                width: WIDTH,
                height: HEIGHT,
            },
            1.0,
            Point::new(320.0, 210.0),
        )?;
        let image = CpuRenderer.render(
            &painted.scene,
            painted.root,
            oxiview_scene::PixelSize {
                width: WIDTH,
                height: HEIGHT,
            },
        )?;
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/xtask/window.png");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut encoder = png::Encoder::new(BufWriter::new(File::create(&path)?), WIDTH, HEIGHT);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(image.pixels())?;
        writer.finish()?;
        presents.fetch_add(1, Ordering::SeqCst);
        println!("Wrote {}", path.display());
        return Ok(true);
    }
    #[cfg(target_os = "macos")]
    {
        let platform = oxiview_platform_macos::MacOsPlatform::default();
        let platform = if idle.is_some() || samples.is_some() || fullscreen_check {
            platform.with_diagnostics()
        } else {
            platform
        };
        let control = platform.control();
        let (tx, rx) = mpsc::channel();
        let (frame_tx, frame_rx) = mpsc::channel();
        let measuring = idle.is_some() || samples.is_some() || fullscreen_check;
        let input_events = Arc::new(AtomicU64::new(0));
        let observed_inputs = input_events.clone();
        let probe = measuring.then(|| (control.clone(), frame_tx));
        let idle_worker = measuring.then(|| {
            let presents = presents.clone();
            std::thread::spawn(move || -> Result<bool, String> {
                let result = (|| {
                    rx.recv_timeout(Duration::from_secs(10)).map_err(|e| {
                        format!("first frame unavailable (display session/GPU required): {e}")
                    })?;
                    if fullscreen_check {
                        measurement::fullscreen(&control, &frame_rx, json)
                    } else if let Some(count) = samples {
                        measurement::latency(&control, &frame_rx, count, json)
                    } else {
                        measurement::idle(
                            &control,
                            &presents,
                            &observed_inputs,
                            idle.unwrap_or_default(),
                            json,
                        )
                    }
                })();
                let close = control.request_exit().map_err(|e| e.to_string());
                close?;
                result
            })
        });
        let result = oxiview_app::run_macos(
            platform,
            WindowAttributes {
                title: "oxiview — M1".into(),
                logical_size: Size::new(f64::from(WIDTH), f64::from(HEIGHT)),
                resize_timeout: Duration::from_millis(250),
            },
            move || {
                Ok(Demo {
                    pointer: Point::new(320.0, 210.0),
                    presents,
                    first: Some(tx),
                    probe,
                    timing: None,
                    paint_started: Duration::ZERO,
                    input_events,
                    fullscreen: false,
                    overlay: debug_overlay.then(overlay::Overlay::new),
                })
            },
        );
        let idle_result = idle_worker
            .map(|worker| {
                worker
                    .join()
                    .map_err(|_| "idle checker panicked")?
                    .map_err(|e| -> Box<dyn Error> { e.into() })
            })
            .transpose();
        result?;
        Ok(idle_result?.unwrap_or(true))
    }
    #[cfg(not(target_os = "macos"))]
    Err("native demo requires macOS 14 or later".into())
}
fn main() -> std::process::ExitCode {
    let presents = Arc::new(AtomicU64::new(0));
    let result = run(presents.clone());
    if !std::env::args().any(|a| a == "--json") {
        println!(
            "Total presented frames: {}",
            presents.load(Ordering::SeqCst)
        );
    }
    match result {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::FAILURE,
        Err(error) => {
            eprintln!("window: {error}");
            std::process::ExitCode::from(2)
        }
    }
}
