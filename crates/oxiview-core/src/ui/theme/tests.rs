use super::*;
use crate::{TextStyle, TextSystem, node, text};
use std::cell::RefCell;

#[test]
fn legacy_nodes_keep_their_original_style_without_interaction_allocations()
-> Result<(), Box<dyn std::error::Error>> {
    let mut tree = UiTree::new()?;
    let root = tree.root();
    let style = Style {
        background: Color::from_rgba8(0x88, 0x33, 0x55, 0xff),
        ..Style::default()
    };
    let legacy = tree.mount(
        root,
        node().children((0..256).map(|_| node().style(style.clone()))),
    )?;
    let themed = tree.mount(
        root,
        node()
            .theme_type("button")
            .focusable(crate::FocusPolicy::Tab),
    )?;
    tree.focus(themed)?;
    assert!(tree.set_theme(Theme::from_toml(
        "version = 1\n[rules.button.focused]\nbackground = '#334499'"
    )?)?);
    assert_eq!(tree.theme.consumers.len(), 1);
    for &id in tree.children(legacy)? {
        assert_eq!(tree.interaction_state(id)?, InteractionState::default());
        let node = tree.node(id)?;
        assert_eq!(node.style, style);
        assert!(node.interaction.is_none() && node.interaction_signal.get().is_none());
        assert!(node.theme.is_none());
    }
    assert!(tree.node(themed)?.interaction_signal.get().is_none());
    Ok(())
}

#[test]
fn a_base_focus_ring_only_affects_paint_while_keyboard_focus_is_visible()
-> Result<(), Box<dyn std::error::Error>> {
    let mut tree = UiTree::new()?;
    let id = tree.mount(
        tree.root(),
        node()
            .theme_type("button")
            .focusable(crate::FocusPolicy::Tab),
    )?;
    let theme = Theme::from_toml(
        "version = 1\n[rules.button]\nfocus_ring = { color = '#123456', width = 2 }",
    )?;
    let ring = |tree: &UiTree| -> Result<Option<FocusRing>, UiError> {
        Ok(tree
            .node(id)?
            .theme
            .as_ref()
            .and_then(|style| style.focus_ring))
    };
    assert!(!tree.set_theme(theme)?);
    assert_eq!(ring(&tree)?, None);
    tree.focus(id)?;
    assert!(ring(&tree)?.is_some());
    tree.dispatch(
        &oxiview_platform::WindowEvent::ActivationChanged {
            active: false,
            timestamp: std::time::Duration::ZERO,
        },
        oxiview_base::Size::new(50.0, 50.0),
        |_, _, _| {},
    )?;
    assert_eq!(tree.focused_node(), Some(id));
    assert_eq!(ring(&tree)?, None);
    Ok(())
}

#[test]
fn a_preparation_failure_preserves_the_entire_installed_revision()
-> Result<(), Box<dyn std::error::Error>> {
    let mut tree = UiTree::new()?;
    let mut fonts = TextSystem::without_system_fonts();
    fonts.register_font(
        include_bytes!("../../../../oxiview-text/tests/fonts/NotoSans.ttf").to_vec(),
    )?;
    let fonts = Rc::new(RefCell::new(fonts));
    let panel = tree.mount(tree.root(), node().theme_type("panel"))?;
    let label = tree.mount(
        panel,
        text(
            fonts,
            "Theme",
            TextStyle {
                family: Some("Noto Sans".into()),
                ..Default::default()
            },
        )
        .theme_type("label"),
    )?;
    let original = Theme::from_toml(
        "version = 1\n[rules.panel]\nbackground = '#23344a'\n[rules.label]\ntext_size = 16",
    )?;
    let next = Theme::from_toml(
        "version = 1\n[rules.panel]\nbackground = '#774411'\n[rules.label]\ntext_size = 21",
    )?;
    tree.set_theme(original.clone())?;
    let previous = tree.style(panel)?.background;
    let revision = std::mem::replace(&mut tree.node_mut(label)?.content_revision, u64::MAX);
    assert_eq!(
        tree.set_theme(next.clone()),
        Err(UiError::RevisionExhausted)
    );
    assert_eq!(tree.style(panel)?.background, previous);
    assert_eq!(tree.theme.theme, original);
    tree.node_mut(label)?.content_revision = revision;
    assert!(tree.set_theme(next)?);
    assert_eq!(
        tree.style(panel)?.background,
        Color::from_rgba8(0x77, 0x44, 0x11, 255)
    );
    Ok(())
}
