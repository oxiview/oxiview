//! Cross-adapter comparisons belong to the composition root, not either adapter.
#[path = "../../oxiview-render-cpu/tests/scenes/mod.rs"]
mod scenes;
use oxiview_base::{Affine, CornerRadii, Rect};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_render_wgpu::RenderContext;
use oxiview_scene::{Brush, Clip, NodeKind, PixelSize, Primitive, Scene};
use scenes::TestResult;

const SIZE: PixelSize = PixelSize {
    width: 64,
    height: 64,
};

#[test]
fn coincident_gradient_endpoints_sample_the_last_stop_at_zero() -> TestResult {
    use oxiview_base::{Color, Point};
    use oxiview_scene::ColorStop;
    let gpu = RenderContext::new()?;
    let red = Color::from_rgba8(255, 0, 0, 255);
    let blue = Color::from_rgba8(0, 0, 255, 255);
    let brush = Brush::LinearGradient(
        Point::default(),
        Point::default(),
        [red, blue]
            .map(|color| ColorStop {
                position: 0.0,
                color,
            })
            .into(),
    );
    for transform in [
        Affine::IDENTITY,
        Affine::new([1.0, 0.0, 0.2, 1.0, 0.0, 0.0]),
    ] {
        let mut scene = Scene::new();
        let root = scene.insert(None, 0, NodeKind::Transform(transform))?;
        scene.insert(
            Some(root),
            0,
            NodeKind::Draw(Primitive::Fill(
                Rect::new(0.0, 0.0, 32.0, 32.0),
                CornerRadii::default(),
                brush.clone(),
            )),
        )?;
        for image in [
            CpuRenderer.render(&scene, root, SIZE)?,
            gpu.render(&scene, root, SIZE)?,
        ] {
            let center = (16 * SIZE.width as usize + 16) * 4;
            assert_eq!(&image.pixels()[center..center + 4], &[0, 0, 255, 255]);
        }
    }
    Ok(())
}

fn metrics(a: &[u8], b: &[u8]) -> (u8, f64) {
    let mut max = 0;
    let mut differing = 0;
    for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let premul = |p: &[u8]| {
            [
                ((u16::from(p[0]) * u16::from(p[3]) + 127) / 255) as u8,
                ((u16::from(p[1]) * u16::from(p[3]) + 127) / 255) as u8,
                ((u16::from(p[2]) * u16::from(p[3]) + 127) / 255) as u8,
                p[3],
            ]
        };
        let a = premul(a);
        let b = premul(b);
        let delta = a
            .iter()
            .zip(b)
            .map(|(a, b)| a.abs_diff(b))
            .max()
            .unwrap_or(0);
        max = max.max(delta);
        differing += usize::from(delta > 4);
    }
    (max, differing as f64 / f64::from(SIZE.width * SIZE.height))
}

#[allow(clippy::panic)] // Every comparison must reject a one-pixel displacement.
fn assert_shift_rejected(name: &str, cpu: &[u8], gpu: &[u8]) {
    let mut shifted = vec![0; gpu.len()];
    for (out, row) in shifted.chunks_exact_mut(256).zip(gpu.chunks_exact(256)) {
        out[4..].copy_from_slice(&row[..252]);
    }
    let (max, ratio) = metrics(cpu, &shifted);
    assert!(max > 64 || ratio > 0.04, "shift was not detected: {name}");
}

#[test]
#[allow(clippy::panic)] // Test assertions and diagnostic metrics.
fn all_cpu_snapshot_scenes_match_gpu() -> TestResult {
    let gpu = RenderContext::new()?;
    for name in [
        "rounded-rect",
        "linear-gradient",
        "clip",
        "opacity",
        "overlap",
        "reused-subtree",
    ] {
        let (scene, root) = scenes::scene(name)?;
        let cpu = CpuRenderer.render(&scene, root, SIZE)?;
        let image = gpu.render(&scene, root, SIZE)?;
        let (max, ratio) = metrics(cpu.pixels(), image.pixels());
        let raw = cpu
            .pixels()
            .iter()
            .zip(image.pixels())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        println!(
            "{name}: premultiplied max={max}, fraction(delta>4)={ratio:.6}, straight max={raw}"
        );
        // Compare premultiplied RGBA: straight RGB is unstable when alpha is
        // nearly zero (one alpha quantum can change RGB by 255). The 64/255
        // cap bounds CPU curve flattening, analytic SDF/polygon coverage, and
        // coverage-weighted compositing differences;
        // <=4% pixels may differ by >4/255. The four-level threshold discounts
        // quantization, while the ratio limits discrepancies to edge/gradient
        // pixels. Every scene is also checked against a one-pixel shift below.
        assert!(max <= 64 && ratio <= 0.04, "{name}: {max}, {ratio}");
        assert_shift_rejected(name, cpu.pixels(), image.pixels());
    }
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Test assertions and diagnostic metrics.
fn sheared_polygon_and_analytic_stroke_clip_match_cpu() -> TestResult {
    let gpu = RenderContext::new()?;
    for (name, transform) in [
        ("sheared", Affine::new([1.0, 0.0, 0.25, 1.0, 0.0, 0.0])),
        ("sdf-stroke-clip", Affine::IDENTITY),
    ] {
        let mut scene = Scene::new();
        let root = scene.insert(None, 0, NodeKind::Transform(transform))?;
        let clip = scene.insert(
            Some(root),
            0,
            NodeKind::Clip(Clip::RoundedRect(
                Rect::new(8.0, 8.0, 48.0, 56.0),
                CornerRadii::uniform(8.0),
            )),
        )?;
        let opacity = scene.insert(Some(clip), 0, NodeKind::Opacity(0.75))?;
        scene.insert(
            Some(opacity),
            0,
            NodeKind::Draw(Primitive::Stroke(
                Rect::new(10.0, 10.0, 46.0, 54.0),
                CornerRadii::uniform(10.0),
                5.0,
                Brush::Solid(scenes::red()),
            )),
        )?;
        let cpu = CpuRenderer.render(&scene, root, SIZE)?;
        let image = gpu.render(&scene, root, SIZE)?;
        let (max, ratio) = metrics(cpu.pixels(), image.pixels());
        let raw = cpu
            .pixels()
            .iter()
            .zip(image.pixels())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        println!(
            "{name}: premultiplied max={max}, fraction(delta>4)={ratio:.6}, straight max={raw}"
        );
        assert!(max <= 64 && ratio <= 0.04, "{name}: {max}, {ratio}");
        assert_shift_rejected(name, cpu.pixels(), image.pixels());
    }
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Exercise asymmetric radii, gradients and rotated uniform scale.
fn asymmetric_gradient_fill_and_stroke_match_cpu() -> TestResult {
    use oxiview_base::Point;
    use oxiview_scene::ColorStop;
    let gpu = RenderContext::new()?;
    for stroke in [false, true] {
        let mut scene = Scene::new();
        let root = scene.insert(
            None,
            0,
            NodeKind::Transform(Affine::new([0.9, 0.3, -0.3, 0.9, 12.0, 0.0])),
        )?;
        let brush = Brush::LinearGradient(
            Point::new(12.0, 12.0),
            Point::new(46.0, 46.0),
            vec![
                ColorStop {
                    position: 0.0,
                    color: scenes::red(),
                },
                ColorStop {
                    position: 1.0,
                    color: scenes::blue(),
                },
            ],
        );
        let rect = Rect::new(12.0, 12.0, 46.0, 46.0);
        let radii = CornerRadii::new(2.0, 6.0, 10.0, 14.0);
        let primitive = if stroke {
            Primitive::Stroke(rect, radii, 4.0, brush)
        } else {
            Primitive::Fill(rect, radii, brush)
        };
        scene.insert(Some(root), 0, NodeKind::Draw(primitive))?;
        let cpu = CpuRenderer.render(&scene, root, SIZE)?;
        let image = gpu.render(&scene, root, SIZE)?;
        let (max, ratio) = metrics(cpu.pixels(), image.pixels());
        let raw = cpu
            .pixels()
            .iter()
            .zip(image.pixels())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        println!(
            "asymmetric-gradient-{}: premultiplied max={max}, fraction(delta>4)={ratio:.6}, straight max={raw}",
            if stroke { "stroke" } else { "fill" }
        );
        assert!(
            max <= 64 && ratio <= 0.04,
            "stroke={stroke}: {max}, {ratio}"
        );
        assert_shift_rejected(
            if stroke {
                "asymmetric-gradient-stroke"
            } else {
                "asymmetric-gradient-fill"
            },
            cpu.pixels(),
            image.pixels(),
        );
    }
    Ok(())
}
