//! Shared oracles for coverage-guided fuzzing and deterministic stable regressions.
#![allow(dead_code)]
use oxiview_text::{EditMode, Editor, NativeEdit, Selection};
use std::time::Duration;

#[path = "../crates/oxiview-platform-macos/src/macos/utf16.rs"]
mod utf16;

const TOKENS: [&str; 8] = ["e\u{301}", "👩‍💻", "مرحبا", "\n", "书", "**", "\r\n", ""];

fn invariants(editor: &Editor) {
    let selection = editor.selection();
    assert!(editor.source().get(selection.range()).is_some());
    let p = editor.presentation();
    assert!(p.text.get(p.selection.range()).is_some());
    if let Some(mark) = p.marked {
        assert!(p.text.get(mark).is_some())
    }
}

pub fn editing(input: &[u8]) {
    let mut editor = Editor::new("A👩‍💻 文 e\u{301} مرحبا", EditMode::MultiLine);
    for (i, step) in input.chunks_exact(3).take(160).enumerate() {
        let [op, a, b] = [step[0], step[1], step[2]];
        let before = (
            editor.source().to_owned(),
            editor.selection(),
            editor.committed_revision(),
        );
        let now = Duration::from_millis(i as u64 * (u64::from(a) + 1));
        let text = TOKENS[usize::from(b) % TOKENS.len()];
        let range = usize::from(a)..usize::from(b);
        let change = match op % 8 {
            0 => editor.replace(range, text, now),
            1 => editor.set_selection(Selection::new(usize::from(a), usize::from(b))),
            2 => editor.insert(text, now),
            3 => editor.delete_backward(a % 2 == 0, now),
            4 => editor.delete_forward(a % 2 == 0, now),
            5 => editor.undo(),
            6 => editor.redo(),
            _ => {
                let end = editor.source().len();
                let r = editor.replace(end..end, text, now);
                if r == Ok(true) {
                    let after = editor.source().to_owned();
                    assert_eq!(editor.undo(), Ok(true));
                    assert_eq!(editor.source(), before.0);
                    assert_eq!(editor.redo(), Ok(true));
                    assert_eq!(editor.source(), after);
                }
                r
            }
        };
        if change.is_err() {
            assert_eq!(
                (
                    editor.source(),
                    editor.selection(),
                    editor.committed_revision()
                ),
                (before.0.as_str(), before.1, before.2)
            )
        }
        invariants(&editor);
    }
}

pub fn input_session(input: &[u8]) {
    let mut editor = Editor::new("A🎵文", EditMode::MultiLine);
    let mut session = 1;
    let _ = editor.begin_session(session);
    for (i, step) in input.chunks_exact(4).take(120).enumerate() {
        let [op, a, b, d] = [step[0], step[1], step[2], step[3]];
        let kind = op % 7;
        let text = TOKENS[usize::from(d) % TOKENS.len()];
        let displayed = editor.presentation().text;
        let native_start = usize::from(a);
        let native_len = if d == 255 { usize::MAX } else { usize::from(b) };
        let clipped = op & 1 != 0;
        let translated = utf16::bytes(displayed, native_start, native_len, clipped);
        if let Some(range) = &translated {
            let encoded = utf16::units(displayed, range.clone());
            assert_eq!(encoded.map(|(start, _)| start), Some(native_start));
            if let Some((start, len)) = encoded {
                assert!(len <= native_len && (clipped || len == native_len));
                assert_eq!(
                    utf16::bytes(displayed, start, len, false).as_ref(),
                    Some(range)
                );
            }
        }
        let before = (
            editor.source().to_owned(),
            editor.selection(),
            editor.committed_revision(),
            editor.presentation().text.to_owned(),
        );
        if kind == 6 {
            let _ = editor.end_session(session);
            assert_eq!(editor.source(), before.0);
            assert_eq!(editor.selection(), before.1);
            session += 1;
            let _ = editor.begin_session(session);
        } else {
            let range = if op & 16 == 0 {
                None
            } else {
                Some(usize::from(a)..usize::from(b))
            };
            let edit = match kind {
                0 | 1 => NativeEdit::Mark {
                    text: text.into(),
                    replacement: range,
                    selection: if op & 32 == 0 {
                        text.len()..text.len()
                    } else {
                        usize::from(b)..usize::from(d)
                    },
                },
                2 => NativeEdit::Commit {
                    text: text.into(),
                    replacement: range,
                },
                3 => NativeEdit::Typing {
                    text: text.into(),
                    replacement: range,
                },
                _ => NativeEdit::Cancel,
            };
            let result = editor.apply_native(
                if op & 64 == 0 { session } else { session - 1 },
                editor.input_revision() + u64::from(op & 128 != 0),
                editor.acknowledged() + u64::from(op & 8 == 0),
                edit,
                Duration::from_millis(i as u64),
            );
            if result.is_err() {
                assert_eq!(
                    (
                        editor.source(),
                        editor.selection(),
                        editor.committed_revision(),
                        editor.presentation().text
                    ),
                    (before.0.as_str(), before.1, before.2, before.3.as_str())
                );
            } else if matches!(kind, 0 | 1 | 4 | 5) {
                assert_eq!(
                    (editor.source(), editor.committed_revision()),
                    (before.0.as_str(), before.2)
                );
            }
        }
        invariants(&editor);
    }
}
