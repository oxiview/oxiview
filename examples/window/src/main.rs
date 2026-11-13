//! M1 native, on-demand pointer-following rectangle demo.
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
    #[cfg(all(target_os = "macos", debug_assertions))]
    probe: Option<(
        oxiview_platform_macos::MacOsControl,
        mpsc::Sender<(Point, u64)>,
    )>,
}
impl Application for Demo {
    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        _: &WindowWaker,
    ) -> Result<(), oxiview_core::Error> {
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
        _: FrameTiming,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, oxiview_core::Error> {
        scene(size, scale, self.pointer)
            .map_err(|e| oxiview_core::Error::Application(e.to_string()))
    }
    fn presented(&mut self, _: Option<LatencyRecord>) {
        self.presents.fetch_add(1, Ordering::SeqCst);
        #[cfg(all(target_os = "macos", debug_assertions))]
        if let Some((control, sender)) = &self.probe {
            let _ = sender.send((self.pointer, control.test_acquired_drawable()));
        }
        if let Some(first) = self.first.take() {
            let _ = first.send(());
        }
    }
}
// Metal defines presentedTime as the host time the drawable appeared on screen;
// it is zero for unpresented/skipped frames. GPU completion and present() calls
// alone do not provide this guarantee. No pixel/visual correctness is inferred.
#[cfg(all(target_os = "macos", debug_assertions))]
fn wait_for_display(
    control: &oxiview_platform_macos::MacOsControl,
    expected: u64,
) -> Result<(u64, f64), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let displayed = control
            .test_presented_drawables()
            .map_err(|e| e.to_string())?;
        let mut newest = None;
        for (id, time) in displayed {
            println!("Native presentation: drawable={id}; presentedTime={time:.9}s");
            if id == expected {
                newest = Some((id, time));
            }
        }
        if let Some(frame) = newest {
            return Ok(frame);
        }
        if std::time::Instant::now() >= deadline {
            return Err(
                "expected drawable has no nonzero MTLDrawable.presentedTime within 5s".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn run(presents: Arc<AtomicU64>) -> Result<(), Box<dyn Error>> {
    let mut headless = false;
    let mut idle = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--headless" => headless = true,
            "--idle-check" => {
                let seconds: f64 = args.next().ok_or("missing idle duration")?.parse()?;
                idle = Some(Duration::try_from_secs_f64(seconds)?);
            }
            _ => {
                return Err(format!(
                    "unknown argument: {arg}; usage: window [--headless | --idle-check seconds]"
                )
                .into());
            }
        }
    }
    if headless {
        if idle.is_some() {
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
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let platform = oxiview_platform_macos::MacOsPlatform::default();
        let control = platform.control();
        let (tx, rx) = mpsc::channel();
        #[cfg(debug_assertions)]
        let input_check = std::env::var_os("OXIVIEW_WINDOW_INPUT_CHECK").is_some();
        #[cfg(not(debug_assertions))]
        let input_check = false;
        #[cfg(debug_assertions)]
        let (frame_tx, frame_rx) = mpsc::channel();
        #[cfg(debug_assertions)]
        let probe = input_check.then(|| (control.clone(), frame_tx));
        let idle_worker = (idle.is_some() || input_check).then(|| {
            let duration = idle.unwrap_or_default();
            let presents = presents.clone();
            std::thread::spawn(move || -> Result<(), String> {
                rx.recv().map_err(|e| e.to_string())?;
                #[cfg(debug_assertions)]
                if input_check {
                    let check = (|| -> Result<(), String> {
                        // Post one move at a time. Match the drawable ID reported
                        // by the window thread after presenting that pointer's scene.
                        // The initial drawable can be skipped before the window
                        // becomes visible; it is not evidence for this check.
                        for position in [Point::new(100.0, 100.0), Point::new(500.0, 300.0)] {
                            control.post_test_input(Some(position)).map_err(|e| e.to_string())?;
                            let deadline = std::time::Instant::now() + Duration::from_secs(5);
                            let drawable = loop {
                                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                                let (painted_pointer, drawable) = frame_rx.recv_timeout(remaining)
                                    .map_err(|e| format!("pointer frame not submitted: {e}"))?;
                                if painted_pointer == position {
                                    break drawable;
                                }
                            };
                            wait_for_display(&control, drawable)?;
                            println!("Pointer ({}, {}): drawable={drawable} displayed", position.x(), position.y());
                        }
                        println!("Native presentation check: two posted NSEvent pointer moves reached the display (MTLDrawable.presentedTime > 0); requesting native close");
                        Ok(())
                    })();
                    if check.is_ok() {
                        control.post_test_input(None).map_err(|e| e.to_string())?;
                    } else {
                        control.request_exit().map_err(|e| e.to_string())?;
                    }
                    return check;
                }
                let initial = presents.load(Ordering::SeqCst);
                let initial_events = control.frame_events().map_err(|e| e.to_string())?;
                std::thread::sleep(duration);
                let frames = presents.load(Ordering::SeqCst) - initial;
                let events = control.frame_events().map_err(|e| e.to_string())? - initial_events;
                println!("Idle interval: {:.3}s; presented frames: {frames}; display-link frame events: {events}", duration.as_secs_f64());
                control.request_exit().map_err(|e| e.to_string())?;
                if frames != 0 || events != 0 { return Err("idle frame check failed".into()); }
                Ok(())
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
                    #[cfg(debug_assertions)]
                    probe,
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
        idle_result?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    Err("native demo requires macOS 14 or later".into())
}
fn main() -> std::process::ExitCode {
    let presents = Arc::new(AtomicU64::new(0));
    let result = run(presents.clone());
    println!(
        "Total presented frames: {}",
        presents.load(Ordering::SeqCst)
    );
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("window: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
