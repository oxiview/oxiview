//! Sparse UI rendering, including encoding, GPU completion and pixel readback.
#![allow(clippy::expect_used)] // Benchmark setup and failures are test-only.

use oxiview_base::{Affine, Color, CornerRadii, Rect};
use oxiview_render_wgpu::{PixelSize, RenderContext};
use oxiview_scene::{Brush, NodeKind, Primitive, Scene};

fn main() {
    divan::main();
}

#[divan::bench(args = [1, 128, 1000], sample_count = 20, sample_size = 1)]
fn sparse_ui(bencher: divan::Bencher, count: usize) {
    let context = RenderContext::new().expect("native GPU required");
    let mut scene = Scene::new();
    let root = scene
        .insert(None, 0, NodeKind::Transform(Affine::IDENTITY))
        .expect("root");
    for index in 0..count {
        let x = (index % 40) as f64 * 32.0 + 2.0;
        let y = (index / 40) as f64 * 32.0 + 2.0;
        scene
            .insert(
                Some(root),
                index,
                NodeKind::Draw(Primitive::Fill(
                    Rect::new(x, y, x + 24.0, y + 24.0),
                    CornerRadii::uniform(4.0),
                    Brush::Solid(Color::from_rgba8(45, 212, 191, 255)),
                )),
            )
            .expect("primitive");
    }
    let size = PixelSize {
        width: 1280,
        height: 840,
    };
    // Exclude device and pipeline warm-up from steady-state samples.
    context.render(&scene, root, size).expect("warm-up render");
    bencher.bench_local(|| {
        context
            .render(divan::black_box(&scene), root, size)
            .expect("GPU render and readback")
    });
}
