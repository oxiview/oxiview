#![allow(clippy::unwrap_used)]

use oxiview_base::{Affine, PixelSize, Point, Rect};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_scene::{Clip, NodeKind, Primitive, Scene};
use oxiview_text::TextSystem;
use std::path::{Path, PathBuf};

use super::{
    BlockKind, CompileStats, Compiler, Graphic, GraphicContent, InlineKind, PreviewDocument,
};

const SOURCE: &str = include_str!("../../fixtures/acceptance.md");

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn text() -> TextSystem {
    let mut text = TextSystem::new();
    text.register_font(
        include_bytes!("../../../../crates/oxiview-text/tests/fonts/NotoSans.ttf").to_vec(),
    )
    .unwrap();
    text.register_font(
        include_bytes!("../../../../crates/oxiview-text/tests/fonts/NotoSansArabic.ttf").to_vec(),
    )
    .unwrap();
    text
}

fn graphics(document: &PreviewDocument) -> impl Iterator<Item = &Graphic> {
    document
        .blocks
        .iter()
        .filter_map(|block| match &block.kind {
            BlockKind::Graphic { graphic, .. } => Some(graphic.as_ref()),
            _ => None,
        })
}

#[test]
fn acceptance_source_compiles_into_native_preview_blocks() {
    fn assert_worker_message<T: Send + Sync>() {}
    assert_worker_message::<PreviewDocument>();

    let mut compiler = Compiler::default();
    let base = fixtures();
    let name = base.join("acceptance.md");
    let document = compiler.compile_named(SOURCE, &base, Some(&name));
    assert!(
        document.diagnostics().is_empty(),
        "{:?}",
        document.diagnostics()
    );
    let first = document.block_ranges().next().unwrap();
    assert_eq!(&SOURCE[first], "# 山间书屋\n");
    assert_eq!(
        document.links()[0].destination,
        "https://spec.commonmark.org/0.31.2/"
    );
    assert!(SOURCE[document.links()[0].range.clone()].starts_with("[CommonMark reference]"));

    let mut found_style_run = false;
    let mut found_tasks = false;
    let mut found_table = false;
    let mut found_image = false;
    let mut found_literal_code = false;
    for block in document.blocks.iter() {
        match &block.kind {
            BlockKind::Paragraph(runs) => {
                found_style_run |= runs.iter().any(|run| run.marks.strong)
                    && runs.iter().any(|run| run.marks.italic);
                for run in runs {
                    if let InlineKind::Graphic {
                        graphic,
                        math: false,
                    } = &run.kind
                        && let GraphicContent::Raster(image) = &graphic.content
                    {
                        assert_eq!((image.width(), image.height()), (144, 72));
                        assert_eq!(graphic.size.width(), 144.0);
                        found_image = true;
                    }
                }
            }
            BlockKind::List(None, items) => {
                assert_eq!(items[0].task, Some(true));
                assert_eq!(items[1].task, Some(false));
                assert!(matches!(&items[1].blocks[1].kind, BlockKind::List(None, _)));
                found_tasks = true;
            }
            BlockKind::Table(table) => {
                assert_eq!(table.alignments.len(), 3);
                assert_eq!(table.rows.len(), 3);
                assert_eq!(table.rows[0].len(), 3);
                found_table = true;
            }
            BlockKind::Code(code) => found_literal_code = code.contains(r#""$not_math$""#),
            _ => {}
        }
    }
    assert!(found_style_run && found_tasks && found_table && found_image && found_literal_code);
    assert_eq!(graphics(&document).count(), 5);
    assert_eq!(
        compiler.stats(),
        CompileStats {
            parses: 1,
            diagram_renders: 3,
            math_renders: 4,
            cache_hits: 0
        }
    );
}

#[test]
fn preview_glyph_images_align_to_device_pixels_at_a_fractional_origin()
-> Result<(), Box<dyn std::error::Error>> {
    let document = Compiler::default().compile("An open book", &fixtures());
    let mut text = text();
    let viewport = Rect::new(0.0, 0.0, 400.0, 100.0);
    let layout = document.layout(&mut text, viewport.size().width())?;
    let origin = Point::new(425.5, 91.25);
    for scale in [1.0, 1.5, 2.0] {
        let draws = layout.primitives_in_scene(&mut text, viewport, 0.0, scale, origin)?;
        assert!(!draws.is_empty());
        for draw in draws {
            let Primitive::Image(rect, _) = draw else {
                return Err("Expected a raster glyph for preview body text".into());
            };
            let [left, top, right, bottom] = rect.edges();
            for edge in [
                left + origin.x(),
                top + origin.y(),
                right + origin.x(),
                bottom + origin.y(),
            ] {
                let pixel = edge * scale;
                assert!((pixel - pixel.round()).abs() < 1e-6);
            }
        }
    }
    Ok(())
}

#[test]
fn fixture_paints_through_cpu_scene_at_two_display_scales() {
    let document = Compiler::default().compile(SOURCE, &fixtures());
    let mut text = text();
    let width = 600.0;
    let layout = document.layout(&mut text, width).unwrap();
    let height = layout.height().ceil();
    let viewport = Rect::new(8.0, 12.0, 8.0 + width, 12.0 + height);
    assert!(height > 800.0);
    let mut counts = Vec::new();
    for scale in [1.0, 2.0] {
        let primitives = layout.primitives(&mut text, viewport, 0.0, scale).unwrap();
        assert_eq!(
            primitives
                .iter()
                .filter(|draw| matches!(draw, Primitive::Image(_, image) if image.width() > 70 && image.height() > 50))
                .count(),
            4,
        );
        assert!(
            primitives
                .iter()
                .filter(|draw| matches!(draw, Primitive::Glyph(glyph)
            if super::math::is_fixture_font(&glyph.font)))
                .count()
                >= 13
        );
        let mut scene = Scene::new();
        let root = scene
            .insert(
                None,
                0,
                NodeKind::Transform(Affine::new([scale, 0.0, 0.0, scale, 0.0, 0.0])),
            )
            .unwrap();
        let clip = scene
            .insert(Some(root), 0, NodeKind::Clip(Clip::Rect(viewport)))
            .unwrap();
        for (index, primitive) in primitives.into_iter().enumerate() {
            scene
                .insert(Some(clip), index, NodeKind::Draw(primitive))
                .unwrap();
        }
        let pixels = CpuRenderer
            .render(
                &scene,
                root,
                PixelSize {
                    width: ((width + 16.0) * scale) as u32,
                    height: ((height + 24.0) * scale) as u32,
                },
            )
            .unwrap();
        assert_eq!(&pixels.pixels()[..4], &[0, 0, 0, 0]);
        let painted = pixels
            .pixels()
            .chunks_exact(4)
            .filter(|pixel| pixel[3] > 32)
            .count();
        assert!(painted > 30_000);
        counts.push(painted);
    }
    assert!((3 * counts[0]..5 * counts[0]).contains(&counts[1]));

    let small = Rect::new(8.0, 12.0, 608.0, 122.0);
    let at_top = layout.primitives(&mut text, small, 0.0, 1.0).unwrap();
    let at_bottom = layout.primitives(&mut text, small, height, 1.0).unwrap();
    assert!(!at_top.is_empty());
    assert!(!at_bottom.is_empty());
    assert_ne!(at_top, at_bottom);
    assert!(
        at_top.iter().all(
            |primitive| !matches!(primitive, Primitive::Image(_, image) if image.width() > 70)
        )
    );
}

#[test]
fn inline_tex_uses_the_native_text_baseline() {
    let mut text = text().with_vector_threshold(0.0).unwrap();
    let document =
        Compiler::default().compile("Rates $x_{i}$ and $y^{2}$ stay aligned.", Path::new("."));
    let layout = document.layout(&mut text, 500.0).unwrap();
    let primitives = layout
        .primitives(&mut text, Rect::new(0.0, 0.0, 500.0, 100.0), 0.0, 1.0)
        .unwrap();
    let mut text_baseline = None;
    let mut math_baselines = Vec::new();
    for primitive in primitives {
        if let Primitive::Glyph(glyph) = primitive {
            if super::math::is_fixture_font(&glyph.font) {
                math_baselines.push((glyph.size, glyph.origin.y()));
            } else {
                text_baseline = Some(glyph.origin.y());
            }
        }
    }
    let baseline = text_baseline.unwrap();
    assert_eq!(math_baselines.len(), 4);
    assert!(
        math_baselines
            .iter()
            .filter(|(size, _)| *size == 16.0)
            .all(|(_, y)| (y - baseline).abs() < 0.01)
    );
    assert!(
        math_baselines
            .iter()
            .any(|(size, y)| *size < 16.0 && *y > baseline)
    );
    assert!(
        math_baselines
            .iter()
            .any(|(size, y)| *size < 16.0 && *y < baseline)
    );
}

#[test]
fn graphics_cache_reuses_errors_and_rebuilds_only_changed_source() {
    let mut compiler = Compiler::default();
    let fixtures = fixtures();
    let first = compiler.compile(SOURCE, &fixtures);
    let updated = SOURCE
        .replace("-->|read|", "-->|read more|")
        .replace(r"\frac{1}{2}", r"\frac{1}{5}");
    let second = compiler.compile(&updated, &fixtures);
    assert!(
        second.diagnostics().is_empty(),
        "{:?}",
        second.diagnostics()
    );
    assert_eq!(
        compiler.stats(),
        CompileStats {
            parses: 2,
            diagram_renders: 4,
            math_renders: 5,
            cache_hits: 5
        }
    );
    let first = graphics(&first).collect::<Vec<_>>();
    let second = graphics(&second).collect::<Vec<_>>();
    let (GraphicContent::Raster(old_flow), GraphicContent::Raster(new_flow)) =
        (&first[0].content, &second[0].content)
    else {
        unreachable!()
    };
    assert_ne!(old_flow, new_flow);
    assert!(std::ptr::eq(first[1], second[1]));
    assert!(std::ptr::eq(first[2], second[2]));
    let (GraphicContent::Math(old_formula), GraphicContent::Math(new_formula)) =
        (&first[3].content, &second[3].content)
    else {
        unreachable!()
    };
    assert_ne!(old_formula, new_formula);
    assert!(std::ptr::eq(first[4], second[4]));

    let invalid = "> ```mermaid\r\n> flowchart LR\r\n>   --> Input\r\n> ```\r\n\r\nA bad $\\frac{1}{$ and more text.\r\n\r\nAfter\r\n";
    let named = Path::new("/drafts/trail.md");
    let broken = compiler.compile_named(invalid, &fixtures, Some(named));
    let errors = broken.diagnostics();
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].file.as_deref(), Some(named));
    assert_eq!((errors[0].line, errors[0].column), (3, 5));
    assert_eq!(&invalid[errors[0].range.clone()], "-");
    assert_eq!((errors[1].line, errors[1].column), (6, 7));
    assert_eq!(&invalid[errors[1].range.clone()], r"$\frac{1}{$");
    let shifted = format!("前言\r\n\r\n{invalid}");
    let again = compiler.compile_named(&shifted, &fixtures, Some(Path::new("renamed.md")));
    assert_eq!(again.diagnostics()[0].line, 5);
    assert_eq!(
        again.diagnostics()[0].file.as_deref(),
        Some(Path::new("renamed.md"))
    );
    assert_eq!(compiler.stats().cache_hits, 7);
    let mut text = text();
    let layout = again.layout(&mut text, 450.0).unwrap();
    let painted = layout
        .primitives(&mut text, Rect::new(0.0, 0.0, 450.0, 500.0), 0.0, 1.0)
        .unwrap();
    assert!(painted.len() > 50);
    let corrected = invalid
        .replace("--> Input", "Start --> Input")
        .replace(r"$\frac{1}{$", r"$\frac{1}{3}$");
    assert!(
        compiler
            .compile(&corrected, &fixtures)
            .diagnostics()
            .is_empty()
    );
}

#[test]
fn local_images_resolve_relative_paths_and_fall_back_to_alt_text() {
    let source = r"![Local](h%69ll.png) ![**No image**](missing.png) ![Remote](https://example.com/remote.png)";
    let document = Compiler::default().compile(source, &fixtures());
    let BlockKind::Paragraph(paragraph) = &document.blocks[0].kind else {
        unreachable!()
    };
    assert!(matches!(
        paragraph[0].kind,
        InlineKind::Graphic { math: false, .. }
    ));
    assert!(matches!(&paragraph[2].kind, InlineKind::Text(text) if text == "No image"));
    assert!(matches!(&paragraph[4].kind, InlineKind::Text(text) if text == "Remote"));
}

#[test]
fn an_unfinished_display_formula_gets_a_source_diagnostic() {
    let source = "Intro\n\n$$\n\\frac{1}{\n$$\n\n`$\\frac{1}{$` stays literal.\n\nAfter";
    let mut compiler = Compiler::default();
    let document = compiler.compile(source, &fixtures());
    assert_eq!(document.diagnostics().len(), 1);
    let error = &document.diagnostics()[0];
    assert_eq!((error.line, error.column), (3, 1));
    assert_eq!(&source[error.range.clone()], "$$\n\\frac{1}{\n$$");
    assert!(matches!(&document.blocks[1].kind, BlockKind::Diagnostic(0)));
    assert!(matches!(&document.blocks[3].kind, BlockKind::Paragraph(_)));
}

#[test]
fn mixed_text_with_inline_math_wraps_at_a_narrow_viewport() {
    let source = "كتبت معادلة $x_{2}$ هنا مع كلمات باللغة العربية. The next formula is $y^2$ and the document continues.";
    let document = Compiler::default().compile(source, &fixtures());
    let mut text = text().with_vector_threshold(0.0).unwrap();
    let narrow = document.layout(&mut text, 210.0).unwrap();
    let wide = document.layout(&mut text, 760.0).unwrap();
    assert!(narrow.height() > wide.height() + 30.0);
    let draws = narrow
        .primitives(&mut text, Rect::new(0.0, 0.0, 210.0, 220.0), 0.0, 1.0)
        .unwrap();
    let math = draws.iter().filter_map(|primitive| match primitive {
        Primitive::Glyph(glyph) if super::math::is_fixture_font(&glyph.font) => Some(glyph),
        _ => None,
    });
    let baselines = math
        .filter(|glyph| glyph.size == 16.0)
        .map(|glyph| glyph.origin)
        .collect::<Vec<_>>();
    assert_eq!(baselines.len(), 2);
    assert!(baselines[0].y() < baselines[1].y());
    assert!(
        baselines
            .iter()
            .all(|point| (0.0..210.0).contains(&point.x()))
    );
}

#[test]
fn class_er_and_pie_blocks_use_the_same_native_diagram_path() {
    let documents = [
        "classDiagram\n  Book --> Page",
        "erDiagram\n  AUTHOR ||--o{ BOOK : writes",
        "pie title Work\n  \"Read\" : 3\n  \"Write\" : 2",
    ];
    let mut compiler = Compiler::default();
    for source in documents {
        let md = format!("```mermaid\n{source}\n```\n");
        let preview = compiler.compile(&md, std::path::Path::new("."));
        assert!(
            preview.diagnostics().is_empty(),
            "{:?}",
            preview.diagnostics()
        );
    }
    assert_eq!(compiler.stats().diagram_renders, 3);
}
