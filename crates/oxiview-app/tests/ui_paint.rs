//! CPU pixel coverage of retained UI painting through the application boundary.
#![allow(clippy::unwrap_used)] // Test fixture setup and assertions.

use oxiview_base::{Color, CornerRadii, Point, Rect, Size};
use oxiview_core::{NodeId, PaintedScene, UiTree};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_scene::{Brush, PixelSize, Primitive, RgbaImage};
use oxiview_style::{Display, Length, Overflow, Position, Style};
use oxiview_testing::{CompareConfig, SnapshotHarness};
use std::path::Path;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const VIEWPORT: Size = Size::new(124.0, 64.0);
const CANVAS: [u8; 4] = [22, 29, 46, 255];
const PANEL: [u8; 4] = [42, 57, 81, 255];
const CYAN: [u8; 4] = [91, 205, 225, 255];
const GOLD: [u8; 4] = [245, 185, 79, 255];
const RED: [u8; 4] = [227, 106, 110, 255];
const BLUE: [u8; 4] = [91, 132, 241, 255];
const TRANSPARENT: [u8; 4] = [0; 4];

fn color([r, g, b, a]: [u8; 4]) -> Color {
    Color::from_rgba8(r, g, b, a)
}

fn absolute(rect: [f64; 4], background: [u8; 4], z_index: Option<i32>) -> Style {
    let [left, top, width, height] = rect;
    let mut style = Style {
        background: color(background),
        z_index,
        ..Style::default()
    };
    style.layout.position = Position::Absolute;
    style.layout.width = Length::Points(width);
    style.layout.height = Length::Points(height);
    style.layout.inset.left = Length::Points(left);
    style.layout.inset.top = Length::Points(top);
    style
}

fn edit(tree: &mut UiTree, id: NodeId, change: impl FnOnce(&mut Style)) {
    let mut style = tree.style(id).unwrap().clone();
    change(&mut style);
    assert!(tree.set_style(id, style).unwrap());
}

fn stacking_row(tree: &mut UiTree, top: f64, z_index: Option<i32>) -> (NodeId, NodeId, NodeId) {
    let root = tree.root();
    let wrapper = tree
        .append(
            root,
            absolute([68.0, top, 46.0, 24.0], [78, 120, 160, 128], z_index),
        )
        .unwrap();
    let mut nested = absolute([16.0, 4.25, 22.0, 16.0], GOLD, Some(2));
    nested.layout.position = Position::Relative;
    let nested = tree.append(wrapper, nested).unwrap();
    let outside = tree
        .append(root, absolute([92.0, top + 6.0, 25.0, 11.0], BLUE, Some(1)))
        .unwrap();
    (wrapper, nested, outside)
}

struct Fixture {
    tree: UiTree,
    clip: NodeId,
    wrapper: NodeId,
    nested: NodeId,
    outside: NodeId,
}

fn fixture() -> Fixture {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    edit(&mut tree, root, |style| style.background = color(CANVAS));

    let mut rounded = absolute([4.0, 4.0, 56.0, 56.0], PANEL, None);
    rounded.layout.border_width = 4.0;
    rounded.border_color = color(CYAN);
    rounded.border_radii = CornerRadii::new(48.0, 32.0, 44.0, 28.0);
    rounded.overflow = Overflow::Clip;
    let clip = tree.append(root, rounded).unwrap();
    let mut group = absolute([-8.0, -4.0, 68.0, 56.0], TRANSPARENT, None);
    group.opacity = 0.5;
    let group = tree.append(clip, group).unwrap();
    tree.append(group, absolute([0.0, 0.0, 48.0, 56.0], RED, None))
        .unwrap();
    tree.append(group, absolute([28.0, 12.0, 50.0, 32.0], GOLD, None))
        .unwrap();

    let (wrapper, nested, outside) = stacking_row(&mut tree, 4.0, None);
    stacking_row(&mut tree, 34.0, Some(0));

    Fixture {
        tree,
        clip,
        wrapper,
        nested,
        outside,
    }
}

fn pixels(tree: &mut UiTree, scale: f64) -> RgbaImage {
    render(&tree.paint(VIEWPORT, scale).unwrap(), scale)
}

fn render(frame: &PaintedScene, scale: f64) -> RgbaImage {
    CpuRenderer
        .render(
            &frame.scene,
            frame.root,
            PixelSize {
                width: (VIEWPORT.width() * scale) as u32,
                height: (VIEWPORT.height() * scale) as u32,
            },
        )
        .unwrap()
}

fn pixel(image: &RgbaImage, scale: f64, x: u32, y: u32) -> [u8; 4] {
    let x = (f64::from(x) * scale) as usize;
    let y = (f64::from(y) * scale) as usize;
    let start = (y * image.width() as usize + x) * 4;
    image.pixels()[start..start + 4].try_into().unwrap()
}

fn assert_fixture_pixels(image: &RgbaImage, scale: f64) {
    assert_eq!(pixel(image, scale, 12, 13), CANVAS, "oversized outer radii");
    assert_eq!(pixel(image, scale, 15, 15), CYAN, "visible rounded border");
    assert_eq!(pixel(image, scale, 6, 32), CYAN, "padding-box clip");
    assert_eq!(
        pixel(image, scale, 62, 30),
        CANVAS,
        "overflow outside the border"
    );
    let single = pixel(image, scale, 53, 30);
    for (actual, expected) in single.into_iter().zip([144u8, 121, 80, 255]) {
        assert!(actual.abs_diff(expected) <= 1, "half opacity: {single:?}");
    }
    assert_eq!(
        pixel(image, scale, 35, 30),
        single,
        "opacity applies once to the overlap"
    );
    assert_eq!(
        pixel(image, scale, 96, 14),
        GOLD,
        "auto leaves descendant z-index effective"
    );
    assert_eq!(
        pixel(image, scale, 88, 44),
        GOLD,
        "zero context still draws its child"
    );
    assert_eq!(
        pixel(image, scale, 96, 44),
        BLUE,
        "zero context isolates its child"
    );
}

fn snapshot(name: &str, scale: f64) -> TestResult {
    let image = pixels(&mut fixture().tree, scale);
    assert_fixture_pixels(&image, scale);
    let image =
        oxiview_testing::RgbaImage::new(image.width(), image.height(), image.pixels().to_vec())?;
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = crate_root.join("../..").canonicalize()?;
    SnapshotHarness::new(&workspace, crate_root, env!("CARGO_PKG_NAME"))?.assert_snapshot(
        name,
        &image,
        &CompareConfig::default(),
    )?;
    Ok(())
}

#[test]
fn tree_at_1x() -> TestResult {
    snapshot("m2-ui-tree-1x", 1.0)
}

#[test]
fn tree_at_1_5x() -> TestResult {
    snapshot("m2-ui-tree-1_5x", 1.5)
}

#[test]
fn tree_at_2x() -> TestResult {
    snapshot("m2-ui-tree-2x", 2.0)
}

#[test]
fn cpu_repaint_reuses_clean_nodes_across_content_and_context_changes() {
    for scale in [1.0, 1.5, 2.0] {
        let Fixture {
            mut tree,
            clip,
            wrapper,
            nested,
            outside,
        } = fixture();
        let initial = pixels(&mut tree, scale);
        assert_fixture_pixels(&initial, scale);
        let before = tree.diagnostics();
        let nested_bounds = tree.bounds(nested).unwrap();
        assert_eq!(nested_bounds.origin(), Point::new(84.0, 8.25));
        assert_eq!(before.painted_nodes, tree.len() as u64);
        assert_eq!(pixels(&mut tree, scale), initial);
        assert_eq!(
            tree.diagnostics().scene_cache_hits,
            before.scene_cache_hits + 1
        );

        edit(&mut tree, outside, |style| style.z_index = Some(3));
        assert_eq!(pixel(&pixels(&mut tree, scale), scale, 96, 14), BLUE);
        edit(&mut tree, outside, |style| style.z_index = Some(1));
        edit(&mut tree, wrapper, |style| style.opacity = 0.75);
        assert_eq!(pixel(&pixels(&mut tree, scale), scale, 96, 14), BLUE);
        edit(&mut tree, wrapper, |style| style.opacity = 1.0);
        assert_eq!(pixels(&mut tree, scale), initial);
        assert_eq!(tree.diagnostics().painted_nodes, before.painted_nodes);
        assert_eq!(tree.diagnostics().layout_passes, before.layout_passes);
        assert_eq!(tree.bounds(nested).unwrap(), nested_bounds);

        edit(&mut tree, clip, |style| style.overflow = Overflow::Visible);
        assert_ne!(pixel(&pixels(&mut tree, scale), scale, 62, 30), CANVAS);
        edit(&mut tree, clip, |style| style.overflow = Overflow::Clip);
        assert_eq!(pixels(&mut tree, scale), initial);

        let mut new_tree = fixture();
        for (tree, nested) in [(&mut tree, nested), (&mut new_tree.tree, new_tree.nested)] {
            tree.set_content(nested, vec![fill(Rect::new(0.0, 0.0, 22.0, 16.0), RED)])
                .unwrap();
        }
        let updated = pixels(&mut tree, scale);
        assert_eq!(pixel(&updated, scale, 96, 14), RED);
        assert_eq!(updated, pixels(&mut new_tree.tree, scale));
        assert_eq!(tree.diagnostics().painted_nodes, before.painted_nodes + 3);
        assert_eq!(tree.diagnostics().layout_passes, before.layout_passes);
        assert_eq!(tree.diagnostics().scene_builds, before.scene_builds + 6);
        assert_eq!(tree.diagnostics().scene_updates, before.scene_updates);
    }
}

fn fill(rect: Rect, rgba: [u8; 4]) -> Primitive {
    Primitive::Fill(rect, CornerRadii::default(), Brush::Solid(color(rgba)))
}

fn overlapping_draws(first: [u8; 4], second: [u8; 4]) -> Vec<Primitive> {
    vec![
        fill(Rect::new(0.0, 0.0, 22.0, 16.0), first),
        fill(Rect::new(10.0, 2.0, 20.0, 12.0), second),
    ]
}

#[test]
fn sparse_draw_edits_match_fresh_pixels_and_preserve_snapshots() {
    for scale in [1.0, 1.5, 2.0] {
        let mut incremental = fixture();
        incremental
            .tree
            .set_content(incremental.nested, overlapping_draws(RED, GOLD))
            .unwrap();
        let published = incremental.tree.paint(VIEWPORT, scale).unwrap();
        let initial = render(&published, scale);
        let before = incremental.tree.diagnostics();

        let mut fresh = fixture();
        for fixture in [&mut incremental, &mut fresh] {
            edit(&mut fixture.tree, fixture.clip, |style| {
                style.background = color(RED);
            });
            edit(&mut fixture.tree, fixture.clip, |style| {
                style.border_color = color(GOLD);
            });
            fixture
                .tree
                .set_content(fixture.nested, overlapping_draws(CYAN, PANEL))
                .unwrap();
        }

        let updated = pixels(&mut incremental.tree, scale);
        assert_ne!(updated, initial);
        assert_eq!(pixel(&initial, scale, 96, 14), GOLD);
        assert_eq!(pixel(&updated, scale, 96, 14), PANEL);
        assert_eq!(updated, pixels(&mut fresh.tree, scale));
        assert_eq!(render(&published, scale), initial);
        let after = incremental.tree.diagnostics();
        assert_eq!(after.scene_builds, before.scene_builds);
        assert_eq!(after.scene_updates, before.scene_updates + 1);
        assert_eq!(after.paint_node_visits, before.paint_node_visits + 2);
        assert_eq!(after.painted_nodes, before.painted_nodes);
        assert_eq!(after.layout_passes, before.layout_passes);
    }
}

#[test]
fn invalid_sparse_draw_discards_the_batch_and_recovery_rebuilds() {
    let mut incremental = fixture();
    incremental
        .tree
        .set_content(incremental.nested, overlapping_draws(RED, GOLD))
        .unwrap();
    let published = incremental.tree.paint(VIEWPORT, 1.0).unwrap();
    let initial = render(&published, 1.0);
    let before = incremental.tree.diagnostics();

    edit(&mut incremental.tree, incremental.clip, |style| {
        style.border_color = color(GOLD);
    });
    incremental
        .tree
        .set_content(
            incremental.nested,
            vec![
                fill(Rect::new(0.0, 0.0, 22.0, 16.0), CYAN),
                Primitive::Stroke(
                    Rect::new(10.0, 2.0, 20.0, 12.0),
                    CornerRadii::default(),
                    -1.0,
                    Brush::Solid(color(PANEL)),
                ),
            ],
        )
        .unwrap();
    for _ in 0..2 {
        assert!(matches!(
            incremental.tree.paint(VIEWPORT, 1.0),
            Err(oxiview_core::UiError::Scene(_))
        ));
        assert_eq!(incremental.tree.diagnostics(), before);
        assert_eq!(render(&published, 1.0), initial);
    }

    let mut fresh = fixture();
    edit(&mut fresh.tree, fresh.clip, |style| {
        style.border_color = color(GOLD);
    });
    for fixture in [&mut incremental, &mut fresh] {
        fixture
            .tree
            .set_content(fixture.nested, overlapping_draws(CYAN, PANEL))
            .unwrap();
    }
    let recovered = pixels(&mut incremental.tree, 1.0);
    assert_eq!(recovered, pixels(&mut fresh.tree, 1.0));
    assert_eq!(render(&published, 1.0), initial);
    let after = incremental.tree.diagnostics();
    assert_eq!(after.scene_builds, before.scene_builds + 1);
    assert_eq!(after.scene_updates, before.scene_updates);
    assert_eq!(after.painted_nodes, before.painted_nodes);
    assert_eq!(after.layout_passes, before.layout_passes);

    incremental
        .tree
        .set_content(incremental.nested, overlapping_draws(GOLD, RED))
        .unwrap();
    assert_eq!(pixel(&pixels(&mut incremental.tree, 1.0), 1.0, 96, 14), RED);
    assert_eq!(
        incremental.tree.diagnostics().scene_updates,
        after.scene_updates + 1
    );
}

#[test]
fn cpu_paint_recovers_from_failure_and_uses_current_subtree_placement() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let mut clipped_parent = absolute([8.0, 6.0, 20.0, 18.0], PANEL, None);
    clipped_parent.overflow = Overflow::Clip;
    let left = tree.append(root, clipped_parent).unwrap();
    let right = tree
        .append(root, absolute([44.0, 6.0, 20.0, 18.0], PANEL, None))
        .unwrap();
    let child = tree
        .append(left, absolute([12.0, 2.0, 16.0, 10.0], RED, Some(1)))
        .unwrap();
    let initial = pixels(&mut tree, 1.0);
    assert_eq!(pixel(&initial, 1.0, 22, 10), RED);
    assert_eq!(pixel(&initial, 1.0, 32, 10), TRANSPARENT);
    let published = tree.paint(VIEWPORT, 1.0).unwrap();
    let before = tree.diagnostics();
    edit(&mut tree, left, |style| style.background = color(CYAN));
    tree.set_content(
        child,
        vec![Primitive::Stroke(
            Rect::new(0.0, 0.0, 8.0, 8.0),
            CornerRadii::default(),
            -1.0,
            Brush::Solid(color(GOLD)),
        )],
    )
    .unwrap();
    assert!(matches!(
        tree.paint(VIEWPORT, 1.0),
        Err(oxiview_core::UiError::Scene(_))
    ));
    assert_eq!(tree.diagnostics(), before);
    let published_pixels = CpuRenderer
        .render(
            &published.scene,
            published.root,
            PixelSize {
                width: 124,
                height: 64,
            },
        )
        .unwrap();
    assert_eq!(published_pixels, initial);

    tree.set_content(child, vec![fill(Rect::new(0.0, 0.0, 16.0, 10.0), GOLD)])
        .unwrap();
    let recovered = pixels(&mut tree, 1.0);
    assert_eq!(tree.diagnostics().painted_nodes, before.painted_nodes + 1);
    assert_eq!(pixel(&recovered, 1.0, 10, 10), CYAN);
    assert_eq!(pixel(&recovered, 1.0, 22, 10), GOLD);

    let before = tree.diagnostics();
    tree.reparent(child, right, 0).unwrap();
    let moved = pixels(&mut tree, 1.0);
    assert_eq!(tree.diagnostics().painted_nodes, before.painted_nodes + 1);
    assert_eq!(pixel(&moved, 1.0, 22, 10), CYAN);
    assert_eq!(pixel(&moved, 1.0, 68, 10), GOLD);

    edit(&mut tree, right, |style| {
        style.layout.display = Display::None
    });
    let suppressed = pixels(&mut tree, 1.0);
    assert_eq!(pixel(&suppressed, 1.0, 68, 10), TRANSPARENT);
    assert_eq!(tree.diagnostics().painted_nodes, before.painted_nodes + 1);
    tree.set_content(child, vec![]).unwrap();
    assert_eq!(pixels(&mut tree, 1.0), suppressed);
    edit(&mut tree, right, |style| {
        style.layout.display = Display::Flex
    });
    assert_eq!(pixel(&pixels(&mut tree, 1.0), 1.0, 68, 10), RED);
    assert_eq!(tree.diagnostics().painted_nodes, before.painted_nodes + 2);

    tree.remove(right).unwrap();
    let replacement = tree
        .append(root, absolute([44.0, 6.0, 20.0, 18.0], BLUE, None))
        .unwrap();
    assert_ne!(replacement, right);
    assert!(!tree.contains(child));
    let replaced = pixels(&mut tree, 1.0);
    assert_eq!(pixel(&replaced, 1.0, 56, 10), BLUE);
    assert_eq!(pixel(&replaced, 1.0, 68, 10), TRANSPARENT);
}

#[test]
fn cpu_paint_uses_stacking_phases_and_tree_order_for_static_items() {
    for display in [Display::Block, Display::Flex, Display::Grid] {
        let mut tree = UiTree::new().unwrap();
        let root = tree.root();
        if display != Display::Flex {
            edit(&mut tree, root, |style| style.layout.display = display);
        }
        tree.append(root, absolute([0.0, 0.0, 86.0, 20.0], RED, Some(-1)))
            .unwrap();
        let mut flow = Style {
            background: color(GOLD),
            z_index: Some(4),
            ..Style::default()
        };
        flow.layout.width = Length::Points(80.0);
        flow.layout.height = Length::Points(20.0);
        let flow = tree.append(root, flow).unwrap();
        tree.append(root, absolute([20.0, 0.0, 60.0, 20.0], BLUE, None))
            .unwrap();
        tree.append(root, absolute([40.0, 0.0, 40.0, 20.0], CYAN, Some(2)))
            .unwrap();
        let tie = tree
            .append(root, absolute([60.0, 0.0, 20.0, 20.0], PANEL, Some(2)))
            .unwrap();
        let image = pixels(&mut tree, 1.0);
        let bounds = tree.bounds(flow).unwrap();
        let before = tree.diagnostics();
        let upper = if display == Display::Block {
            PANEL
        } else {
            GOLD
        };
        assert_eq!(pixel(&image, 1.0, 65, 10), upper, "{display:?}");
        assert_eq!(pixel(&image, 1.0, 84, 10), RED);

        edit(&mut tree, flow, |style| style.z_index = Some(-4));
        let image = pixels(&mut tree, 1.0);
        let phase = if display == Display::Block { GOLD } else { RED };
        assert_eq!(pixel(&image, 1.0, 10, 10), phase, "{display:?}");
        assert_eq!(pixel(&image, 1.0, 30, 10), BLUE, "positioned auto phase");
        assert_eq!(pixel(&image, 1.0, 45, 10), CYAN, "positive phase");
        assert_eq!(pixel(&image, 1.0, 65, 10), PANEL, "later equal z-index");
        assert_eq!(tree.bounds(flow).unwrap(), bounds);
        assert_eq!(tree.diagnostics().painted_nodes, before.painted_nodes);
        assert_eq!(tree.diagnostics().layout_passes, before.layout_passes);

        tree.reparent(tie, root, 0).unwrap();
        assert_eq!(pixel(&pixels(&mut tree, 1.0), 1.0, 65, 10), CYAN);
        assert_eq!(tree.diagnostics().painted_nodes, before.painted_nodes);
    }
}

#[test]
fn clip_preserves_block_margin_collapse_and_opacity_preserves_containing_block() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let mut parent = absolute([10.0, 10.0, 80.0, 50.0], TRANSPARENT, None);
    parent.layout.display = Display::Block;
    let parent = tree.append(root, parent).unwrap();
    let mut flow = Style {
        background: color(BLUE),
        ..Style::default()
    };
    flow.layout.width = Length::Points(26.0);
    flow.layout.height = Length::Points(8.0);
    flow.layout.margin.left = Length::Points(5.0);
    let mut first = flow.clone();
    first.layout.margin.bottom = Length::Points(6.0);
    let first = tree.append(parent, first).unwrap();
    flow.layout.margin.top = Length::Points(10.0);
    let second = tree.append(parent, flow).unwrap();
    let positioned = tree
        .append(first, absolute([40.0, 4.0, 10.0, 8.0], RED, Some(1)))
        .unwrap();
    let initial = pixels(&mut tree, 1.0);
    let positioned_bounds = tree.bounds(positioned).unwrap();
    let second_bounds = tree.bounds(second).unwrap();
    assert_eq!(positioned_bounds.origin(), Point::new(50.0, 14.0));
    assert_eq!(second_bounds.origin(), Point::new(15.0, 28.0));
    assert_eq!(pixel(&initial, 1.0, 51, 16), RED);
    assert_eq!(pixel(&initial, 1.0, 18, 30), BLUE);
    let before = tree.diagnostics();

    edit(&mut tree, parent, |style| style.overflow = Overflow::Clip);
    assert_eq!(pixels(&mut tree, 1.0), initial);
    assert_eq!(tree.bounds(second).unwrap(), second_bounds);
    edit(&mut tree, first, |style| style.opacity = 0.5);
    let opacity = pixel(&pixels(&mut tree, 1.0), 1.0, 51, 16)[3];
    assert!(opacity.abs_diff(128) <= 1);
    assert_eq!(tree.bounds(positioned).unwrap(), positioned_bounds);
    assert_eq!(tree.diagnostics().layout_passes, before.layout_passes);
    assert_eq!(tree.diagnostics().painted_nodes, before.painted_nodes + 1);
}
