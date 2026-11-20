//! Pinned-font CPU specimen of selection, inline preedit and the rendered document.

use super::app::EditorApp;
use oxiview_core::{Application, FrameKind, FrameTiming, Scheduler, TextSystem};
use oxiview_platform::{
    PhysicalSize, TextInputEdit, TextInputEvent, WindowEvent, WindowWake, WindowWaker,
};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_scene::PixelSize;
use oxiview_testing::{CompareConfig, SnapshotHarness};
use std::{
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const SOURCE: &str = concat!(
    "# 山间书屋\n\n",
    "A shelf for $x_i$ and notes in مرحبا.\n\n",
    "```mermaid\nflowchart LR\n  A[Read] --> B[Write]\n```\n\n",
    "$$\n\\frac{a+b}{\\sqrt{c}}\n$$\n",
);
const SCALES: [(&str, f64); 3] = [
    ("m6-editor-1x", 1.),
    ("m6-editor-1_5x", 1.5),
    ("m6-editor-2x", 2.),
];

struct Wake;
impl WindowWake for Wake {
    fn wake(&self) -> Result<(), oxiview_platform::Error> {
        Ok(())
    }
}

#[test]
fn marked_selection_and_markdown_preview_at_three_scales() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fonts = TextSystem::without_system_fonts();
    fonts.register_font(
        include_bytes!("../../../crates/oxiview-text/tests/fonts/NotoSans.ttf").to_vec(),
    )?;
    fonts.register_font(
        include_bytes!("../../../crates/oxiview-text/tests/fonts/NotoSansArabic.ttf").to_vec(),
    )?;
    fonts.register_font(include_bytes!("../tests/fonts/EditingFixture.otf").to_vec())?;
    let mut app = EditorApp::with_text(None, SOURCE, fonts)?;
    let mut scheduler = Scheduler::default();
    let waker = WindowWaker::new(Arc::new(Wake));
    let start = Instant::now();
    while app.preview.submitted == 0 || app.preview.published != app.preview.submitted {
        app.event(&WindowEvent::Wake, &mut scheduler, &waker)?;
        if start.elapsed() > Duration::from_secs(20) {
            return Err("Preview did not finish".into());
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(app.preview.diagnostics(), 0);
    let published_preview = app.preview.published;
    let state = app.input_snapshot()?.ok_or("No text input session")?;
    app.event(
        &WindowEvent::TextInput(TextInputEvent {
            session: state.session,
            revision: state.revision,
            sequence: state.acknowledged + 1,
            timestamp: Duration::ZERO,
            edit: TextInputEdit::SetMarked {
                range: 2..2 + "山间".len(),
                text: "shan jian".into(),
                selection: 5..9,
            },
        }),
        &mut scheduler,
        &waker,
    )?;
    assert_eq!(app.ui.source.editor().borrow().source(), SOURCE);
    assert_eq!(app.preview.submitted, published_preview);
    let marked = app.input_snapshot()?.ok_or("The source lost input focus")?;
    assert!(marked.is_valid());
    assert_eq!(marked.marked, Some(2..11));
    assert_eq!(marked.text.get(marked.selection.clone()), Some("jian"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let snapshots = SnapshotHarness::new(
        &root.join("../..").canonicalize()?,
        root,
        env!("CARGO_PKG_NAME"),
    )?;
    let mut failures = Vec::new();
    for (name, scale) in SCALES {
        let size = PhysicalSize {
            width: (850. * scale) as u32,
            height: (416. * scale) as u32,
        };
        let frame = app.paint(
            FrameTiming {
                kind: FrameKind::Resize,
                predicted_presentation_timestamp: Duration::ZERO,
                refresh_interval: Duration::from_millis(16),
            },
            size,
            scale,
        )?;
        let state = app
            .tree
            .focused_editor_state(app.viewport)?
            .ok_or("The source lost input focus during paint")?;
        let source = app.tree.bounds(state.node)?;
        let preview = app.tree.bounds(app.ui.slots.preview.get()?)?;
        let [_, top, right, bottom] = source.edges();
        let [preview_left, preview_top, _, preview_bottom] = preview.edges();
        assert_eq!(source.size(), preview.size());
        assert_eq!((preview_top, preview_bottom), (top, bottom));
        assert!(right < preview_left);
        let (selection, rect) = state
            .first_rect(marked.selection.clone())
            .ok_or("The marked selection has no painted geometry")?;
        assert_eq!(selection, marked.selection);
        let [x0, y0, x1, y1] = rect.edges();
        let [left, top, right, bottom] = state.viewport.edges();
        assert!(left <= x0 && x0 < x1 && x1 <= right && top <= y0 && y0 < y1 && y1 <= bottom);

        let image = CpuRenderer.render(
            &frame.scene,
            frame.root,
            PixelSize {
                width: size.width,
                height: size.height,
            },
        )?;
        let actual = oxiview_testing::RgbaImage::new(
            image.width(),
            image.height(),
            image.pixels().to_vec(),
        )?;
        if let Err(e) = snapshots.assert_snapshot(name, &actual, &CompareConfig::default()) {
            failures.push(e.to_string());
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n").into())
    }
}
