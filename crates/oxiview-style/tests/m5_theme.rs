//! Versioned theme validation and typed rule resolution.

use oxiview_base::{Color, CornerRadii};
use oxiview_style::{Edges, FocusRing, InteractionState, Length, Theme, ThemeEnvironment};

#[test]
fn resolves_typed_palette_and_state_layers() -> Result<(), oxiview_style::ThemeError> {
    let theme = Theme::from_toml(
        r##"
        version = 1
        [tokens.colors]
        surface = "#eeeef0"
        system_accent = "#313257"
        [tokens.spacing]
        control_padding = 6
        ring_width = 2
        [tokens.radii]
        control_radius = 4.5
        [tokens.font_sizes]
        label = 15

        [dark.colors]
        surface = "#101122"
        [high_contrast.colors]
        system_accent = "#cdaa82ff"

        [rules.button]
        background = "@surface"
        padding = "@control_padding"
        border_radii = "@control_radius"
        text_size = "@label"
        [rules.button.hovered]
        background = "#808688"
        [rules.button.focused]
        background = "#101050"
        focus_ring = { color = "@system_accent", width = "@ring_width" }
        [rules.button.pressed]
        background = "#292990"
        [rules.button.disabled]
        background = "#888888"
    "##,
    )?;
    let idle = InteractionState::default();
    let light = theme.resolve("button", idle, ThemeEnvironment::default());
    assert_eq!(
        light.background,
        Some(Color::from_rgba8(0xee, 0xee, 0xf0, 255))
    );
    assert_eq!(light.padding, Some(Edges::all(Length::Points(6.0))));
    assert_eq!(light.border_radii, Some(CornerRadii::uniform(4.5)));
    assert_eq!(light.text_size, Some(15.0));
    assert!(light.focus_ring.is_none());

    let accent = Color::from_rgba8(20, 60, 81, 255);
    let environment = ThemeEnvironment {
        dark: true,
        accent_color: Some(accent),
        ..Default::default()
    };
    let dark = theme.resolve("button", idle, environment);
    assert_eq!(dark.background, Some(Color::from_rgba8(16, 17, 34, 255)));
    let mut combined = InteractionState {
        hovered: true,
        pressed: true,
        focused: true,
        disabled: false,
    };
    let pressed = theme.resolve("button", combined, environment);
    assert_eq!(
        pressed.background,
        Some(Color::from_rgba8(41, 41, 144, 255))
    );
    assert_eq!(
        pressed.focus_ring,
        Some(FocusRing {
            color: accent,
            width: 2.0
        })
    );

    combined.disabled = true;
    let contrast = theme.resolve(
        "button",
        combined,
        ThemeEnvironment {
            increased_contrast: true,
            ..environment
        },
    );
    assert_eq!(
        contrast.background,
        Some(Color::from_rgba8(136, 136, 136, 255))
    );
    assert_eq!(
        contrast.focus_ring.map(|ring| ring.color),
        Some(Color::from_rgba8(205, 170, 130, 255))
    );
    Ok(())
}

#[test]
fn validation_reports_the_offending_key_even_in_inactive_rules_and_palettes() {
    for (source, key) in [
        ("version = 2", "version"),
        ("version = 1\nmisspelling = true", "misspelling"),
        (
            "version = 1\n[tokens.spacing]\nsize = -1",
            "tokens.spacing.size",
        ),
        (
            "version = 1\n[tokens.radii]\nsize = nan",
            "tokens.radii.size",
        ),
        (
            "version = 1\n[tokens.font_sizes]\nsize = 0",
            "tokens.font_sizes.size",
        ),
        (
            "version = 1\n[tokens.colors]\nshade = '#aøbcd'",
            "tokens.colors.shade",
        ),
        (
            "version = 1\n[dark.colors]\nnew = '#000000'",
            "dark.colors.new",
        ),
        (
            "version = 1\n[rules.button]\npaddding = 0",
            "rules.button.paddding",
        ),
        (
            "version = 1\n[tokens.spacing]\nbody = 12\n[rules.button.disabled]\ntext_size = '@body'",
            "rules.button.disabled.text_size",
        ),
        (
            "version = 1\n[rules.button.focused]\nfocus_ring = { color = '#ffffff' }",
            "rules.button.focused.focus_ring.width",
        ),
    ] {
        assert_eq!(
            Theme::from_toml(source)
                .err()
                .as_ref()
                .map(|error| error.key()),
            Some(key),
            "{source}"
        );
    }
}
