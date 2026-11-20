//! CPU rendering of a retained keyboard focus ring during pressed state.

use oxiview_base::{Color, Size};
use oxiview_core::{Action, FocusPolicy, UiTree, node};
use oxiview_platform::{
    KeyCode, KeyModifiers, LogicalKey, SystemAppearance, SystemPreferences, WindowEvent,
};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_scene::{PixelSize, RgbaImage};
use oxiview_style::{AppearanceMode, LayoutPatch, Length, StateVariant, StylePatch, Theme};
use oxiview_testing::{CompareConfig, SnapshotHarness};
use std::{path::Path, time::Duration};

const VIEWPORT: Size = Size::new(96.0, 56.0);
const LOCAL_PRESSED: [u8; 4] = [128, 31, 33, 255];
const RING: [u8; 4] = [255, 226, 84, 255];

const THEME: &str = r##"
    version = 1
    [tokens.colors]
    system_accent = "#ffe254"
    [high_contrast.colors]
    system_accent = "#ffffff"
    [rules.button]
    background = "#164078"
    border_radii = 8
    padding = 5
    [rules.button.focused]
    focus_ring = { color = "@system_accent", width = 3 }
    [rules.button.pressed]
    background = "#2986f6"
"##;

fn rgba([red, green, blue, alpha]: [u8; 4]) -> Color {
    Color::from_rgba8(red, green, blue, alpha)
}

fn pixel(image: &RgbaImage, x: usize, y: usize) -> &[u8] {
    let start = (y * image.width() as usize + x) * 4;
    &image.pixels()[start..start + 4]
}

fn paint(tree: &mut UiTree, scale: f64) -> Result<RgbaImage, Box<dyn std::error::Error>> {
    let frame = tree.paint(VIEWPORT, scale)?;
    Ok(CpuRenderer.render(
        &frame.scene,
        frame.root,
        PixelSize {
            width: (VIEWPORT.width() * scale) as u32,
            height: (VIEWPORT.height() * scale) as u32,
        },
    )?)
}

#[test]
fn combined_pressed_and_keyboard_focused_states_paint_separate_pixels_at_three_scales()
-> Result<(), Box<dyn std::error::Error>> {
    let mut tree = UiTree::new()?;
    tree.set_theme(Theme::from_toml(THEME)?)?;
    let root = tree.root();
    let control = tree.mount(
        root,
        node()
            .theme_type("button")
            .style_patch(StylePatch {
                background: Some(rgba([203, 49, 60, 255])),
                layout: LayoutPatch {
                    width: Some(Length::Points(72.0)),
                    height: Some(Length::Points(44.0)),
                    ..Default::default()
                },
                ..Default::default()
            })
            .style_variant(
                StateVariant::Pressed,
                StylePatch {
                    background: Some(rgba(LOCAL_PRESSED)),
                    ..Default::default()
                },
            )
            .focusable(FocusPolicy::Tab)
            .on_action(Action::new("test.button", "Activate button"), |_, _, _| {})
            .activate_with("test.button"),
    )?;
    tree.focus(control)?;
    tree.dispatch(
        &WindowEvent::Key {
            code: KeyCode::Space,
            logical: LogicalKey::Named(KeyCode::Space),
            modifiers: KeyModifiers::default(),
            pressed: true,
            repeat: false,
            timestamp: Duration::ZERO,
        },
        VIEWPORT,
        |_, _, _| {},
    )?;

    let state = tree.interaction_state(control)?;
    assert!(state.focused && state.pressed);
    let layout_passes = tree.diagnostics().layout_passes;
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let harness = SnapshotHarness::new(
        &crate_root.join("../..").canonicalize()?,
        crate_root,
        env!("CARGO_PKG_NAME"),
    )?;
    let mut differences = Vec::new();
    for (name, scale) in [("1x", 1.0), ("1_5x", 1.5), ("2x", 2.0)] {
        let image = paint(&mut tree, scale)?;
        let y = (22.0 * scale) as usize;
        assert_eq!(
            pixel(&image, (1.0 * scale) as usize, y),
            RING,
            "ring at scale {scale}"
        );
        let actual = oxiview_testing::RgbaImage::new(
            image.width(),
            image.height(),
            image.pixels().to_vec(),
        )?;
        if let Err(error) = harness.assert_snapshot(
            &format!("m5-focused-pressed-{name}"),
            &actual,
            &CompareConfig::default(),
        ) {
            differences.push(error.to_string());
        }
        assert_eq!(
            pixel(&image, (30.0 * scale) as usize, y),
            LOCAL_PRESSED,
            "fill at scale {scale}"
        );
    }
    assert_eq!(tree.diagnostics().layout_passes, layout_passes);

    assert!(!tree.set_appearance_mode(AppearanceMode::Light)?);
    let accent = [20, 220, 150, 255];
    let preferences = SystemPreferences {
        appearance: Some(SystemAppearance::Dark),
        accent_color: Some(rgba(accent)),
        ..Default::default()
    };
    let before = tree.diagnostics();
    assert!(tree.set_system_preferences(preferences)?);
    assert_eq!(pixel(&paint(&mut tree, 2.0)?, 2, 44), accent);
    assert_eq!(tree.diagnostics().scene_updates, before.scene_updates + 1);
    assert_eq!(tree.diagnostics().layout_passes, before.layout_passes);
    assert!(tree.set_system_preferences(SystemPreferences {
        increased_contrast: Some(true),
        ..preferences
    })?);
    assert_eq!(pixel(&paint(&mut tree, 2.0)?, 2, 44), [255, 255, 255, 255]);

    tree.dispatch(
        &WindowEvent::ActivationChanged {
            active: false,
            timestamp: Duration::ZERO,
        },
        VIEWPORT,
        |_, _, _| {},
    )?;
    assert_eq!(tree.focused_node(), Some(control));
    assert_eq!(pixel(&paint(&mut tree, 1.0)?, 1, 22), [203, 49, 60, 255]);
    if !differences.is_empty() {
        return Err(differences.join("\n").into());
    }
    Ok(())
}
