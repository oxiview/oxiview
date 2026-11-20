//! Owned model and Parley geometry tests with pinned Noto fonts.
#![allow(clippy::unwrap_used)]

use oxiview_base::{Affine, Point};
use oxiview_text::{
    Affinity, Caret, EditCommand, EditError, EditMode, Editor, NativeEdit, Selection, TextStyle,
    TextSystem,
};
use std::time::Duration;

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

#[test]
fn deletion_treats_combining_marks_joiners_flags_and_crlf_as_graphemes() {
    let mut empty = Editor::new("", EditMode::MultiLine);
    assert!(!empty.delete_backward(false, ms(0)).unwrap());
    assert!(!empty.delete_forward(false, ms(0)).unwrap());

    for grapheme in ["e\u{301}", "👩‍💻", "🇨🇳", "\r\n", "क्\u{200d}ष"] {
        let mut editor = Editor::new(format!("A{grapheme}B"), EditMode::MultiLine);
        editor.set_selection(Selection::caret(1)).unwrap();
        assert!(editor.delete_forward(false, ms(1)).unwrap(), "{grapheme:?}");
        assert_eq!(editor.source(), "AB");
        assert!(editor.undo().unwrap());
        assert_eq!(editor.selection(), Selection::caret(1));
        editor
            .set_selection(Selection::caret(1 + grapheme.len()))
            .unwrap();
        assert!(editor.delete_backward(false, ms(2)).unwrap());
        assert_eq!(editor.source(), "AB");
    }

    let mut editor = Editor::new("A e\u{301} B", EditMode::MultiLine);
    editor.set_selection(Selection::caret(3)).unwrap(); // Between e and its accent.
    editor.delete_backward(false, ms(0)).unwrap();
    assert_eq!(editor.source(), "A  B");
}

#[test]
fn undo_delimits_selection_replacement_nonadjacent_typing_and_injected_time() {
    let mut editor = Editor::new("old", EditMode::MultiLine);
    let original_selection = Selection::new(3, 0);
    editor.set_selection(original_selection).unwrap();
    let original_state = editor.document_state();
    editor.insert("n", ms(100)).unwrap();
    let replacement_state = editor.document_state();
    editor.insert("e", ms(200)).unwrap();
    editor.insert("w", ms(700)).unwrap();
    let saved_state = editor.document_state();
    editor.insert("!", ms(1201)).unwrap();
    assert_eq!(editor.source(), "new!");

    assert!(editor.undo().unwrap());
    assert_eq!(
        (editor.source(), editor.document_state()),
        ("new", saved_state)
    );
    assert!(editor.undo().unwrap());
    assert_eq!(
        (editor.source(), editor.document_state()),
        ("n", replacement_state)
    );
    assert!(editor.undo().unwrap());
    assert_eq!(
        (editor.source(), editor.document_state()),
        ("old", original_state)
    );
    assert_eq!(editor.selection(), original_selection);

    let revision = editor.committed_revision();
    assert!(editor.redo().unwrap());
    assert_eq!(
        (editor.source(), editor.document_state()),
        ("n", replacement_state)
    );
    assert!(editor.committed_revision() > revision);
    editor.set_selection(Selection::caret(0)).unwrap();
    editor.insert("O", ms(1202)).unwrap();
    assert_eq!(editor.source(), "On");
    assert!(!editor.can_redo());

    editor.set_selection(Selection::caret(2)).unwrap();
    editor.delete_backward(false, ms(2000)).unwrap();
    editor.delete_backward(false, ms(2100)).unwrap();
    assert_eq!(editor.source(), "");
    editor.undo().unwrap();
    assert_eq!(editor.source(), "On");
    assert_eq!(editor.selection(), Selection::caret(2));

    editor.set_selection(Selection::caret(2)).unwrap();
    editor.insert("e", ms(3000)).unwrap();
    editor.insert("!", ms(2999)).unwrap();
    editor.undo().unwrap();
    assert_eq!(editor.source(), "One");
}

#[test]
fn native_callbacks_validate_utf8_session_and_order_and_restore_the_original_selection() {
    let original = "😀old title";
    let mut editor = Editor::new(original, EditMode::MultiLine);
    let original_selection = Selection::new(7, 4);
    editor.set_selection(original_selection).unwrap();
    editor.begin_session(11).unwrap();
    let revision = editor.input_revision();
    let committed = editor.committed_revision();

    let invalid = NativeEdit::Mark {
        text: "a".into(),
        replacement: Some(1..4),
        selection: 0..0,
    };
    let before = editor.presentation().clone();
    let old_presentation = (before.text.to_owned(), before.selection, before.revision);
    assert_eq!(
        editor.apply_native(11, revision, 1, invalid, ms(1)),
        Err(EditError::InvalidRange)
    );
    assert_eq!(editor.acknowledged(), 0);
    let after = editor.presentation();
    assert_eq!(
        old_presentation,
        (after.text.to_owned(), after.selection, after.revision)
    );

    let mark = NativeEdit::Mark {
        text: "zhong".into(),
        replacement: None,
        selection: 3..5,
    };
    assert!(
        editor
            .apply_native(11, revision, 1, mark.clone(), ms(2))
            .unwrap()
    );
    assert_eq!(editor.source(), original);
    assert_eq!(editor.selection(), original_selection);
    assert_eq!(editor.committed_revision(), committed);
    assert!(!editor.can_undo());
    assert_eq!(editor.presentation().text, "😀zhong title");
    assert_eq!(editor.presentation().selection.range(), 7..9);
    assert_eq!(editor.presentation().marked, Some(4..9));
    assert_eq!(editor.input_revision(), revision);
    assert_eq!(
        editor.apply_native(11, revision, 1, mark.clone(), ms(3)),
        Err(EditError::StaleInput)
    );

    editor
        .apply_native(
            11,
            revision,
            2,
            NativeEdit::Mark {
                text: "中".into(),
                replacement: Some(4..7),
                selection: 3..3,
            },
            ms(4),
        )
        .unwrap();
    assert_eq!(editor.presentation().text, "😀中ng title");
    assert_eq!(editor.presentation().marked, Some(4..9));
    editor
        .apply_native(
            11,
            revision,
            3,
            NativeEdit::Commit {
                text: "中文".into(),
                replacement: Some(4..10),
            },
            ms(5),
        )
        .unwrap();
    assert_eq!(editor.source(), "😀中文title");
    assert_eq!(editor.presentation().marked, None);
    assert_eq!(editor.committed_revision(), committed + 1);
    assert_eq!(editor.selection().range(), 10..10);
    assert_eq!(editor.acknowledged(), 3);

    let before_mark = editor.selection();
    editor
        .apply_native(11, revision, 4, mark.clone(), ms(6))
        .unwrap();
    assert!(editor.end_session(11).unwrap());
    assert_eq!(editor.selection(), before_mark);
    assert_eq!(editor.source(), "😀中文title");
    assert_eq!(editor.presentation().text, editor.source());
    assert_eq!(editor.committed_revision(), committed + 1);

    editor.begin_session(12).unwrap();
    assert_eq!(
        editor.apply_native(11, revision, 5, mark.clone(), ms(7)),
        Err(EditError::StaleInput)
    );
    editor.undo().unwrap();
    assert_eq!(editor.source(), original);
    assert_eq!(editor.selection(), original_selection);
    assert!(!editor.can_undo());
    assert!(editor.redo().unwrap());
    assert_eq!(editor.source(), "😀中文title");
    assert_eq!(
        editor.apply_native(12, revision, 1, mark, ms(8)),
        Err(EditError::StaleInput)
    );
}

#[test]
fn single_line_normalizes_native_mark_selection_and_multiline_retains_crlf() {
    let mut editor = Editor::new("a\r\nb\u{2028}c", EditMode::SingleLine);
    assert_eq!(editor.source(), "a b c");
    editor.set_selection(Selection::new(0, 5)).unwrap();
    let old_selection = editor.selection();
    editor.begin_session(42).unwrap();
    let revision = editor.input_revision();
    editor
        .apply_native(
            42,
            revision,
            1,
            NativeEdit::Mark {
                text: "m\r\n😀".into(),
                replacement: None,
                selection: 3..7,
            },
            ms(1),
        )
        .unwrap();
    assert_eq!(editor.source(), "a b c");
    assert_eq!(editor.presentation().text, "m 😀");
    assert_eq!(editor.presentation().selection.range(), 2..6);
    assert_eq!(editor.presentation().marked, Some(0..6));
    assert!(editor.input_revision() > revision);
    editor
        .apply_native(42, editor.input_revision(), 2, NativeEdit::Cancel, ms(2))
        .unwrap();
    assert_eq!(editor.selection(), old_selection);
    assert_eq!(editor.presentation().text, "a b c");
    assert!(
        !editor
            .apply_command(EditCommand::InsertNewline, ms(2))
            .unwrap()
    );

    let mut source = Editor::new("one\r\ntwo", EditMode::MultiLine);
    source.set_selection(Selection::caret(8)).unwrap();
    source
        .apply_command(EditCommand::InsertNewline, ms(1))
        .unwrap();
    assert_eq!(source.source(), "one\r\ntwo\r\n");
}

#[test]
fn fresh_revision_can_skip_a_discarded_sequence_and_identical_reset_retires_old_input() {
    let mut editor = Editor::new("unchanged", EditMode::MultiLine);
    editor.begin_session(1).unwrap();
    let revision = editor.input_revision();
    let typing = NativeEdit::Typing {
        text: "x".into(),
        replacement: None,
    };
    assert!(!editor.reset("unchanged").unwrap());
    assert_eq!(editor.committed_revision(), 0);
    assert_eq!(
        editor.apply_native(1, revision, 1, typing.clone(), ms(1)),
        Err(EditError::StaleInput)
    );
    let revision = editor.input_revision();
    assert!(
        editor
            .apply_native(1, revision, 2, typing.clone(), ms(2))
            .unwrap()
    );
    assert!(editor.apply_native(1, revision, 3, typing, ms(3)).unwrap());
    assert_eq!(editor.acknowledged(), 3);
    assert_eq!(editor.source(), "xxunchanged");
    editor.undo().unwrap();
    assert_eq!(editor.source(), "unchanged");
}

#[test]
fn word_deletion_uses_unicode_and_an_explicit_batch_undo_restores_selection() {
    let mut editor = Editor::new("one العربية two", EditMode::SingleLine);
    editor
        .set_selection(Selection::caret("one العربية ".len()))
        .unwrap();
    editor.delete_backward(true, ms(1)).unwrap();
    assert_eq!(editor.source(), "one two");
    editor.undo().unwrap();
    let selected = Selection::new(editor.source().len(), 0);
    editor.set_selection(selected).unwrap();
    editor
        .replace(selected.range(), "1 العربية 2", ms(2))
        .unwrap();
    editor.undo().unwrap();
    assert_eq!(editor.selection(), selected);
    assert_eq!(editor.source(), "one العربية two");
}

fn fonts() -> (TextSystem, TextStyle) {
    let mut system = TextSystem::without_system_fonts();
    for bytes in [
        include_bytes!("fonts/NotoSans.ttf").as_slice(),
        include_bytes!("fonts/NotoSansArabic.ttf").as_slice(),
    ] {
        system.register_font(bytes.to_vec()).unwrap();
    }
    let style = TextStyle {
        family: Some("Noto Sans, Noto Sans Arabic".into()),
        ..TextStyle::default()
    };
    (system, style)
}

#[test]
fn wrapped_line_affinities_empty_lines_and_visible_draws_use_one_layout() {
    let (mut text, style) = fonts();
    let source = "office hour gathers everyone around e\u{301}lan.\r\n\n";
    let layout = text.layout_editable(source, &style, Some(108.0)).unwrap();
    let rects = layout.visible_range_rects(0.0..layout.size().height());
    let [_, first_top, _, first_bottom] = rects[0].1.edges();
    let following_line = rects
        .iter()
        .find(|(_, rect)| rect.edges()[1] >= first_bottom)
        .unwrap();
    let join = following_line.0.start;
    let before_wrap = Caret::with_affinity(join, Affinity::Upstream);
    let after_wrap = Caret::new(join);
    assert_eq!(
        layout.caret_rect(before_wrap).unwrap().origin().y(),
        first_top
    );
    assert!(layout.caret_rect(after_wrap).unwrap().origin().y() > first_top);
    assert_eq!(layout.move_visual(before_wrap, true), after_wrap);
    assert_eq!(layout.move_visual(after_wrap, false), before_wrap);

    let end = layout.caret_rect(Caret::new(source.len())).unwrap();
    assert!(end.origin().y() > rects.last().unwrap().1.origin().y());
    let mid = Point::new(1.0, end.origin().y() + end.size().height() * 0.5);
    assert_eq!(layout.hit_test(mid).offset, source.len());
    let glyphs = text
        .editable_glyph_draws(
            &layout,
            Point::default(),
            Affine::IDENTITY,
            first_top..first_bottom,
        )
        .unwrap();
    assert!(!glyphs.is_empty());

    let empty = text.layout_editable("", &style, None).unwrap();
    assert_eq!(empty.line_count(), 1);
    assert!(empty.caret_rect(Caret::new(0)).unwrap().size().height() > 0.0);
    assert_eq!(empty.hit_test(Point::new(123.0, 456.0)).offset, 0);
    assert_eq!(layout.first_rect(0..source.len()).unwrap().0.end, join);
}

#[test]
fn mixed_direction_geometry_round_trips_and_visual_arrows_cross_arabic_in_reverse_logical_order() {
    let (mut text, style) = fonts();
    let source = "abc العربية def e\u{301} office";
    let layout = text.layout_editable(source, &style, Some(340.0)).unwrap();
    let rtl_start = source.find("العربية").unwrap();
    let rtl_end = rtl_start + "العربية".len();
    let rtl = layout.visible_range_rects(0.0..layout.size().height());
    let rtl: Vec<_> = rtl
        .into_iter()
        .filter(|(range, _)| (rtl_start..rtl_end).contains(&range.start))
        .collect();
    assert!(rtl.len() >= 6);
    assert!(rtl[0].1.origin().x() > rtl[1].1.origin().x());

    for (range, rect) in &rtl {
        let [left, top, right, bottom] = rect.edges();
        let y = (top + bottom) * 0.5;
        for (point, expected_offset, expected_x) in [
            (Point::new(left + (right - left) * 0.2, y), range.end, left),
            (
                Point::new(left + (right - left) * 0.8, y),
                range.start,
                right,
            ),
        ] {
            let caret = layout.hit_test(point);
            assert_eq!(caret.offset, expected_offset);
            assert!((layout.caret_rect(caret).unwrap().origin().x() - expected_x).abs() < 0.001);
        }
    }
    let first_visually = &rtl[rtl.len() - 1];
    let caret = Caret::with_affinity(first_visually.0.end, Affinity::Upstream);
    let next = layout.move_visual(caret, true);
    assert_eq!(next.offset, first_visually.0.start);
    assert!(
        layout.caret_rect(next).unwrap().origin().x()
            > layout.caret_rect(caret).unwrap().origin().x()
    );
    let next_word = layout.move_word(Caret::new(0), true);
    assert_eq!(next_word.offset, 3);

    let split = layout.selection_rects(0..rtl[1].0.end);
    assert_eq!(split.len(), 2);
    let combining = source.find("e\u{301}").unwrap();
    let before_accent = layout.caret_rect(Caret::new(combining)).unwrap();
    assert_eq!(
        layout.caret_rect(Caret::new(combining + 1)),
        Some(before_accent)
    );
}
