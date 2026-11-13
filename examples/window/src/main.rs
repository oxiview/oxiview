//! M0's one-image demo: offscreen GPU rendering, or CPU rendering with --headless.

use oxiview_base::{Affine, Color, CornerRadii, Point, Rect};
use oxiview_platform::{Frame, WindowPresenter};
use oxiview_platform_macos::MacOsPresenter;
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_scene::{Brush, ColorStop, NodeId, NodeKind, Primitive, Scene};
use std::{error::Error, fs::File, io::BufWriter, path::Path};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 420;

fn scene() -> Result<(Scene, NodeId), oxiview_scene::Error> {
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Transform(Affine::IDENTITY))?;
    let shapes = [
        (
            Rect::new(0.0, 0.0, f64::from(WIDTH), f64::from(HEIGHT)),
            0.0,
            Brush::Solid(Color::from_rgba8(19, 27, 44, 255)),
        ),
        (
            Rect::new(64.0, 64.0, 464.0, 300.0),
            36.0,
            Brush::LinearGradient(
                Point::new(64.0, 64.0),
                Point::new(464.0, 300.0),
                vec![
                    ColorStop {
                        position: 0.0,
                        color: Color::from_rgba8(45, 212, 191, 255),
                    },
                    ColorStop {
                        position: 1.0,
                        color: Color::from_rgba8(59, 100, 246, 255),
                    },
                ],
            ),
        ),
        (
            Rect::new(330.0, 200.0, 576.0, 356.0),
            28.0,
            Brush::Solid(Color::from_rgba8(255, 186, 92, 230)),
        ),
    ];
    for (index, (rect, radius, brush)) in shapes.into_iter().enumerate() {
        scene.insert(
            Some(root),
            index,
            NodeKind::Draw(Primitive::Fill(rect, CornerRadii::uniform(radius), brush)),
        )?;
    }
    Ok((scene, root))
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut headless = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--headless" => headless = true,
            _ => return Err(format!("unknown argument: {arg}; usage: window [--headless]").into()),
        }
    }
    let (scene, root) = scene()?;
    if headless {
        let image = CpuRenderer.render(
            &scene,
            root,
            oxiview_render_cpu::PixelSize {
                width: WIDTH,
                height: HEIGHT,
            },
        )?;
        // Anchor output to the repository even when invoked from examples/window.
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
        println!("Wrote {}", path.display());
    } else {
        let image = oxiview_render_wgpu::RenderContext::new()?.render(
            &scene,
            root,
            oxiview_render_wgpu::PixelSize {
                width: WIDTH,
                height: HEIGHT,
            },
        )?;
        MacOsPresenter.present(Frame {
            width: image.width(),
            height: image.height(),
            pixels: image.pixels(),
        })?;
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("window: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
