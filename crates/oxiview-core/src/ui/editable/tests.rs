#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use oxiview_base::Color;
use oxiview_platform::{KeyCode, KeyModifiers, LogicalKey, TextEditingCommand, WindowEvent};
use oxiview_scene::{Brush, Clip, NodeKind, Primitive};
use oxiview_style::{Length, Position, Style};

fn fixture() -> (Rc<RefCell<TextSystem>>, TextStyle) {
    let mut system = TextSystem::without_system_fonts();
    for font in [
        include_bytes!("../../../../oxiview-text/tests/fonts/NotoSans.ttf").as_slice(),
        include_bytes!("../../../../oxiview-text/tests/fonts/NotoSansArabic.ttf").as_slice(),
    ] {
        system.register_font(font.to_vec()).unwrap();
    }
    let style = TextStyle {
        family: Some("Noto Sans, Noto Sans Arabic".into()),
        color: Color::from_rgba8(36, 42, 51, 255),
        ..TextStyle::default()
    };
    (Rc::new(RefCell::new(system)), style)
}

fn viewport() -> Size {
    Size::new(320.0, 220.0)
}

fn field(x: f64, y: f64, width: f64, height: f64) -> Style {
    let mut style = Style::default();
    style.layout.position = Position::Absolute;
    style.layout.inset.left = Length::Points(x);
    style.layout.inset.top = Length::Points(y);
    style.layout.width = Length::Points(width);
    style.layout.height = Length::Points(height);
    style.layout.padding.left = Length::Points(6.0);
    style.layout.padding.top = Length::Points(4.0);
    style.layout.padding.right = Length::Points(6.0);
    style.layout.padding.bottom = Length::Points(4.0);
    style
}

fn click(tree: &mut UiTree, point: Point) {
    for pressed in [true, false] {
        tree.dispatch(
            &WindowEvent::PointerButton {
                button: PointerButton::Primary,
                pressed,
                position: point,
                timestamp: Duration::from_millis(10),
            },
            viewport(),
            |_, _, _| {},
        )
        .unwrap();
    }
}

fn key(tree: &mut UiTree, code: KeyCode, modifiers: KeyModifiers) -> bool {
    let mut handled = false;
    for pressed in [true, false] {
        let event = WindowEvent::Key {
            code,
            logical: LogicalKey::Named(code),
            modifiers,
            pressed,
            repeat: false,
            timestamp: Duration::from_millis(20),
        };
        handled |= tree
            .dispatch(&event, viewport(), |_, _, _| {})
            .unwrap()
            .key_handled;
    }
    handled
}

fn snapshot(tree: &mut UiTree) -> EditorWindowState {
    tree.focused_editor_state(viewport()).unwrap().unwrap()
}

fn native(state: &EditorWindowState, sequence: u64, edit: TextInputEdit) -> TextInputEvent {
    TextInputEvent {
        session: TextInputSession(state.session),
        revision: state.input_revision,
        sequence,
        edit,
        timestamp: Duration::from_millis(sequence),
    }
}

#[test]
fn retained_pointer_focus_cancels_outgoing_preedit_after_ordered_commits() {
    let (fonts, style) = fixture();
    let mut tree = UiTree::new().unwrap();
    let source = Editable::new("old heading العربية", EditMode::MultiLine);
    let find = Editable::new("", EditMode::SingleLine);
    let source_id = tree
        .mount(
            tree.root(),
            editable_text(fonts.clone(), source.clone(), style.clone())
                .style(field(24.0, 40.0, 132.0, 80.0)),
        )
        .unwrap();
    let find_id = tree
        .mount(
            tree.root(),
            editable_text(fonts, find.clone(), style).style(field(24.0, 138.0, 220.0, 32.0)),
        )
        .unwrap();

    let source_model = source.editor();
    let original_selection = Selection::new(3, 0);
    source_model
        .borrow_mut()
        .set_selection(original_selection)
        .unwrap();
    tree.focus(source_id).unwrap();
    let initial = snapshot(&mut tree);
    let committed = native(
        &initial,
        1,
        TextInputEdit::Commit {
            range: 0..3,
            text: "new".into(),
            kind: TextCommitKind::Composition,
        },
    );
    assert!(tree.apply_text_input_event(committed, viewport()).unwrap());
    let mark = native(
        &initial,
        2,
        TextInputEdit::SetMarked {
            range: 3..3,
            text: "shi".into(),
            selection: 3..3,
        },
    );
    assert!(tree.apply_text_input_event(mark, viewport()).unwrap());
    let marked = snapshot(&mut tree);
    assert_eq!(marked.text, "newshi heading العربية");
    assert_eq!(marked.marked, Some(3..6));
    assert_eq!(marked.acknowledged, 2);
    assert_eq!(source_model.borrow().source(), "new heading العربية");
    assert_eq!(source_model.borrow().committed_revision(), 1);

    click(&mut tree, Point::new(40.0, 151.0));
    assert_eq!(tree.focused_node(), Some(find_id));
    assert_eq!(source_model.borrow().presentation().selection.range(), 3..3);
    assert!(source_model.borrow().presentation().marked.is_none());
    let find_state = snapshot(&mut tree);
    assert_ne!(find_state.session, marked.session);
    let late = native(
        &marked,
        3,
        TextInputEdit::Commit {
            range: 3..6,
            text: "是".into(),
            kind: TextCommitKind::Composition,
        },
    );
    assert_eq!(
        tree.apply_text_input_event(late, viewport()),
        Err(EditError::StaleInput.into())
    );

    let first = native(
        &find_state,
        1,
        TextInputEdit::Commit {
            range: 0..0,
            text: "n".into(),
            kind: TextCommitKind::Typing,
        },
    );
    tree.apply_text_input_event(first, viewport()).unwrap();
    assert_eq!(find.editor().borrow().source(), "n");
    assert!(!key(&mut tree, KeyCode::Enter, KeyModifiers::default()));
    assert_eq!(find.editor().borrow().source(), "n");

    tree.focus(source_id).unwrap();
    assert!(tree.invoke_action(source_id, "editor.undo").unwrap());
    assert_eq!(source_model.borrow().source(), "old heading العربية");
    assert_eq!(source_model.borrow().selection(), original_selection);
    assert_eq!(find.editor().borrow().source(), "n");
}

#[test]
fn a_busy_incoming_model_preserves_the_outgoing_session_and_mark() {
    let (fonts, style) = fixture();
    let mut tree = UiTree::new().unwrap();
    let source = Editable::new("write here", EditMode::MultiLine);
    let search = Editable::new("", EditMode::SingleLine);
    let source_id = tree
        .mount(
            tree.root(),
            editable_text(fonts.clone(), source, style.clone())
                .style(field(12.0, 12.0, 132.0, 80.0)),
        )
        .unwrap();
    let search_id = tree
        .mount(
            tree.root(),
            editable_text(fonts, search.clone(), style).style(field(12.0, 102.0, 132.0, 28.0)),
        )
        .unwrap();
    tree.focus(source_id).unwrap();
    let state = snapshot(&mut tree);
    tree.apply_text_input_event(
        native(
            &state,
            1,
            TextInputEdit::SetMarked {
                range: 0..0,
                text: "a".into(),
                selection: 1..1,
            },
        ),
        viewport(),
    )
    .unwrap();

    let model = search.editor();
    let reading = model.borrow();
    assert_eq!(tree.focus(search_id), Err(UiError::HandlerBusy));
    let still_focused = snapshot(&mut tree);
    assert_eq!(still_focused.session, state.session);
    assert_eq!(still_focused.marked, Some(0..1));
    drop(reading);
    tree.focus(search_id).unwrap();
    assert_ne!(snapshot(&mut tree).session, state.session);
}

#[test]
fn primary_drag_crosses_wrapped_lines_and_shift_extends_the_same_anchor() {
    let (fonts, style) = fixture();
    let mut tree = UiTree::new().unwrap();
    let control = Editable::new(
        "first line mixed العربية here, then another line follows",
        EditMode::MultiLine,
    );
    let id = tree
        .mount(
            tree.root(),
            editable_text(fonts, control.clone(), style).style(field(28.0, 40.0, 126.0, 100.0)),
        )
        .unwrap();
    let from = Point::new(35.0, 50.0);
    let to = Point::new(115.0, 82.0);
    tree.dispatch(
        &WindowEvent::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
            position: from,
            timestamp: Duration::ZERO,
        },
        viewport(),
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(tree.capture_holder(), Some(id));
    let result = tree
        .dispatch(
            &WindowEvent::PointerMoved {
                position: to,
                timestamp: Duration::from_millis(1),
            },
            viewport(),
            |_, _, _| {},
        )
        .unwrap();
    assert!(result.changed);
    tree.dispatch(
        &WindowEvent::PointerButton {
            button: PointerButton::Primary,
            pressed: false,
            position: to,
            timestamp: Duration::from_millis(2),
        },
        viewport(),
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(tree.capture_holder(), None);

    let model = control.editor();
    let dragged = model.borrow().selection();
    assert_eq!(dragged.anchor.offset, 0);
    assert!(dragged.active.offset > "first line".len());
    let shift = KeyModifiers {
        shift: true,
        ..Default::default()
    };
    assert!(key(&mut tree, KeyCode::ArrowRight, shift));
    let extended = model.borrow().selection();
    assert_eq!(extended.anchor, dragged.anchor);
    assert_ne!(extended.active, dragged.active);
    assert!(key(&mut tree, KeyCode::Backspace, KeyModifiers::default()));
    assert!(model.borrow().selection().is_empty());
    assert!(tree.invoke_action(id, "editor.undo").unwrap());
    assert_eq!(model.borrow().selection(), extended);
}

#[test]
fn vertical_actions_preserve_the_column_and_multiline_enter_preserves_the_line_ending() {
    let (fonts, style) = fixture();
    let mut tree = UiTree::new().unwrap();
    let control = Editable::new(
        "remember the column\r\nx\r\nremember the column",
        EditMode::MultiLine,
    );
    let id = tree
        .mount(
            tree.root(),
            editable_text(fonts, control.clone(), style).style(field(24.0, 36.0, 268.0, 104.0)),
        )
        .unwrap();
    let model = control.editor();
    model
        .borrow_mut()
        .set_selection(Selection::caret(12))
        .unwrap();
    tree.focus(id).unwrap();
    let first = snapshot(&mut tree).caret.origin();
    assert!(key(&mut tree, KeyCode::ArrowDown, KeyModifiers::default()));
    let second = snapshot(&mut tree).caret.origin();
    assert!(second.y() > first.y());
    assert!(second.x() + 10.0 < first.x());

    let state = snapshot(&mut tree);
    let down = native(
        &state,
        1,
        TextInputEdit::Command(TextEditingCommand::MoveDown { select: false }),
    );
    assert!(tree.apply_text_input_event(down, viewport()).unwrap());
    let third = snapshot(&mut tree).caret.origin();
    assert!(third.y() > second.y());
    assert_eq!(first.x(), third.x());
    assert!(key(&mut tree, KeyCode::Enter, KeyModifiers::default()));
    assert!(model.borrow().source().contains("remember the\r\n column"));
}

#[test]
fn retained_paint_and_window_queries_share_the_clipped_viewport_and_caret() {
    let (fonts, style) = fixture();
    let mut tree = UiTree::new().unwrap();
    let source = std::iter::repeat_n("more text for the source pane\n", 12).collect::<String>();
    let control = Editable::new(&source, EditMode::MultiLine);
    let id = tree
        .mount(
            tree.root(),
            editable_text(fonts.clone(), control.clone(), style.clone())
                .style(field(24.0, 34.0, 192.0, 70.0)),
        )
        .unwrap();
    tree.focus(id).unwrap();
    let initial = snapshot(&mut tree);
    let model = control.editor();
    model
        .borrow_mut()
        .set_selection(Selection::caret(source.len()))
        .unwrap();
    assert!(tree.refresh_editor(id).unwrap());
    let end = snapshot(&mut tree);
    assert_eq!(initial.viewport, end.viewport);
    let [x0, y0, x1, y1] = end.viewport.edges();
    let [cx0, cy0, cx1, cy1] = end.caret.edges();
    assert!(x0 <= cx0 && cx1 <= x1 && y0 <= cy0 && cy1 <= y1);
    let caret_hit = Point::new(cx0, (cy0 + cy1) * 0.5);
    assert_eq!(end.character_index_at_point(caret_hit), source.len());
    assert_eq!(
        end.first_rect(source.len()..source.len()),
        Some((source.len()..source.len(), end.caret))
    );

    let mark = native(
        &end,
        1,
        TextInputEdit::SetMarked {
            range: source.len()..source.len(),
            text: "marked".into(),
            selection: 6..6,
        },
    );
    tree.apply_text_input_event(mark, viewport()).unwrap();
    let live = snapshot(&mut tree);
    let platform = tree
        .focused_text_input_snapshot(viewport())
        .unwrap()
        .unwrap();
    assert!(platform.is_valid());
    assert_eq!(platform.caret, live.caret);
    assert!(
        platform
            .geometry
            .iter()
            .any(|fragment| fragment.range.start == source.len())
    );

    let painted = tree.paint(viewport(), 1.5).unwrap();
    let editor_clip = walk(&painted)
        .into_iter()
        .find(|&node| {
            painted.scene.content(node).unwrap() == &NodeKind::Clip(Clip::Rect(live.viewport))
        })
        .unwrap();
    let draws = painted.scene.children(Some(editor_clip)).unwrap();
    assert_eq!(draws.len(), 4);
    assert!(!painted.scene.children(Some(draws[1])).unwrap().is_empty());
    let underline = painted.scene.children(Some(draws[2])).unwrap();
    assert!(!underline.is_empty());
    assert!(matches!(painted.scene.content(underline[0]).unwrap(),
        NodeKind::Draw(Primitive::Fill(rect, _, Brush::Solid(color)))
        if rect.size().height() == 1.0 && *color == style.color));
    assert!(matches!(painted.scene.content(draws[3]).unwrap(),
        NodeKind::Draw(Primitive::Fill(rect, _, Brush::Solid(color)))
        if *rect == live.caret && *color == style.color));

    let cache_before = fonts.borrow().cache_stats();
    assert_eq!(
        tree.advance_editor_caret(Duration::ZERO).unwrap(),
        (false, Some(BLINK_INTERVAL))
    );
    assert!(tree.advance_editor_caret(BLINK_INTERVAL).unwrap().0);
    let blink = tree.paint(viewport(), 1.5).unwrap();
    assert!(matches!(blink.scene.content(draws[3]).unwrap(),
        NodeKind::Draw(Primitive::Fill(rect, _, _)) if *rect == Rect::default()));
    assert_eq!(fonts.borrow().cache_stats(), cache_before);

    tree.dispatch(
        &WindowEvent::ActivationChanged {
            active: false,
            timestamp: BLINK_INTERVAL,
        },
        viewport(),
        |_, _, _| {},
    )
    .unwrap();
    assert!(model.borrow().presentation().marked.is_none());
    assert!(
        tree.focused_text_input_snapshot(viewport())
            .unwrap()
            .is_none()
    );
    for tick in 0..1000 {
        assert_eq!(
            tree.advance_editor_caret(Duration::from_secs(tick))
                .unwrap(),
            (false, None)
        );
    }
    tree.remove(id).unwrap();
    assert_eq!(control.node(), None);
}

fn walk(painted: &crate::PaintedScene) -> Vec<oxiview_scene::NodeId> {
    let mut nodes = Vec::new();
    let mut stack = vec![painted.root];
    while let Some(node) = stack.pop() {
        nodes.push(node);
        stack.extend(painted.scene.children(Some(node)).unwrap());
    }
    nodes
}

#[test]
fn focused_editor_queries_and_pointer_selection_follow_nested_scroll_translations() {
    let (fonts, style) = fixture();
    let mut tree = UiTree::new().unwrap();
    let outer = tree
        .append(tree.root(), field(10.0, 10.0, 180.0, 85.0))
        .unwrap();
    let inner = tree.append(outer, field(20.0, 15.0, 124.0, 63.0)).unwrap();
    let control = Editable::new("candidate location", EditMode::SingleLine);
    let editor = tree
        .mount(
            inner,
            editable_text(fonts, control.clone(), style).style(field(10.0, 16.0, 155.0, 40.0)),
        )
        .unwrap();
    tree.set_scroll_content_extent(outer, Size::new(240.0, 180.0))
        .unwrap();
    tree.set_scroll_content_extent(inner, Size::new(220.0, 160.0))
        .unwrap();
    tree.layout(viewport()).unwrap();

    // Pointer focus preserves the original position of an editor wider than its clip.
    click(&mut tree, Point::new(82.0, 54.0));
    let before = snapshot(&mut tree);
    assert_eq!(before.node, editor);
    assert!(before.selection.active.offset > 0);
    let mark = native(
        &before,
        1,
        TextInputEdit::SetMarked {
            range: before.selection.active.offset..before.selection.active.offset,
            text: "漢字".into(),
            selection: 0..0,
        },
    );
    assert!(tree.apply_text_input_event(mark, viewport()).unwrap());
    let marked = snapshot(&mut tree);
    let range = marked.selection.active.offset..marked.selection.active.offset;

    tree.set_scroll_offset(outer, Point::new(3.5, 11.25))
        .unwrap();
    tree.set_scroll_offset(inner, Point::new(2.25, 9.5))
        .unwrap();
    let after = snapshot(&mut tree);
    assert_eq!(after.session, marked.session);
    assert_eq!(after.input_revision, marked.input_revision);
    assert_eq!(after.marked, marked.marked);
    assert_eq!(after.selection, marked.selection);
    let x = marked.caret.origin().x() - 5.75;
    let y = marked.caret.origin().y() - 20.75;
    assert_eq!(after.caret.origin(), Point::new(x, y));
    assert_eq!(after.first_rect(range.clone()), Some((range, after.caret)));
    assert_eq!(
        after.viewport.edges()[2],
        tree.scroll_viewport_bounds(inner).unwrap().edges()[2],
    );
    let platform = tree
        .focused_text_input_snapshot(viewport())
        .unwrap()
        .unwrap();
    assert!(platform.is_valid());
    assert_eq!(platform.caret, after.caret);

    let commit = native(
        &after,
        2,
        TextInputEdit::Commit {
            range: after.marked.clone().unwrap(),
            text: "漢字".into(),
            kind: TextCommitKind::Composition,
        },
    );
    assert!(tree.apply_text_input_event(commit, viewport()).unwrap());
    let committed = snapshot(&mut tree);
    let point = Point::new(
        committed.caret.origin().x() - 12.0,
        committed.caret.origin().y() + committed.caret.size().height() / 2.0,
    );
    let index = committed.character_index_at_point(point);
    click(&mut tree, point);
    assert_eq!(control.editor().borrow().selection().active.offset, index);
}
