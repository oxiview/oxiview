//! Real Snake scene comparison across CPU and offscreen GPU, without a native window.

use oxiview_base::{PixelSize, RgbaImage};
use oxiview_core::Scheduler;
use oxiview_platform::{KeyCode, PhysicalSize, WindowEvent};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_render_wgpu::RenderContext;
use oxiview_scene::{EncodedNode, Primitive};
use snake::{
    application::{Config, Counters, SnakeApp},
    game::Phase,
};
use std::{error::Error, sync::Arc, time::Duration};

type TestResult = Result<(), Box<dyn Error>>;
const LOGICAL_WIDTH: u32 = 800;
const LOGICAL_HEIGHT: u32 = 600;

#[derive(Debug)]
struct Metrics {
    maximum: u8,
    differing_ratio: f64,
    compared: usize,
}

fn metrics(cpu: &[u8], gpu: &[u8], mask: Option<&[bool]>) -> Metrics {
    let mut maximum = 0;
    let mut differing = 0;
    let mut compared = 0;
    for (index, (a, b)) in cpu.chunks_exact(4).zip(gpu.chunks_exact(4)).enumerate() {
        if mask.is_some_and(|mask| !mask.get(index).copied().unwrap_or(false)) {
            continue;
        }
        let premultiply = |p: &[u8]| {
            [
                ((u16::from(p[0]) * u16::from(p[3]) + 127) / 255) as u8,
                ((u16::from(p[1]) * u16::from(p[3]) + 127) / 255) as u8,
                ((u16::from(p[2]) * u16::from(p[3]) + 127) / 255) as u8,
                p[3],
            ]
        };
        let difference = premultiply(a)
            .into_iter()
            .zip(premultiply(b))
            .map(|(a, b)| a.abs_diff(b))
            .max()
            .unwrap_or(0);
        maximum = maximum.max(difference);
        differing += usize::from(difference > 4);
        compared += 1;
    }
    Metrics {
        maximum,
        differing_ratio: differing as f64 / compared.max(1) as f64,
        compared,
    }
}

fn within_existing_bounds(metrics: &Metrics) -> bool {
    // Same premultiplied bounds as oxiview-app/tests/renderers.rs: curve
    // coverage may differ by at most 64/255; <=4% of compared pixels may
    // exceed 4/255. The separate glyph-region check prevents a large quiet
    // background from hiding text differences.
    metrics.maximum <= 64 && metrics.differing_ratio <= 0.04
}

fn glyph_regions(nodes: &[EncodedNode], size: PixelSize) -> (Vec<bool>, usize) {
    let width = size.width as usize;
    let mut mask = vec![false; width * size.height as usize];
    let mut glyphs = 0;
    for node in nodes {
        let EncodedNode::Draw {
            primitive: Primitive::Image(rect, image),
            transform,
            ..
        } = node
        else {
            continue;
        };
        assert!(
            image.pixels().chunks_exact(4).any(|pixel| pixel[3] != 0),
            "empty glyph draw"
        );
        glyphs += 1;
        let [left, top, right, bottom] = rect.edges();
        let [a, b, c, d, e, f] = transform.coefficients();
        let corners = [(left, top), (right, top), (right, bottom), (left, bottom)]
            .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
        let min_x = corners
            .iter()
            .map(|(x, _)| *x)
            .fold(f64::INFINITY, f64::min)
            .floor()
            .clamp(0.0, f64::from(size.width)) as usize;
        let max_x = corners
            .iter()
            .map(|(x, _)| *x)
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil()
            .clamp(0.0, f64::from(size.width)) as usize;
        let min_y = corners
            .iter()
            .map(|(_, y)| *y)
            .fold(f64::INFINITY, f64::min)
            .floor()
            .clamp(0.0, f64::from(size.height)) as usize;
        let max_y = corners
            .iter()
            .map(|(_, y)| *y)
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil()
            .clamp(0.0, f64::from(size.height)) as usize;
        for y in min_y..max_y {
            for x in min_x..max_x {
                mask[y * width + x] = true;
            }
        }
    }
    (mask, glyphs)
}

fn shifted_one_pixel(image: &RgbaImage) -> Vec<u8> {
    let mut output = vec![0; image.pixels().len()];
    let stride = image.width() as usize * 4;
    for (out, row) in output
        .chunks_exact_mut(stride)
        .zip(image.pixels().chunks_exact(stride))
    {
        out[4..].copy_from_slice(&row[..stride - 4]);
    }
    output
}

#[test]
fn actual_ready_and_playing_scenes_match_cpu_and_gpu_at_both_scales() -> TestResult {
    // Device creation failure is a real test failure; this test never silently
    // skips GPU coverage and never requests a native surface or visible window.
    let gpu = RenderContext::new()?;
    for debug_overlay in [false, true] {
        let config = Config {
            debug_overlay,
            ..Config::default()
        };
        let mut app = SnakeApp::new(config, Arc::new(Counters::default()))?;
        let mut scheduler = Scheduler::default();
        for playing in [false, true] {
            if playing {
                app.process(
                    &WindowEvent::Key {
                        code: KeyCode::Enter,
                        pressed: true,
                        repeat: false,
                        timestamp: Duration::from_millis(1),
                    },
                    &mut scheduler,
                )?;
                assert_eq!(app.snapshot().phase, Phase::Running);
            } else {
                assert_eq!(app.snapshot().phase, Phase::Ready);
            }
            for scale in [1u32, 2] {
                let size = PixelSize {
                    width: LOGICAL_WIDTH * scale,
                    height: LOGICAL_HEIGHT * scale,
                };
                let frame = app.render(
                    PhysicalSize {
                        width: size.width,
                        height: size.height,
                    },
                    f64::from(scale),
                )?;
                let encoded = frame.scene.encode(frame.root)?;
                let (text_mask, glyphs) = glyph_regions(&encoded.nodes, size);
                assert!(glyphs > 80, "actual scene lost its real text");
                assert!(
                    encoded
                        .nodes
                        .iter()
                        .any(|node| matches!(node, EncodedNode::BeginOpacity(_))),
                    "scene must exercise group opacity"
                );
                assert!(
                    encoded.nodes.iter().any(
                        |node| matches!(node,EncodedNode::Draw { clips,.. } if !clips.is_empty())
                    ),
                    "scene must exercise clips"
                );
                let cpu = CpuRenderer.render(&frame.scene, frame.root, size)?;
                let gpu_image = gpu.render(&frame.scene, frame.root, size)?;
                assert_eq!(
                    (cpu.width(), cpu.height()),
                    (gpu_image.width(), gpu_image.height())
                );
                let whole = metrics(cpu.pixels(), gpu_image.pixels(), None);
                let text = metrics(cpu.pixels(), gpu_image.pixels(), Some(&text_mask));
                let state = if playing { "playing" } else { "ready" };
                println!(
                    "{state}@{scale}x debug={debug_overlay} glyphs={glyphs}, scene={whole:?}, glyph-regions={text:?}"
                );
                assert!(
                    text.compared > 1000,
                    "glyph region must contain actual rendered text"
                );
                assert!(
                    within_existing_bounds(&whole),
                    "{state}@{scale}x whole scene: {whole:?}"
                );
                assert!(
                    within_existing_bounds(&text),
                    "{state}@{scale}x glyph regions: {text:?}"
                );
                let shifted = shifted_one_pixel(&gpu_image);
                let displaced_text = metrics(cpu.pixels(), &shifted, Some(&text_mask));
                assert!(
                    !within_existing_bounds(&displaced_text),
                    "one-pixel text displacement escaped comparison: {displaced_text:?}"
                );
            }
        }
    }
    Ok(())
}
