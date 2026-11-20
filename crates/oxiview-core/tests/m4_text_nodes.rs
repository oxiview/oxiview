//! Real text-node shaping, reactive updates, DPI rasterization, and failure recovery.
// Assertions and fixture unwraps identify the exact failing integration contract.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use oxiview_base::{Color, Point, Rect, RgbaImage, Size};
use oxiview_core::{PaintedScene, UiTree, node, show, text};
use oxiview_reactive::{Scope, signal};
use oxiview_scene::{EncodedNode, NodeKind, Primitive};
use oxiview_style::{Length, Position, Style};
use oxiview_text::{TextAlign, TextStyle, TextSystem};
use std::{cell::RefCell, rc::Rc};

fn viewport() -> Size {
    Size::new(500.0, 300.0)
}
fn image_pixels(painted: &PaintedScene) -> usize {
    let mut pixels = 0;
    let mut stack = vec![painted.root];
    while let Some(id) = stack.pop() {
        if let NodeKind::Draw(Primitive::Image(_, image)) = painted.scene.content(id).unwrap() {
            pixels += image.pixels().len() / 4;
        }
        stack.extend(painted.scene.children(Some(id)).unwrap());
    }
    pixels
}

#[test]
fn reactive_text_reuses_node_identity_and_rasterizes_at_actual_scale() {
    let scope = Scope::new().unwrap();
    let source = scope.run(|| signal(String::from("Score 42"))).unwrap();
    let mut fonts = TextSystem::without_system_fonts();
    fonts
        .register_font(include_bytes!("../../oxiview-text/tests/fonts/NotoSans.ttf").to_vec())
        .unwrap();
    let system = Rc::new(RefCell::new(fonts));
    let mut style = Style::default();
    style.layout.position = Position::Absolute;
    style.layout.width = Length::Points(150.0);
    let font = TextStyle {
        font_size: 20.0,
        family: Some("Noto Sans".into()),
        ..TextStyle::default()
    };
    let mut tree = UiTree::new().unwrap();
    let id = tree
        .mount(
            tree.root(),
            text(Rc::clone(&system), "", font)
                .style(style)
                .bind_text(move || source.get()),
        )
        .unwrap();
    let first = tree.paint(viewport(), 1.0).unwrap();
    let first_size = tree.text_size(id).unwrap();
    assert!(first_size.height() > 0.0);
    assert!(image_pixels(&first) > 0);
    let cache = system.borrow().cache_stats();
    let before = tree.diagnostics();
    let mut root_style = tree.style(tree.root()).unwrap().clone();
    root_style.background = Color::from_rgba8(245, 245, 245, 255);
    tree.set_style(tree.root(), root_style).unwrap();
    tree.paint(viewport(), 1.0).unwrap();
    assert_eq!(system.borrow().cache_stats(), cache);
    let recolored = tree.diagnostics();
    assert_eq!(recolored.scene_updates, before.scene_updates + 1);
    assert_eq!(recolored.scene_builds, before.scene_builds);
    assert_eq!(recolored.painted_nodes, before.painted_nodes);
    let doubled = tree.paint(viewport(), 2.0).unwrap();
    let scaled = tree.diagnostics();
    assert_eq!(scaled.scene_builds, recolored.scene_builds + 1);
    assert_eq!(scaled.painted_nodes, recolored.painted_nodes + 1);
    assert_eq!(tree.text_size(id).unwrap(), first_size);
    assert!(image_pixels(&doubled) > image_pixels(&first));
    let before = tree.diagnostics();
    source
        .set("Score 42. This is a much longer wrapped message with real glyphs.".into())
        .unwrap();
    tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    assert!(tree.contains(id));
    assert_eq!(tree.text_source(id).unwrap(), source.get().unwrap());
    assert!(tree.text_size(id).unwrap().height() > first_size.height());
    let after = tree.diagnostics();
    assert_eq!(after.layout_input_visits - before.layout_input_visits, 1);
    assert_eq!(after.resolved_styles, before.resolved_styles);
    assert!(image_pixels(&tree.paint(viewport(), 1.5).unwrap()) > 0);
    tree.remove(id).unwrap();
    source.set("Detached".into()).unwrap();
    assert!(
        tree.flush_updates(viewport(), |_, _, _| {})
            .unwrap()
            .reports
            .is_empty()
    );
    let system_ref = Rc::downgrade(&system);
    drop(system);
    assert!(system_ref.upgrade().is_none());
    assert!(image_pixels(&first) > 0);
}

#[test]
fn unavailable_font_fails_mount_without_leaving_a_partial_subtree() {
    let mut tree = UiTree::new().unwrap();
    let existing = tree.mount(tree.root(), node()).unwrap();
    let fonts = Rc::new(RefCell::new(TextSystem::without_system_fonts()));
    let result = tree.mount(
        tree.root(),
        text(fonts, "No installed font", TextStyle::default()),
    );
    assert!(result.is_err());
    assert_eq!(tree.children(tree.root()).unwrap(), &[existing]);
    tree.paint(viewport(), 1.0).unwrap();
}

#[test]
fn hiding_and_updating_text_in_one_batch_cancels_disposed_bindings() {
    let scope = Scope::new().unwrap();
    let visible = scope.run(|| signal(true)).unwrap();
    let source = scope.run(|| signal(String::from("Ready"))).unwrap();
    let mut fonts = TextSystem::without_system_fonts();
    fonts
        .register_font(include_bytes!("../../oxiview-text/tests/fonts/NotoSans.ttf").to_vec())
        .unwrap();
    let system = Rc::new(RefCell::new(fonts));
    let mut tree = UiTree::new().unwrap();
    let before = oxiview_reactive::runtime_stats().unwrap();
    let container = tree
        .mount(
            tree.root(),
            show(visible, move || {
                Ok(text(
                    Rc::clone(&system),
                    "",
                    TextStyle {
                        family: Some("Noto Sans".into()),
                        ..TextStyle::default()
                    },
                )
                .bind_text(move || source.get()))
            }),
        )
        .unwrap();
    let child = tree.children(container).unwrap()[0];
    oxiview_reactive::batch(|| {
        visible.set(false)?;
        source.set("Running".into())?;
        Ok(())
    })
    .unwrap();
    let result = tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    assert!(
        result
            .reports
            .iter()
            .flat_map(|report| &report.outcomes)
            .all(|outcome| outcome.result.is_ok())
    );
    assert!(
        result
            .reports
            .iter()
            .map(|report| report.canceled_bindings)
            .sum::<usize>()
            > 0
    );
    assert!(!tree.contains(child));
    assert_eq!(image_pixels(&tree.paint(viewport(), 1.0).unwrap()), 0);
    tree.remove(container).unwrap();
    assert_eq!(oxiview_reactive::runtime_stats().unwrap(), before);
}

fn device_glyphs(painted: &PaintedScene) -> Vec<(Rect, RgbaImage)> {
    painted
        .scene
        .encode(painted.root)
        .unwrap()
        .nodes
        .into_iter()
        .filter_map(|node| {
            if let EncodedNode::Draw {
                primitive: Primitive::Image(rect, image),
                transform,
                ..
            } = node
            {
                let [left, top, right, bottom] = rect.edges();
                let first = transform.transform_point(Point::new(left, top));
                let last = transform.transform_point(Point::new(right, bottom));
                Some((Rect::new(first.x(), first.y(), last.x(), last.y()), image))
            } else {
                None
            }
        })
        .collect()
}
fn assert_pixel_aligned(glyphs: &[(Rect, RgbaImage)]) {
    assert!(!glyphs.is_empty());
    for (rect, image) in glyphs {
        for edge in rect.edges() {
            assert!(
                (edge - edge.round()).abs() < 1e-8,
                "device glyph edge {edge} is fractional"
            );
        }
        assert!((rect.size().width() - f64::from(image.width())).abs() < 1e-8);
        assert!((rect.size().height() - f64::from(image.height())).abs() < 1e-8);
    }
}

#[test]
fn fractional_world_origins_select_glyph_phase_without_double_translation() {
    for scale in [1.0, 1.5, 2.0] {
        for direction in [1.0, -1.0] {
            let mut fonts = TextSystem::without_system_fonts();
            fonts
                .register_font(
                    include_bytes!("../../oxiview-text/tests/fonts/NotoSans.ttf").to_vec(),
                )
                .unwrap();
            let system = Rc::new(RefCell::new(fonts));
            let font = TextStyle {
                font_size: 19.5,
                family: Some("Noto Sans".into()),
                align: TextAlign::Center,
                ..TextStyle::default()
            };
            let mut parent_style = Style::default();
            parent_style.layout.position = Position::Absolute;
            parent_style.layout.width = Length::Points(300.0);
            parent_style.layout.height = Length::Points(150.0);
            parent_style.layout.inset.left = Length::Points(17.125 * direction);
            parent_style.layout.inset.top = Length::Points(11.375 * direction);
            let mut label_style = Style::default();
            label_style.layout.position = Position::Absolute;
            label_style.layout.width = Length::Points(200.0);
            label_style.layout.inset.left = Length::Points(5.25);
            label_style.layout.inset.top = Length::Points(3.125);
            label_style.layout.border_width = 0.25;
            label_style.layout.padding.left = Length::Points(1.125);
            label_style.layout.padding.right = Length::Points(1.75);
            label_style.layout.padding.top = Length::Points(2.375);
            let mut tree = UiTree::new().unwrap();
            let parent = tree.append(tree.root(), parent_style.clone()).unwrap();
            let label = tree
                .mount(
                    parent,
                    text(Rc::clone(&system), "Phase", font.clone()).style(label_style.clone()),
                )
                .unwrap();
            let first = device_glyphs(&tree.paint(viewport(), scale).unwrap());
            assert_pixel_aligned(&first);
            let bounds = tree.bounds(label).unwrap();
            let logical_size = bounds.size();
            let content_origin =
                Point::new(bounds.origin().x() + 1.375, bounds.origin().y() + 2.625);
            let content_width = bounds.size().width() - 0.5 - 1.125 - 1.75;
            let paragraph = system
                .borrow_mut()
                .layout("Phase", &font, Some(content_width))
                .unwrap();
            let expected = system
                .borrow_mut()
                .glyph_images(&paragraph, content_origin, scale)
                .unwrap();
            assert_eq!(first.len(), expected.len());
            for ((actual, image), expected) in first.iter().zip(expected) {
                for (actual, expected) in actual.edges().into_iter().zip(expected.rect.edges()) {
                    assert!((actual - expected * scale).abs() < 1e-8);
                }
                assert_eq!(image, &expected.image);
            }
            let cached = system.borrow().cache_stats();
            // Whole logical translations are exact in Taffy's coordinates
            // and produce whole device-pixel translations at every test scale.
            parent_style.layout.inset.left = Length::Points(17.125 * direction + 2.0);
            parent_style.layout.inset.top = Length::Points(11.375 * direction + 4.0);
            tree.set_style(parent, parent_style).unwrap();
            let before = tree.diagnostics();
            let translated = device_glyphs(&tree.paint(viewport(), scale).unwrap());
            assert_eq!(tree.diagnostics().painted_nodes - before.painted_nodes, 2);
            assert_pixel_aligned(&translated);
            assert_eq!(tree.bounds(label).unwrap().size(), logical_size);
            assert_eq!(
                system.borrow().cache_stats(),
                cached,
                "whole device-pixel translation should reuse the node raster cache"
            );
            for ((first, old_image), (moved, image)) in first.iter().zip(&translated) {
                let expected = [2.0 * scale, 4.0 * scale, 2.0 * scale, 4.0 * scale];
                for ((old, new), delta) in
                    first.edges().into_iter().zip(moved.edges()).zip(expected)
                {
                    assert!((new - old - delta).abs() < 1e-8);
                }
                assert_eq!(old_image.pixels().as_ptr(), image.pixels().as_ptr());
            }
            let mut parent_style = tree.style(parent).unwrap().clone();
            parent_style.layout.inset.left = Length::Points(17.375 * direction + 2.0);
            tree.set_style(parent, parent_style).unwrap();
            let fractional = device_glyphs(&tree.paint(viewport(), scale).unwrap());
            assert_pixel_aligned(&fractional);
            assert_eq!(tree.bounds(label).unwrap().size(), logical_size);
            assert!(
                system.borrow().cache_stats().misses > cached.misses,
                "fractional movement must select the new glyph phase"
            );
        }
    }
}

#[test]
fn tree_switches_glyphs_at_final_display_size_without_moving_the_baseline() {
    let mut fonts = TextSystem::without_system_fonts()
        .with_vector_threshold(30.0)
        .unwrap();
    fonts
        .register_font(
            include_bytes!("../../oxiview-text/tests/fonts/OxiviewColorFixture.ttf").to_vec(),
        )
        .unwrap();
    let system = Rc::new(RefCell::new(fonts));
    let mut style = Style::default();
    style.layout.position = Position::Absolute;
    style.layout.inset.left = Length::Points(11.5);
    style.layout.inset.top = Length::Points(9.25);
    style.layout.padding.left = Length::Points(3.0);
    let mut tree = UiTree::new().unwrap();
    let id = tree
        .mount(
            tree.root(),
            text(
                Rc::clone(&system),
                "O",
                TextStyle {
                    font_size: 20.0,
                    family: Some("Oxiview Color Fixture".into()),
                    ..TextStyle::default()
                },
            )
            .style(style),
        )
        .unwrap();
    let collect_outline = |scene: &PaintedScene| {
        scene
            .scene
            .encode(scene.root)
            .unwrap()
            .nodes
            .into_iter()
            .filter_map(|node| match node {
                EncodedNode::Draw {
                    primitive: Primitive::Glyph(glyph),
                    transform,
                    ..
                } => Some((transform.transform_point(glyph.origin), glyph)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    let small = tree.paint(viewport(), 1.5).unwrap();
    let logical_size = tree.text_size(id).unwrap();
    assert_eq!(device_glyphs(&small).len(), 1);
    assert!(collect_outline(&small).is_empty());

    let (first, glyph) = collect_outline(&tree.paint(viewport(), 2.0).unwrap())
        .pop()
        .unwrap();
    let (second, _) = collect_outline(&tree.paint(viewport(), 3.0).unwrap())
        .pop()
        .unwrap();
    assert_eq!(logical_size, tree.text_size(id).unwrap());
    assert_eq!(glyph.size, 20.0);
    assert_eq!(glyph.kind, oxiview_base::GlyphKind::Outline);
    assert_eq!(first.x(), (11.5 + 3.0) * 2.0);
    assert_eq!(second.x() / 3.0, first.x() / 2.0);
    assert_eq!(second.y() / 3.0, first.y() / 2.0);
    assert_eq!(
        device_glyphs(&tree.paint(viewport(), 1.5).unwrap()).len(),
        1
    );
}

fn fixture_system() -> Rc<RefCell<TextSystem>> {
    let mut system = TextSystem::without_system_fonts();
    system
        .register_font(include_bytes!("../../oxiview-text/tests/fonts/NotoSans.ttf").to_vec())
        .unwrap();
    Rc::new(RefCell::new(system))
}

fn font(size: f64) -> TextStyle {
    TextStyle {
        font_size: size,
        family: Some("Noto Sans".into()),
        ..TextStyle::default()
    }
}

fn paint_has_red(painted: &PaintedScene) -> bool {
    device_glyphs(painted).iter().any(|(_, glyph)| {
        glyph
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel[..3] == [212, 28, 33] && pixel[3] != 0)
    })
}

#[test]
fn rich_reactive_updates_invalidate_baseline_and_paint_and_skip_equal_values() {
    use oxiview_core::{RichText, TextSpan, TextSpanStyle, rich_text};
    use oxiview_style::Alignment;

    let scope = Scope::new().unwrap();
    let plain = RichText::plain("Café");
    let source = scope.run(|| signal(plain.clone())).unwrap();
    let system = fixture_system();
    let mut tree = UiTree::new().unwrap();
    let mut root = tree.style(tree.root()).unwrap().clone();
    root.layout.align_items = Alignment::Baseline;
    tree.set_style(tree.root(), root).unwrap();
    let rich = tree
        .mount(
            tree.root(),
            rich_text(Rc::clone(&system), plain.clone(), font(16.0))
                .bind_rich_text(move || source.get()),
        )
        .unwrap();
    let other = tree
        .mount(tree.root(), text(Rc::clone(&system), "Label", font(16.0)))
        .unwrap();
    tree.paint(viewport(), 1.0).unwrap();
    let initial_bounds = tree.bounds(rich).unwrap();
    let initial_peer_y = tree.bounds(other).unwrap().origin().y();
    let initial_diagnostics = tree.diagnostics();
    let initial_cache = system.borrow().cache_stats();

    tree.command_sender()
        .set_rich_text(rich, RichText::plain("Café"))
        .unwrap();
    tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    tree.paint(viewport(), 1.0).unwrap();
    let same = tree.diagnostics();
    assert_eq!(same.layout_passes, initial_diagnostics.layout_passes);
    assert_eq!(same.painted_nodes, initial_diagnostics.painted_nodes);
    assert_eq!(
        same.scene_cache_hits,
        initial_diagnostics.scene_cache_hits + 1
    );
    assert_eq!(system.borrow().cache_stats(), initial_cache);

    let red = Color::from_rgba8(212, 28, 33, 255);
    let colored = RichText::new(
        "Café",
        vec![TextSpan {
            range: 3..5,
            style: TextSpanStyle {
                color: Some(red),
                ..TextSpanStyle::default()
            },
        }],
    )
    .unwrap();
    source.set(colored).unwrap();
    tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    let recolored = tree.paint(viewport(), 1.0).unwrap();
    assert_eq!(tree.diagnostics().layout_passes, same.layout_passes);
    assert_eq!(tree.bounds(rich).unwrap(), initial_bounds);
    assert!(paint_has_red(&recolored));

    let update = RichText::new(
        "Café",
        vec![TextSpan {
            range: 3..5,
            style: TextSpanStyle {
                font_size: Some(40.0),
                color: Some(red),
                ..TextSpanStyle::default()
            },
        }],
    )
    .unwrap();
    source.set(update.clone()).unwrap();
    tree.flush_updates(viewport(), |_, _, _| {}).unwrap();
    let painted = tree.paint(viewport(), 1.0).unwrap();
    assert_eq!(tree.rich_text_source(rich).unwrap(), update);
    assert!(tree.bounds(rich).unwrap().size().height() > initial_bounds.size().height());
    assert!(tree.bounds(other).unwrap().origin().y() > initial_peer_y);
    assert!(device_glyphs(&painted).iter().any(|(_, glyph)| {
        glyph
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel[..3] == [212, 28, 33] && pixel[3] != 0)
    }));
    tree.set_text(rich, "Café").unwrap();
    tree.paint(viewport(), 1.0).unwrap();
    assert_eq!(tree.rich_text_source(rich).unwrap(), plain);
    assert_eq!(tree.bounds(rich).unwrap(), initial_bounds);
    assert!((tree.bounds(other).unwrap().origin().y() - initial_peer_y).abs() < 0.001);
}

#[test]
fn rich_spacing_and_line_height_updates_invalidate_measurement_on_the_same_node() {
    use oxiview_core::{RichText, TextSpan, TextSpanStyle, rich_text};

    let mut tree = UiTree::new().unwrap();
    let mut box_style = Style::default();
    box_style.layout.position = Position::Absolute;
    let id = tree
        .mount(
            tree.root(),
            rich_text(fixture_system(), RichText::plain("A A A"), font(20.0)).style(box_style),
        )
        .unwrap();
    tree.layout(viewport()).unwrap();

    let style = TextSpanStyle {
        letter_spacing: Some(5.0),
        word_spacing: Some(5.0),
        ..Default::default()
    };
    let updates = [
        TextSpanStyle {
            word_spacing: None,
            ..style.clone()
        },
        style.clone(),
        TextSpanStyle {
            line_height: Some(2.5),
            ..style
        },
    ];
    for (index, style) in updates.into_iter().enumerate() {
        let before = tree.text_size(id).unwrap();
        let diagnostics = tree.diagnostics();
        let source = RichText::new("A A A", vec![TextSpan { range: 0..3, style }]).unwrap();
        tree.set_rich_text(id, source.clone()).unwrap();
        tree.layout(viewport()).unwrap();
        let after = tree.text_size(id).unwrap();
        if index < 2 {
            assert!(after.width() > before.width());
            assert_eq!(after.height(), before.height());
        } else {
            assert_eq!(after.width(), before.width());
            assert!(after.height() > before.height());
        }
        assert_eq!(
            tree.diagnostics().layout_passes,
            diagnostics.layout_passes + 1
        );
        assert_eq!(tree.rich_text_source(id).unwrap(), source);
    }
}

#[test]
fn flex_and_grid_align_first_line_baselines_with_per_item_overrides() {
    use oxiview_core::{RichText, TextSpan, TextSpanStyle, rich_text};
    use oxiview_style::{Alignment, Display, TrackSize};

    let system = fixture_system();
    let multiline = RichText::new(
        "A LARGE\nmore text",
        vec![TextSpan {
            range: 2..7,
            style: TextSpanStyle {
                font_size: Some(39.0),
                ..TextSpanStyle::default()
            },
        }],
    )
    .unwrap();
    for display in [Display::Flex, Display::Grid] {
        for per_item in [false, true] {
            let mut tree = UiTree::new().unwrap();
            let root = tree.root();
            let mut container = tree.style(root).unwrap().clone();
            container.layout.display = display;
            container.layout.align_items = if per_item {
                Alignment::Start
            } else {
                Alignment::Baseline
            };
            container.layout.grid_template_columns = vec![TrackSize::Points(165.0); 4];
            tree.set_style(root, container).unwrap();

            let mut small_box = Style::default();
            small_box.layout.width = Length::Points(50.0);
            small_box.layout.border_width = 2.0;
            // Percentage block padding resolves against the containing inline size.
            small_box.layout.padding.top = Length::Percent(0.05);
            let mut rich_box = Style::default();
            rich_box.layout.border_width = 3.0;
            rich_box.layout.padding.top = Length::Points(5.0);
            if per_item {
                small_box.layout.align_self = Some(Alignment::Baseline);
                rich_box.layout.align_self = Some(Alignment::Baseline);
            }
            let mut empty_box = Style::default();
            empty_box.layout.height = Length::Points(48.0);
            empty_box.layout.align_self = Some(Alignment::Baseline);
            let mut start_box = Style::default();
            start_box.layout.align_self = Some(Alignment::Start);
            let small = tree
                .mount(
                    root,
                    text(Rc::clone(&system), "sm", font(12.0)).style(small_box),
                )
                .unwrap();
            let rich = tree
                .mount(
                    root,
                    rich_text(Rc::clone(&system), multiline.clone(), font(14.0)).style(rich_box),
                )
                .unwrap();
            let empty = tree.mount(root, node().style(empty_box)).unwrap();
            let start = tree
                .mount(
                    root,
                    text(Rc::clone(&system), "start", font(13.0)).style(start_box),
                )
                .unwrap();

            let small_top = if display == Display::Grid {
                165.0
            } else {
                680.0
            } * 0.05
                + 2.0;
            for width in [680.0, 570.0, 680.0] {
                let mut rich_box = tree.style(rich).unwrap().clone();
                rich_box.layout.width = Length::Points(if width < 600.0 { 75.0 } else { 150.0 });
                tree.set_style(rich, rich_box).unwrap();
                tree.layout(Size::new(width, 320.0)).unwrap();
                let small_bounds = tree.bounds(small).unwrap();
                let rich_bounds = tree.bounds(rich).unwrap();
                let small_top = if display == Display::Flex {
                    small_top + (width - 680.0) * 0.05
                } else {
                    small_top
                };
                let small_baseline = small_bounds.origin().y()
                    + small_top
                    + system
                        .borrow_mut()
                        .layout("sm", &font(12.0), Some(46.0))
                        .unwrap()
                        .first_baseline()
                        .unwrap();
                let paragraph = system
                    .borrow_mut()
                    .layout_rich(
                        &multiline,
                        &font(14.0),
                        Some(rich_bounds.size().width() - 6.0),
                    )
                    .unwrap();
                assert!(paragraph.line_count() > 1);
                let rich_baseline =
                    rich_bounds.origin().y() + 8.0 + paragraph.first_baseline().unwrap();
                let empty_baseline = tree.bounds(empty).unwrap().origin().y() + 48.0;
                assert!(
                    (small_baseline - rich_baseline).abs() < 0.01,
                    "{display:?} width {width} per-item {per_item}: small {small_baseline}, rich {rich_baseline}"
                );
                assert!((empty_baseline - rich_baseline).abs() < 0.01);
                assert!(tree.bounds(start).unwrap().origin().y().abs() < 0.01);
            }
        }
    }
}

#[test]
fn grid_uses_resolved_track_width_for_rich_first_line_and_propagates_nested_baselines() {
    use oxiview_core::{RichText, TextSpan, TextSpanStyle, rich_text};
    use oxiview_style::{Alignment, Display, TrackSize};

    let system = fixture_system();
    let content = RichText::new(
        "a WIDE\nline three",
        vec![TextSpan {
            range: 2..6,
            style: TextSpanStyle {
                font_size: Some(38.0),
                ..TextSpanStyle::default()
            },
        }],
    )
    .unwrap();
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let mut host = tree.style(root).unwrap().clone();
    host.layout.align_items = Alignment::Start;
    tree.set_style(root, host).unwrap();
    let mut grid = Style::default();
    grid.layout.display = Display::Grid;
    grid.layout.width = Length::Points(245.0);
    grid.layout.align_items = Alignment::Baseline;
    let grid_id = tree.mount(root, node().style(grid)).unwrap();
    let mut nested_box = Style::default();
    nested_box.layout.display = Display::Block;
    nested_box.layout.border_width = 2.0;
    nested_box.layout.padding.top = Length::Points(3.0);
    let nested = tree.mount(grid_id, node().style(nested_box)).unwrap();
    let rich = tree
        .mount(
            nested,
            rich_text(Rc::clone(&system), content.clone(), font(14.0)),
        )
        .unwrap();
    let small = tree
        .mount(grid_id, text(Rc::clone(&system), "peer", font(22.0)))
        .unwrap();

    let mut baselines = Vec::new();
    for track in [155.0, 90.0, 155.0] {
        let mut grid = tree.style(grid_id).unwrap().clone();
        grid.layout.grid_template_columns = vec![TrackSize::Points(track), TrackSize::Points(90.0)];
        tree.set_style(grid_id, grid).unwrap();
        tree.layout(viewport()).unwrap();
        let rich_bounds = tree.bounds(rich).unwrap();
        let small_bounds = tree.bounds(small).unwrap();
        let rich_metric = system
            .borrow_mut()
            .layout_rich(&content, &font(14.0), Some(rich_bounds.size().width()))
            .unwrap()
            .first_baseline()
            .unwrap();
        let small_metric = system
            .borrow_mut()
            .layout("peer", &font(22.0), Some(small_bounds.size().width()))
            .unwrap()
            .first_baseline()
            .unwrap();
        assert!(
            (rich_bounds.origin().y() + rich_metric - small_bounds.origin().y() - small_metric)
                .abs()
                < 0.01,
            "track {track}, rich {rich_bounds:?} metric {rich_metric}, small {small_bounds:?} metric {small_metric}"
        );
        let grid_bottom = tree.bounds(grid_id).unwrap().edges()[3];
        let nested_bottom = tree.bounds(nested).unwrap().edges()[3];
        let peer_bottom = small_bounds.edges()[3];
        assert!((grid_bottom - nested_bottom.max(peer_bottom)).abs() < 0.01);
        baselines.push(rich_metric);
    }
    assert!(baselines[0] > baselines[1]);
    assert_eq!(baselines[0], baselines[2]);
}
