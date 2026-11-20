//! M6 fixed window-free workloads for the editing model and its cached geometry.
#![allow(clippy::expect_used)] // Invalid fixtures must fail the benchmark.

use oxiview_base::Point;
use oxiview_text::{EditMode, Editor, NativeEdit, Selection, TextStyle, TextSystem};
use std::time::Duration;

const MIXED: &str = "Résumé e\u{301} — العربية واللاتينية — 中文，👩‍💻.\n";
const WRAPPED: &str = "Editor state and caret placement — العربية";
const PREEDIT: [&str; 16] = [
    "z",
    "zh",
    "zho",
    "zhon",
    "zhong",
    "zhongw",
    "zhongwe",
    "zhongwen",
    "zhong wen",
    "zhong wen b",
    "zhong wen bi",
    "zhong wen bia",
    "zhong wen biao",
    "中文 biao",
    "中文标",
    "中文标题",
];

fn main() {
    divan::main();
}

#[divan::bench(sample_count = 80, sample_size = 128)]
fn replace_and_undo_4k(bencher: divan::Bencher) {
    let mut source = MIXED.repeat(4096 / MIXED.len());
    source.extend(std::iter::repeat_n(' ', 4096 - source.len()));
    assert_eq!(source.len(), 4096);
    let start = (4096 / MIXED.len() / 2) * MIXED.len();
    let range = start..start + MIXED.len();
    let mut editor = Editor::new(source.as_str(), EditMode::MultiLine);
    editor
        .set_selection(Selection::new(range.start, range.end))
        .expect("fixture range");

    bencher.bench_local(|| {
        let editor = divan::black_box(&mut editor);
        editor
            .replace(range.clone(), "New heading — 中文", Duration::ZERO)
            .expect("replace");
        editor.undo().expect("undo");
        divan::black_box(editor.document_state());
    });
    assert_eq!(editor.source(), source);
    assert_eq!(editor.selection().range(), range);
}

#[divan::bench(sample_count = 80, sample_size = 128)]
fn cached_wrapped_geometry_32_lines(bencher: divan::Bencher) {
    let mut fonts = TextSystem::without_system_fonts();
    for bytes in [
        include_bytes!("../crates/oxiview-text/tests/fonts/NotoSans.ttf").as_slice(),
        include_bytes!("../crates/oxiview-text/tests/fonts/NotoSansArabic.ttf").as_slice(),
    ] {
        fonts
            .register_font(bytes.to_vec())
            .expect("register pinned font");
    }
    let style = TextStyle {
        family: Some("Noto Sans, Noto Sans Arabic".into()),
        ..TextStyle::default()
    };
    let source = std::iter::repeat_n(WRAPPED, 16)
        .collect::<Vec<_>>()
        .join("\n");
    let layout = fonts
        .layout_editable(&source, &style, Some(185.0))
        .expect("shape fixture");
    assert_eq!(layout.line_count(), 32);
    let point = Point::new(72.0, layout.size().height() * 0.48);
    let range = 4..source.len() - "ية".len();
    assert!(layout.selection_rects(range.clone()).len() >= layout.line_count());

    bencher.bench_local(|| {
        let layout = divan::black_box(&layout);
        let caret = layout.hit_test(divan::black_box(point));
        divan::black_box(layout.caret_rect(caret));
        divan::black_box(layout.selection_rects(range.clone()));
    });
}

#[divan::bench(sample_count = 80, sample_size = 64)]
fn preedit_16_updates_then_commit(bencher: divan::Bencher) {
    bencher
        .with_inputs(|| {
            let mut editor = Editor::new("A Markdown heading", EditMode::MultiLine);
            editor
                .set_selection(Selection::new(2, 10))
                .expect("select fixture");
            editor.begin_session(1).expect("start session");
            editor
        })
        .bench_local_refs(|editor| {
            let revision = editor.input_revision();
            for (index, text) in PREEDIT.into_iter().enumerate() {
                editor
                    .apply_native(
                        1,
                        revision,
                        index as u64 + 1,
                        NativeEdit::Mark {
                            text: text.into(),
                            replacement: None,
                            selection: text.len()..text.len(),
                        },
                        Duration::from_millis(index as u64),
                    )
                    .expect("update preedit");
            }
            editor
                .apply_native(
                    1,
                    revision,
                    17,
                    NativeEdit::Commit {
                        text: "中文标题".into(),
                        replacement: None,
                    },
                    Duration::from_millis(16),
                )
                .expect("commit");
            assert_eq!(editor.committed_revision(), 1);
            divan::black_box(editor.document_state());
        });
}
