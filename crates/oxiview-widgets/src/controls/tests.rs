use super::*;
use oxiview_base::Size;
use oxiview_platform::{PointerButton, WindowEvent};
use oxiview_style::Theme;
use std::time::Duration;

const VIEWPORT: Size = Size::new(320., 160.);

fn system() -> Result<Rc<RefCell<TextSystem>>, Box<dyn std::error::Error>> {
    let mut text = TextSystem::without_system_fonts();
    text.register_font(include_bytes!("../../../oxiview-text/tests/fonts/NotoSans.ttf").to_vec())?;
    Ok(Rc::new(RefCell::new(text)))
}

fn typography() -> TextStyle {
    TextStyle {
        family: Some("Noto Sans".into()),
        font_size: 12.,
        ..Default::default()
    }
}

fn point(bounds: Rect, offset: Point) -> Point {
    let origin = bounds.origin();
    Point::new(origin.x() + offset.x(), origin.y() + offset.y())
}

fn pointer(position: Point, pressed: bool) -> WindowEvent {
    WindowEvent::PointerButton {
        button: PointerButton::Primary,
        position,
        pressed,
        timestamp: Duration::ZERO,
    }
}

fn key(code: KeyCode, pressed: bool) -> WindowEvent {
    WindowEvent::Key {
        code,
        logical: LogicalKey::Named(code),
        modifiers: KeyModifiers::default(),
        pressed,
        repeat: false,
        timestamp: Duration::ZERO,
    }
}

fn send(tree: &mut UiTree, event: WindowEvent) -> Result<(), UiError> {
    tree.dispatch(&event, VIEWPORT, |_, _, _| {})?;
    Ok(())
}

#[test]
fn button_uses_retained_activation_and_theme_state() -> Result<(), Box<dyn std::error::Error>> {
    let mut tree = UiTree::new()?;
    tree.set_theme(Theme::from_toml(CONTROL_THEME)?)?;
    let invoked = Rc::new(Cell::new(0));
    let counter = invoked.clone();
    let button = Button::default();
    let id = tree.mount(
        tree.root(),
        button.view(system()?, "Open", typography(), move || {
            counter.set(counter.get() + 1);
        }),
    )?;
    assert_eq!(button.node(), Some(id));
    let semantics = tree.semantics(id)?.ok_or("missing button semantics")?;
    assert_eq!(
        (semantics.role.as_str(), semantics.label.as_str()),
        ("button", "Open")
    );
    assert_eq!(semantics.actions, [ACTIVATE]);
    tree.layout(VIEWPORT)?;
    let position = point(tree.bounds(id)?, Point::new(6., 6.));
    send(
        &mut tree,
        WindowEvent::PointerMoved {
            position,
            timestamp: Duration::ZERO,
        },
    )?;
    assert!(tree.interaction_state(id)?.hovered);
    assert_eq!(tree.style(id)?.background, rgb(0xe8f2ef));
    send(&mut tree, pointer(position, true))?;
    assert!(tree.interaction_state(id)?.pressed);
    send(&mut tree, pointer(position, false))?;
    assert_eq!(invoked.get(), 1);

    send(&mut tree, key(KeyCode::Space, true))?;
    send(&mut tree, key(KeyCode::Space, false))?;
    assert_eq!(invoked.get(), 2);
    button.set_enabled(&mut tree, false)?;
    assert!(tree.interaction_state(id)?.disabled);
    assert_eq!(tree.style(id)?.opacity, 0.45);
    assert!(!tree.invoke_action(id, ACTIVATE)?);
    send(&mut tree, pointer(position, true))?;
    send(&mut tree, pointer(position, false))?;
    assert_eq!(invoked.get(), 2);

    tree.set_theme(Theme::from_toml(
        r##"
        version = 1
        [rules.button]
        background = "#123456"
    "##,
    )?)?;
    assert_eq!(tree.style(id)?.background, rgb(0x123456));
    Ok(())
}

#[test]
fn toggles_segments_and_field_publish_current_values() -> Result<(), Box<dyn std::error::Error>> {
    let mut tree = UiTree::new()?;
    tree.set_theme(Theme::from_toml(CONTROL_THEME)?)?;
    let system = system()?;
    let switches = Toggle::default();
    let next = Rc::new(Cell::new(false));
    let changed = next.clone();
    let toggle = tree.mount(
        tree.root(),
        switches.view(
            system.clone(),
            "Follow",
            typography(),
            ToggleKind::Switch,
            false,
            move |value| changed.set(value),
        ),
    )?;
    assert!(tree.invoke_action(toggle, ACTIVATE)?);
    assert!(next.get());
    switches.set_checked(&mut tree, ToggleKind::Switch, true)?;
    assert_eq!(
        tree.semantics(toggle)?.ok_or("missing switch")?.value,
        "true"
    );
    assert!(tree.invoke_action(toggle, ACTIVATE)?);
    assert!(!next.get());

    let choice = Segmented::default();
    let selection = Rc::new(Cell::new(0));
    let selected = selection.clone();
    tree.mount(
        tree.root(),
        choice.view(
            system.clone(),
            "Presentation",
            ["List", "Table"],
            typography(),
            move |value| selected.set(value),
        )?,
    )?;
    let list = choice.node(0).ok_or("missing List segment")?;
    let table = choice.node(1).ok_or("missing Table segment")?;
    tree.focus(list)?;
    send(&mut tree, key(KeyCode::ArrowRight, true))?;
    assert_eq!(selection.get(), 1);
    choice.set_selected(&mut tree, 1)?;
    assert_eq!(tree.focused_node(), Some(table));
    assert_eq!(
        tree.semantics(table)?.ok_or("missing segment")?.value,
        "true"
    );

    let search = TextField::new("before");
    let id = tree.mount(tree.root(), search.view(system, "Search", typography()))?;
    search
        .editor()
        .borrow_mut()
        .replace(0..6, "after", Duration::ZERO)?;
    search.sync_value(&mut tree)?;
    let semantics = tree.semantics(id)?.ok_or("missing search semantics")?;
    assert_eq!(
        (semantics.role.as_str(), semantics.value.as_str()),
        ("textbox", "after")
    );
    assert!(!semantics.actions.is_empty());
    Ok(())
}

#[test]
fn thumb_capture_maps_fractional_travel_to_viewport_offset()
-> Result<(), Box<dyn std::error::Error>> {
    let mut tree = UiTree::new()?;
    tree.set_theme(Theme::from_toml(CONTROL_THEME)?)?;
    let scrollbar = Scrollbar::default();
    let last = Rc::new(Cell::new(0.));
    let proposed = last.clone();
    let track = tree.mount(
        tree.root(),
        scrollbar.view(ScrollbarAxis::Vertical, "Offset", move |offset| {
            proposed.set(offset)
        })?,
    )?;
    tree.layout(VIEWPORT)?;
    scrollbar.update(
        &mut tree,
        ScrollbarAxis::Vertical,
        ScrollbarMetrics {
            viewport: 160.,
            maximum: 640.,
            offset: 0.,
        },
    )?;
    tree.layout(VIEWPORT)?;
    let bounds = tree.bounds(track)?;
    let length = bounds.size().height();
    let thumb = tree
        .bounds(scrollbar.thumb_node().ok_or("missing thumb")?)?
        .size()
        .height();
    let start = point(bounds, Point::new(6., 4.));
    let middle = point(bounds, Point::new(6., 4. + (length - thumb) * 0.5));
    send(&mut tree, pointer(start, true))?;
    assert_eq!(tree.capture_holder(), Some(track));
    send(
        &mut tree,
        WindowEvent::PointerMoved {
            position: middle,
            timestamp: Duration::ZERO,
        },
    )?;
    assert!((last.get() - 320.).abs() < 0.001, "proposed {}", last.get());
    send(&mut tree, pointer(middle, false))?;
    assert_eq!(tree.capture_holder(), None);
    send(&mut tree, key(KeyCode::End, true))?;
    assert_eq!(last.get(), 640.);
    Ok(())
}

#[test]
fn remounted_scrollbar_uses_unchanged_viewport_metrics() -> Result<(), Box<dyn std::error::Error>> {
    let mut tree = UiTree::new()?;
    let scrollbar = Scrollbar::default();
    let axis = ScrollbarAxis::Vertical;
    let metrics = ScrollbarMetrics {
        viewport: 160.,
        maximum: 640.,
        offset: 320.,
    };
    let track = tree.mount(tree.root(), scrollbar.view(axis, "Offset", |_| {})?)?;
    tree.layout(VIEWPORT)?;
    scrollbar.update(&mut tree, axis, metrics)?;
    tree.remove(track)?;

    let track = tree.mount(tree.root(), scrollbar.view(axis, "Offset", |_| {})?)?;
    tree.layout(VIEWPORT)?;
    scrollbar.update(&mut tree, axis, metrics)?;
    tree.layout(VIEWPORT)?;
    assert!(!tree.interaction_state(track)?.disabled);
    assert_eq!(
        tree.semantics(track)?.ok_or("missing scrollbar")?.value,
        "50%"
    );
    let thumb = scrollbar.thumb_node().ok_or("missing thumb")?;
    assert!(tree.bounds(thumb)?.size().height() > 0.);
    Ok(())
}
