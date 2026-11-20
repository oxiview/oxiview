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
