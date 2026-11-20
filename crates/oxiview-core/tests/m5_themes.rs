//! Retained theme precedence, sparse invalidation, text, and system preferences.

use oxiview_base::{Color, Point, Size};
use oxiview_core::{Action, FocusPolicy, TextStyle, TextSystem, UiTree, View, node, text};
use oxiview_platform::{
    KeyCode, KeyModifiers, LogicalKey, SystemAppearance, SystemPreferences, WindowEvent,
};
use oxiview_reactive::{Scope, effect, signal};
use oxiview_style::{
    AppearanceMode, Edges, LayoutPatch, Length, StateVariant, Style, StylePatch, Theme,
};
use std::{cell::RefCell, rc::Rc, time::Duration};

const VIEWPORT: Size = Size::new(256.0, 100.0);
const RED: [u8; 3] = [190, 32, 40];
const DARK_RED: [u8; 3] = [133, 21, 26];

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn color([r, g, b]: [u8; 3]) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

fn focused_button() -> View {
    node()
        .theme_type("button")
        .focusable(FocusPolicy::Tab)
        .on_action(Action::new("test.activate", "Activate"), |_, _, _| {})
        .activate_with("test.activate")
}

fn key(code: KeyCode, pressed: bool) -> WindowEvent {
    WindowEvent::Key {
        code,
        logical: LogicalKey::Named(code),
        modifiers: KeyModifiers::default(),
        pressed,
        repeat: false,
        timestamp: Duration::from_millis(1),
    }
}

#[test]
fn local_properties_and_complete_bindings_keep_their_precedence_in_combined_states() -> TestResult {
    let theme = Theme::from_toml(
        r##"
        version = 1
        [rules.button]
        background = "#ffeedd"
        padding = 4
        [rules.button.hovered]
        background = "#99aaff"
        [rules.button.pressed]
        background = "#07079a"
        [rules.button.focused]
        focus_ring = { color = "#ddac11", width = 2 }
    "##,
    )?;
    let mut tree = UiTree::new()?;
    tree.set_theme(theme)?;
    let root = tree.root();
    let local = |shade| StylePatch {
        background: Some(color(shade)),
        ..Default::default()
    };
    let first = tree.mount(root, focused_button().style_patch(local(RED)))?;
    let second = tree.mount(
        root,
        focused_button()
            .style_patch(local(RED))
            .style_variant(StateVariant::Pressed, local(DARK_RED)),
    )?;
    let third = tree.mount(
        root,
        focused_button().style_patch(StylePatch {
            layout: LayoutPatch {
                padding: Some(Edges::all(Length::Points(0.0))),
                ..Default::default()
            },
            ..Default::default()
        }),
    )?;
    let scope = Scope::new()?;
    let fill = scope.run(|| signal(Style::default().background))?;
    let bound = tree.mount(
        root,
        focused_button().bind_style(move || {
            Ok(Style {
                background: fill.get()?,
                ..Default::default()
            })
        }),
    )?;
    let complete = tree.mount(root, focused_button().style(Style::default()))?;

    tree.focus(first)?;
    tree.dispatch(
        &WindowEvent::PointerMoved {
            position: Point::new(3.0, 12.0),
            timestamp: Duration::ZERO,
        },
        VIEWPORT,
        |_, _, _| {},
    )?;
    tree.dispatch(&key(KeyCode::Space, true), VIEWPORT, |_, _, _| {})?;
    let state = tree.interaction_state(first)?;
    assert!(state.pressed && state.focused && state.hovered);
    assert_eq!(tree.style(first)?.background, color(RED));
    assert_eq!(
        tree.style(first)?.layout.padding,
        Edges::all(Length::Points(4.0))
    );
    assert_eq!(
        tree.style(third)?.layout.padding,
        Edges::all(Length::Points(0.0))
    );
    tree.dispatch(&key(KeyCode::Space, false), VIEWPORT, |_, _, _| {})?;

    tree.focus(second)?;
    tree.dispatch(&key(KeyCode::Space, true), VIEWPORT, |_, _, _| {})?;
    assert_eq!(tree.style(second)?.background, color(DARK_RED));
    tree.focus(third)?;
    tree.dispatch(&key(KeyCode::Space, true), VIEWPORT, |_, _, _| {})?;
    assert_eq!(tree.style(third)?.background, color([7, 7, 154]));
    assert_eq!(tree.style(complete)?, &Style::default());
    assert_eq!(tree.style(bound)?, &Style::default());
    fill.set(color(RED))?;
    tree.flush_updates(VIEWPORT, |_, _, _| {})?;
    tree.focus(bound)?;
    tree.dispatch(&key(KeyCode::Space, true), VIEWPORT, |_, _, _| {})?;
    assert_eq!(tree.style(bound)?.background, color(RED));
    assert_eq!(
        tree.style(bound)?.layout.padding,
        Style::default().layout.padding
    );
    Ok(())
}

#[test]
fn color_revision_visits_only_unmasked_consumers_and_reapplication_stays_idle() -> TestResult {
    let candidate = |shade: &str| {
        Theme::from_toml(&format!(
            "version = 1\n[tokens.colors]\nfill = '{shade}'\n[rules.control]\nbackground = '@fill'"
        ))
    };
    let mut tree = UiTree::new()?;
    tree.set_theme(candidate("#aabbcc")?)?;
    let root = tree.root();
    let container = tree.mount(
        root,
        node().children((0..256).map(|index| {
            node().theme_type("control").style_patch(StylePatch {
                layout: LayoutPatch {
                    width: Some(Length::Points(1.0)),
                    ..Default::default()
                },
                background: (index >= 16).then(|| color(RED)),
                ..Default::default()
            })
        })),
    )?;
    let children = tree.children(container)?.to_vec();
    tree.paint(VIEWPORT, 1.0)?;
    let before = tree.diagnostics();

    assert!(tree.set_theme(candidate("#354252")?)?);
    tree.paint(VIEWPORT, 1.0)?;
    let after = tree.diagnostics();
    assert_eq!(after.layout_passes, before.layout_passes);
    assert_eq!(after.scene_builds, before.scene_builds);
    assert_eq!(after.scene_updates, before.scene_updates + 1);
    assert_eq!(after.paint_node_visits, before.paint_node_visits + 16);
    assert_eq!(tree.children(container)?, &children);
    assert_eq!(
        tree.style(children[0])?.background,
        color([0x35, 0x42, 0x52])
    );
    assert_eq!(tree.style(children[16])?.background, color(RED));

    assert!(!tree.set_theme(Theme::from_toml(
        "version = 1\n[rules.control]\nbackground = '#354252'"
    )?)?);
    assert!(!tree.flush_updates(VIEWPORT, |_, _, _| {})?.changed);
    assert_eq!(tree.diagnostics(), after);
    Ok(())
}

#[test]
fn themed_text_remeasures_for_logical_sizes_and_observes_atomic_preferences() -> TestResult {
    let theme_source = |size, foreground| {
        format!(
            "version = 1\n[tokens.colors]\nfill = '#ededed'\n\
         [dark.colors]\nfill = '#232323'\n\
         [rules.panel]\nbackground = '@fill'\n\
         [rules.label]\ntext_size = {size}\ntext_color = '{foreground}'"
        )
    };
    let mut tree = UiTree::new()?;
    tree.set_theme(Theme::from_toml(&theme_source(16, "#121212"))?)?;
    let mut fonts = TextSystem::without_system_fonts();
    fonts.register_font(include_bytes!("../../oxiview-text/tests/fonts/NotoSans.ttf").to_vec())?;
    let fonts = Rc::new(RefCell::new(fonts));
    let root = tree.root();
    let panel = tree.mount(root, node().theme_type("panel"))?;
    let label = tree.mount(
        panel,
        text(
            fonts,
            "Restart",
            TextStyle {
                family: Some("Noto Sans".into()),
                ..Default::default()
            },
        )
        .theme_type("label")
        .style_patch(StylePatch {
            layout: LayoutPatch {
                align_self: Some(Some(oxiview_style::Alignment::Start)),
                ..Default::default()
            },
            ..Default::default()
        }),
    )?;
    tree.paint(VIEWPORT, 1.0)?;
    let original = tree.text_size(label)?;
    let before = tree.diagnostics();

    assert!(tree.set_theme(Theme::from_toml(&theme_source(16, "#005577"))?)?);
    tree.paint(VIEWPORT, 1.0)?;
    assert_eq!(tree.diagnostics().layout_passes, before.layout_passes);
    assert_eq!(tree.text_size(label)?, original);

    assert!(tree.set_theme(Theme::from_toml(&theme_source(20, "#005577"))?)?);
    tree.paint(VIEWPORT, 1.0)?;
    let larger = tree.text_size(label)?;
    assert!(larger.width() > original.width() && larger.height() > original.height());

    let signals = tree.preference_signals();
    let scope = Scope::new()?;
    let observed = Rc::new(RefCell::new(Vec::new()));
    scope.run(|| {
        let observed = Rc::clone(&observed);
        effect(move || {
            observed.borrow_mut().push((
                signals.appearance()?,
                signals.text_scale()?,
                signals.reduced_motion()?,
                signals.accent_color()?,
                signals.increased_contrast()?,
            ));
            Ok(())
        })
    })?;
    let native = SystemPreferences {
        appearance: Some(SystemAppearance::Dark),
        text_scale: Some(1.25),
        reduced_motion: Some(true),
        accent_color: Some(color([12, 40, 80])),
        increased_contrast: Some(true),
    };
    assert!(
        tree.dispatch(
            &WindowEvent::PreferencesChanged {
                preferences: native
            },
            VIEWPORT,
            |_, _, _| {}
        )?
        .changed
    );
    tree.paint(VIEWPORT, 1.5)?;
    assert!(tree.text_size(label)?.width() > larger.width());
    assert_eq!(tree.style(panel)?.background, color([35, 35, 35]));
    assert_eq!(
        observed.borrow().as_slice(),
        &[
            (None, None, None, None, None),
            (
                native.appearance,
                native.text_scale,
                native.reduced_motion,
                native.accent_color,
                native.increased_contrast
            ),
        ]
    );

    assert!(!tree.set_system_preferences(native)?);
    assert!(tree.set_appearance_mode(AppearanceMode::Light)?);
    assert_eq!(tree.style(panel)?.background, color([237, 237, 237]));
    tree.paint(VIEWPORT, 1.0)?;
    assert!(!tree.set_system_preferences(SystemPreferences {
        appearance: Some(SystemAppearance::Light),
        ..native
    })?);
    assert!(!tree.flush_updates(VIEWPORT, |_, _, _| {})?.changed);
    Ok(())
}
