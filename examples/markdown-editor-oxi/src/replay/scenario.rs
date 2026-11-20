//! Exercises the writer journey by sending platform events to one editor window.

use super::{
    Report, Result, new_file,
    window::{Focus, HEIGHT, IDLE_OPPORTUNITIES, State, WIDTH, Window, center},
    write_png,
};
use crate::app::WELCOME;
use oxiview_base::{Point, Rect, RgbaImage};
use oxiview_platform::{
    ClipboardRequest, ClipboardResponse, FilePanelKind, FilePanelRequest, KeyCode, KeyModifiers,
    TextCommitKind, TextInputEdit, TextInputSnapshot,
};
use std::{fs, io::Write, ops::Range, path::Path};

const HEADING: &str = "山间书屋";
const CONFIRMED: &str = "山林笔记";
const EMOJI: &str = "👩‍💻";
const ROUNDTRIP: &str = "roundtrip.md";

pub(super) fn run(window: &mut Window, output: &Path) -> Result<Report> {
    let source = WELCOME.replacen(
        "\n\n## Before dusk",
        &format!("\n\nWriter {EMOJI} keeps notes.\n\n## Before dusk"),
        1,
    );
    let input = output.join("acceptance.md");
    new_file(&input)?.write_all(encoded(&source).as_bytes())?;

    let mut replay = Replay {
        window,
        output,
        images: Vec::new(),
    };
    let opened = replay.open_fixture(&source, &input)?;
    let (preedit_parses, commit_parses) = replay.selection_and_ime(&opened)?;
    let edited = replay.find_and_clipboard(&source.replacen(HEADING, CONFIRMED, 1))?;
    let graphics = replay.graphics(&edited, &input)?;
    replay.files(&graphics, &source, &input)?;

    replay
        .window
        .driver
        .activation_changed(replay.window.id, false, None)?;
    replay
        .window
        .until("inactive editor stopped publishing input", || {
            Ok(replay
                .window
                .driver
                .text_input_snapshot(replay.window.id)?
                .is_none()
                .then_some(()))
        })?;
    replay.image("inactive.png")?;
    let idle_presentations = replay.window.idle_presentations()?;
    check(
        idle_presentations == 0,
        format!("Inactive editor presented {idle_presentations} extra frames"),
    )?;

    Ok(Report {
        output: output.to_owned(),
        checks: vec![
            "Open the named Markdown fixture with a relative image, three diagrams and four formulas",
            "Drag across wrapped bidirectional text, extend a selection with Shift and delete a joined emoji",
            "Keep marked text local and publish a single confirmed edit with undo and redo",
            "Restore source selection on blur and ignore the previous IME session",
            "Find next and previous, replace a match, and undo and redo Replace All as one edit",
            "Copy, cut and paste plain text, preserving selection on cut denial and rejecting a late paste",
            "Render changed Mermaid and math, show located diagnostics, then restore both drawings",
            "Cancel a dirty close and a Save As panel, recover from a failed write and save a named file",
            "Discard an unsaved edit with New, cancel Open and reopen the exact saved UTF-8 BOM and CRLF bytes",
            "Zero additional presentations over 1,000 inactive display opportunities, then clean close",
        ],
        images: replay.images,
        roundtrip: ROUNDTRIP.into(),
        idle_presentations,
        idle_opportunities: IDLE_OPPORTUNITIES,
        preedit_parses,
        commit_parses,
    })
}

struct Replay<'a> {
    window: &'a mut Window,
    output: &'a Path,
    images: Vec<String>,
}

impl Replay<'_> {
    fn image(&mut self, name: &str) -> Result<RgbaImage> {
        let image = self.window.capture(name)?;
        self.save_image(name, &image)?;
        Ok(image)
    }

    fn save_image(&mut self, name: &str, image: &RgbaImage) -> Result<()> {
        write_png(&self.output.join(name), image)?;
        self.images.push(name.into());
        Ok(())
    }

    fn scales(&mut self, name: &str, height: u32) -> Result<RgbaImage> {
        let first = self.image(&format!("{name}-1x.png"))?;
        check(
            first.width() == WIDTH && first.height() == height,
            "Unexpected 1× frame size",
        )?;
        for (scale, label, width, height) in [
            (1.5, "1.5x", WIDTH * 3 / 2, height * 3 / 2),
            (2., "2x", WIDTH * 2, height * 2),
        ] {
            let image = self.window.resize(width, height, scale)?;
            self.save_image(&format!("{name}-{label}.png"), &image)?;
        }
        self.window.resize(WIDTH, height, 1.)?;
        Ok(first)
    }

    fn open_fixture(&mut self, source: &str, input: &Path) -> Result<State> {
        self.window
            .state("initial preview", State::current_preview)?;
        let panel = self.panel(KeyCode::KeyO, false, FilePanelKind::Open)?;
        self.window
            .driver
            .file_panel_reply(self.window.id, panel.id, Ok(Some(input.into())))?;
        let opened = self.ready_document(input, source)?;
        check(
            opened.diagnostics.is_empty()
                && opened.stats.diagram_renders == 3
                && opened.stats.math_renders == 4,
            format!(
                "Invalid Markdown fixture preview: {:?}, {:?}",
                opened.stats, opened.diagnostics
            ),
        )?;
        self.image("document.png")?;
        Ok(opened)
    }

    fn selection_and_ime(&mut self, opened: &State) -> Result<(u64, u64)> {
        let window = &*self.window;
        let source = &opened.source;
        let start = locate(source, "At the trailhead")?.start;
        let end = locate(source, "window*.")?.start;
        let initial = self.source_snapshot(source)?;
        let from = caret_point(&initial, start)?;
        let to = caret_point(&initial, end)?;
        check(
            to.y() > from.y(),
            "Mixed-direction fixture must wrap in the source viewport",
        )?;
        window.drag(from, to)?;
        window.snapshot("drag through wrapped bidirectional text", |s| {
            s.text == *source && s.selection == (start..end)
        })?;
        self.scales("selection", HEIGHT)?;

        // The joined cluster occupies one caret stop and is deleted in one step.
        let emoji = locate(source, EMOJI)?;
        let snapshot = self.source_snapshot(source)?;
        let caret = caret_point(&snapshot, emoji.end)?;
        self.window.drag(caret, caret)?;
        self.window
            .snapshot("caret immediately after the joined emoji", |s| {
                s.text == *source && s.selection == (emoji.end..emoji.end)
            })?;
        self.window
            .key(KeyCode::Backspace, KeyModifiers::default())?;
        let without_emoji = replace(source, emoji, "")?;
        self.changed(opened, &without_emoji, "delete one joined emoji")?;
        self.window.shortcut(KeyCode::KeyZ, false)?;
        let before = self.window.state("restored emoji", |s| {
            s.source == *source && !s.dirty && s.current_preview()
        })?;

        let heading = locate(source, HEADING)?;
        self.select_heading_prefix(source, heading.start, "山间书")?;
        self.window.key(
            KeyCode::ArrowRight,
            KeyModifiers {
                shift: true,
                ..KeyModifiers::default()
            },
        )?;
        let selected = self
            .window
            .snapshot("Shift extended the heading selection", |s| {
                s.text == *source && s.selection == heading
            })?;
        let interim = self.mark(&selected, heading.clone(), "shan")?;
        let marked = self.mark(
            &interim,
            heading.start..heading.start + "shan".len(),
            "shanlin",
        )?;
        self.scales("preedit", HEIGHT)?;
        let during = self.window.state("marked text in the source editor", |s| {
            s.source_marked == marked.marked
        })?;
        let preedit_parses = during
            .stats
            .parses
            .checked_sub(before.stats.parses)
            .ok_or("Preview parse counter went backwards")?;
        check(
            during.source == *source
                && !during.dirty
                && during.published == before.published
                && during.submitted == before.submitted
                && preedit_parses == 0,
            "Marked text changed committed Markdown or its preview",
        )?;

        let edited = replace(source, heading.clone(), CONFIRMED)?;
        self.window.input(
            &marked,
            TextInputEdit::Commit {
                range: marked.marked.clone().ok_or("Expected a marked range")?,
                text: CONFIRMED.into(),
                kind: TextCommitKind::Composition,
            },
        )?;
        let confirmed = self.changed(&before, &edited, "confirmed heading")?;
        let after_commit = heading.start + CONFIRMED.len();
        self.window
            .snapshot("committed IME caret and cleared mark", |s| {
                s.session == marked.session
                    && s.acknowledged == marked.acknowledged + 1
                    && s.text == edited
                    && s.marked.is_none()
                    && s.selection == (after_commit..after_commit)
            })?;
        let commit_parses = confirmed.stats.parses - before.stats.parses;
        self.window.shortcut(KeyCode::KeyZ, false)?;
        self.window
            .snapshot("undo restored the original heading and selection", |s| {
                s.text == *source && s.selection == heading
            })?;
        let undo = self.window.state("undo restored the savepoint", |s| {
            s.source == *source && !s.dirty && s.current_preview()
        })?;
        self.window.shortcut(KeyCode::KeyZ, true)?;
        let redo = self.changed(&undo, &edited, "redo composition")?;
        self.window
            .snapshot("redo restored the confirmed caret", |s| {
                s.text == edited && s.selection == (after_commit..after_commit)
            })?;

        self.select_heading_prefix(&edited, heading.start, "山林")?;
        let range = heading.start..heading.start + "山林".len();
        let snapshot = self
            .window
            .snapshot("selection before blurred preedit", |s| {
                s.text == edited && s.selection == range
            })?;
        let old = self.mark(&snapshot, range.clone(), "lin")?;
        self.window.click("Find")?;
        let find = self
            .window
            .snapshot("Find received a fresh input session", |s| {
                s.session != old.session && s.text.is_empty() && s.marked.is_none()
            })?;
        self.window.input(
            &old,
            TextInputEdit::Commit {
                range: old
                    .marked
                    .clone()
                    .ok_or("Expected marked text before blur")?,
                text: "迟到".into(),
                kind: TextCommitKind::Composition,
            },
        )?;
        self.window.input(&find, typed(0..0, "trail"))?;
        let blurred = self
            .window
            .state("first valid Find input after the stale callback", |s| {
                s.query == "trail" && s.focus == Focus::Query
            })?;
        check(
            blurred.source == edited
                && blurred.source_selection == range
                && blurred.source_marked.is_none()
                && blurred.published == redo.published
                && blurred.submitted == redo.submitted
                && blurred.stats == redo.stats,
            "Blur failed to restore the committed selection and preview or allowed stale IME input",
        )?;
        Ok((preedit_parses, commit_parses))
    }

    fn find_and_clipboard(&mut self, source: &str) -> Result<String> {
        let matches: Vec<_> = source
            .match_indices("trail")
            .map(|(start, text)| start..start + text.len())
            .collect();
        let first = matches.first().ok_or("Missing trail match")?;
        let last = matches.last().ok_or("Missing last trail match")?;
        check(
            matches.len() >= 3,
            "The Find fixture needs at least three matches",
        )?;

        self.find_match(false, first)?;
        self.find_match(true, last)?;
        self.find_match(false, first)?;
        self.image("find.png")?;
        self.window.key(KeyCode::Escape, KeyModifiers::default())?;
        self.window
            .snapshot("selected match after dismissing Find", |s| {
                s.text == source && s.selection == *first
            })?;

        self.window.shortcut(KeyCode::KeyC, false)?;
        let request = self.clipboard()?;
        check(
            matches!(&request, ClipboardRequest::Write { text, .. } if text == "trail"),
            "Copy did not request the selected plain text",
        )?;
        self.window.driver.clipboard_reply(
            self.window.id,
            request.id(),
            Ok(ClipboardResponse::Written),
        )?;

        self.window.shortcut(KeyCode::KeyX, false)?;
        let request = self.clipboard()?;
        check(
            matches!(&request, ClipboardRequest::Write { text, .. } if text == "trail"),
            "Cut did not request the selected plain text",
        )?;
        self.window.driver.clipboard_reply(
            self.window.id,
            request.id(),
            Err("replay permission denied".into()),
        )?;
        let denied = self.window.state("cut denial", |s| {
            s.notice == "Clipboard: replay permission denied"
        })?;
        check(
            denied.source == source && denied.source_selection == *first,
            "Denied cut destroyed the selection",
        )?;

        self.window.shortcut(KeyCode::KeyX, false)?;
        let request = self.clipboard()?;
        check(
            matches!(&request, ClipboardRequest::Write { text, .. } if text == "trail"),
            "Cut retry used a different selection",
        )?;
        self.window.driver.clipboard_reply(
            self.window.id,
            request.id(),
            Ok(ClipboardResponse::Written),
        )?;
        self.changed(
            &denied,
            &replace(source, first.clone(), "")?,
            "authorized cut",
        )?;
        self.window.shortcut(KeyCode::KeyZ, false)?;
        let restored = self.window.state("undo restored the cut selection", |s| {
            s.source == source && s.source_selection == *first && s.current_preview()
        })?;

        let read = self.request_paste()?;
        self.window.shortcut(KeyCode::KeyF, false)?;
        let find = self
            .window
            .snapshot("Find focused while paste is in flight", |s| {
                s.text == "trail"
            })?;
        self.window.driver.clipboard_reply(
            self.window.id,
            read.id(),
            Ok(ClipboardResponse::Read(Some("stale clipboard text".into()))),
        )?;
        self.window.input(&find, typed(0..5, "trail!"))?;
        let after = self.window.state("Find input following stale paste", |s| {
            s.query == "trail!" && s.focus == Focus::Query
        })?;
        check(
            after.source == source
                && after.source_selection == *first
                && after.stats == restored.stats
                && after.published == restored.published
                && after.submitted == restored.submitted,
            "Delayed paste modified the previous field or published a preview",
        )?;
        let query = self
            .window
            .snapshot("current search text", |s| s.text == "trail!")?;
        self.window.input(&query, typed(0..6, "trail"))?;
        self.window
            .state("restored search query", |s| s.query == "trail")?;
        self.window.key(KeyCode::Escape, KeyModifiers::default())?;
        self.window
            .snapshot("returned to the source selection for paste", |s| {
                s.text == source && s.selection == *first
            })?;

        let read = self.request_paste()?;
        self.window.driver.clipboard_reply(
            self.window.id,
            read.id(),
            Ok(ClipboardResponse::Read(Some("footpath".into()))),
        )?;
        let pasted = replace(source, first.clone(), "footpath")?;
        let pasted = self.changed(&restored, &pasted, "plain-text paste")?;

        self.window.shortcut(KeyCode::KeyF, false)?;
        self.window
            .state("Find focused for replacement", |s| s.focus == Focus::Query)?;
        self.window.key(KeyCode::Tab, KeyModifiers::default())?;
        let field = self
            .window
            .snapshot("replacement field focused", |s| s.text.is_empty())?;
        self.window.input(&field, typed(0..0, "ridge"))?;
        self.window.state("replacement text", |s| {
            s.focus == Focus::Replacement && s.replacement == "ridge"
        })?;
        self.window.click("Replace")?;
        self.changed(
            &pasted,
            &pasted.source.replacen("trail", "ridge", 1),
            "replace next literal match",
        )?;
        self.window.key(KeyCode::Escape, KeyModifiers::default())?;
        self.window
            .state("source focused for undo", |s| s.focus == Focus::Source)?;
        self.window.shortcut(KeyCode::KeyZ, false)?;
        let before_all = self.window.state("undo single replacement", |s| {
            s.source == pasted.source && s.current_preview()
        })?;
        self.window.shortcut(KeyCode::KeyF, false)?;
        self.window
            .state("Find focused for Replace All", |s| s.focus == Focus::Query)?;
        self.window.click("Replace All")?;
        let all = pasted.source.replace("trail", "ridge");
        let replaced = self.changed(&before_all, &all, "replace all literal matches")?;
        check(
            replaced.notice == "Replaced 2 matches.",
            "Replace All did not replace both remaining matches",
        )?;
        self.window.key(KeyCode::Escape, KeyModifiers::default())?;
        self.window
            .state("source focused for atomic replacement undo", |s| {
                s.focus == Focus::Source
            })?;
        self.window.shortcut(KeyCode::KeyZ, false)?;
        let undone = self.changed(
            &replaced,
            &pasted.source,
            "one undo restored both replacements",
        )?;
        self.window.shortcut(KeyCode::KeyZ, true)?;
        self.changed(&undone, &all, "redo both replacements")?;
        Ok(all)
    }

    fn graphics(&mut self, source: &str, input: &Path) -> Result<String> {
        const GRAPHICS_HEIGHT: u32 = 1920;
        const EDGE: &str = "Open[Open file] -->|read| Write[Write notes]";
        const LABELED: &str = "Open[Open file] -->|read more| Write[Write notes]";
        const BROKEN_EDGE: &str = "--> Write";
        const FORMULA: &str = r"\frac{1}{2}";
        const REVISED: &str = r"\frac{1}{5}";
        const BROKEN_FORMULA: &str = r"\frac{1}{";

        self.window.resize(WIDTH, GRAPHICS_HEIGHT, 1.)?;
        let initial = self.window.state("published graphics fixture", |s| {
            s.source == source && s.current_preview()
        })?;
        let preview = initial.preview;
        self.window
            .driver
            .wheel(self.window.id, center(preview), 0., -10_000., None)?;
        let original = self.image("graphics-original.png")?;

        let diagram = self.edit(&initial, EDGE, LABELED, "changed Mermaid edge label")?;
        check(
            diagram.diagnostics.is_empty()
                && diagram.stats.diagram_renders == initial.stats.diagram_renders + 1
                && diagram.stats.math_renders == initial.stats.math_renders,
            "Changing the edge did not recompile only Mermaid",
        )?;
        let diagram_image = self.image("graphics-diagram-changed.png")?;
        check(
            !same_pixels(&original, &diagram_image, preview)?,
            "The labeled diagram did not change preview pixels",
        )?;

        let formula = self.edit(&diagram, FORMULA, REVISED, "changed math denominator")?;
        check(
            formula.diagnostics.is_empty()
                && formula.stats.math_renders == diagram.stats.math_renders + 1
                && formula.stats.diagram_renders == diagram.stats.diagram_renders,
            "Changing the denominator did not recompile only math",
        )?;
        let valid = self.image("graphics-math-changed.png")?;
        check(
            !same_pixels(&diagram_image, &valid, preview)?,
            "The changed formula did not change preview pixels",
        )?;

        let edge_error = self.edit(&formula, LABELED, BROKEN_EDGE, "broken Mermaid edge")?;
        let broken = self.edit(&edge_error, REVISED, BROKEN_FORMULA, "broken display math")?;
        let edge_range = locate(&broken.source, "  --> Write")?;
        let formula_range = locate(&broken.source, "$$\n\\frac{1}{ + \\sqrt{u}\n$$")?;
        let expected_ranges = [edge_range.start + 2..edge_range.start + 3, formula_range];
        check(
            broken.diagnostics.len() == 2,
            format!(
                "Expected a Mermaid and math diagnostic, got {:?}",
                broken.diagnostics
            ),
        )?;
        for ((diagnostic, range), format) in broken
            .diagnostics
            .iter()
            .zip(expected_ranges)
            .zip(["Mermaid", "Math"])
        {
            let prefix = &broken.source[..range.start];
            let line = prefix.bytes().filter(|&byte| byte == b'\n').count() + 1;
            let column = prefix
                .rsplit('\n')
                .next()
                .unwrap_or_default()
                .chars()
                .count()
                + 1;
            check(
                diagnostic.file.as_deref() == Some(input)
                    && diagnostic.range == range
                    && diagnostic.line == line
                    && diagnostic.column == column
                    && diagnostic.message.starts_with(format),
                format!("Incorrect {format} diagnostic: {diagnostic:?}"),
            )?;
        }
        let broken_image = self.image("graphics-broken.png")?;
        let [left, top, right, _] = preview.edges();
        let surrounding = Rect::new(left, top, right, top + 240.);
        check(
            !same_pixels(&valid, &broken_image, preview)?
                && same_pixels(&valid, &broken_image, surrounding)?,
            "Broken graphics did not update their preview while preserving the surrounding text",
        )?;

        let repaired_edge = self.edit(&broken, BROKEN_EDGE, LABELED, "corrected Mermaid edge")?;
        check(
            repaired_edge.diagnostics.len() == 1
                && repaired_edge.diagnostics[0].message.starts_with("Math"),
            "Repairing Mermaid did not leave only the math diagnostic",
        )?;
        let repaired = self.edit(
            &repaired_edge,
            BROKEN_FORMULA,
            REVISED,
            "corrected display math",
        )?;
        check(
            repaired.diagnostics.is_empty()
                && repaired.source == formula.source
                && repaired.focus == Focus::Source
                && repaired.stats.diagram_renders == broken.stats.diagram_renders
                && repaired.stats.math_renders == broken.stats.math_renders,
            "Repair did not restore both cached graphics in the live preview",
        )?;
        let corrected = self.scales("graphics-corrected", GRAPHICS_HEIGHT)?;
        check(
            same_pixels(&valid, &corrected, preview)?,
            "Correcting both graphics did not restore the valid preview pixels",
        )?;
        self.window.resize(WIDTH, HEIGHT, 1.)?;
        let resized = self.window.state("normal preview viewport", |s| {
            s.preview.size().height() < preview.size().height()
        })?;
        self.window
            .driver
            .wheel(self.window.id, center(resized.preview), 0., 620., None)?;
        let scrolled = self.image("preview-scroll.png")?;
        check(
            !same_pixels(
                &original,
                &scrolled,
                Rect::new(left, top, right, top + 400.),
            )?,
            "Preview wheel had no visible effect",
        )?;
        Ok(repaired.source)
    }

    fn files(&self, source: &str, input_text: &str, input: &Path) -> Result<()> {
        let window = &*self.window;
        window.driver.request_user_close(window.id)?;
        let prompt = window.state("dirty close prompt", |s| {
            s.notice == "This document has unsaved changes."
        })?;
        check(
            prompt.dirty && prompt.source == source && prompt.file.as_deref() == Some(input),
            "Close prompt discarded the current document",
        )?;
        window.key(KeyCode::Escape, KeyModifiers::default())?;
        window.state("cancelled dirty close", |s| {
            s.notice == "Ready" && s.focus == Focus::Source
        })?;

        let kind = FilePanelKind::Save {
            suggested_name: Some("acceptance.md".into()),
        };
        let cancelled = self.panel(KeyCode::KeyS, true, kind.clone())?;
        window
            .driver
            .file_panel_reply(window.id, cancelled.id, Ok(None))?;
        let state = window.state("cancelled Save As", |s| s.notice == "Cancelled.")?;
        check(
            state.dirty && state.source == source && state.file.as_deref() == Some(input),
            "Cancelling Save As changed the document",
        )?;

        let failing = self.panel(KeyCode::KeyS, true, kind.clone())?;
        window
            .driver
            .file_panel_reply(window.id, failing.id, Ok(Some(self.output.to_owned())))?;
        let state = window.state("failed Save As", |s| {
            s.notice.starts_with("Could not save:")
        })?;
        check(
            state.dirty && state.source == source && state.file.as_deref() == Some(input),
            "A failed write changed the current document",
        )?;

        let panel = self.panel(KeyCode::KeyS, true, kind)?;
        let path = self.output.join(ROUNDTRIP);
        window
            .driver
            .file_panel_reply(window.id, panel.id, Ok(Some(path.clone())))?;
        let saved = self.ready_document(&path, source)?;
        check(
            fs::read(&path)? == encoded(source).as_bytes(),
            "Save As did not preserve UTF-8 BOM, Unicode and CRLF bytes",
        )?;

        let mut final_source = source.to_owned();
        let more = "\nSaved via the normal Save Action.\n";
        let end = source.len();
        final_source.push_str(more);
        let snapshot = self.source_snapshot(source)?;
        window.input(&snapshot, typed(end..end, more))?;
        let current = self.changed(&saved, &final_source, "edit before ordinary Save")?;
        window.shortcut(KeyCode::KeyS, false)?;
        self.ready_document(&path, &final_source)?;
        check(
            window.driver.file_panel_requests(window.id)?.is_empty(),
            "Saving a named file opened another panel",
        )?;
        check(
            fs::read(&path)? == encoded(&final_source).as_bytes(),
            "Save missed the edited bytes",
        )?;
        check(
            fs::read(input)? == encoded(input_text).as_bytes(),
            "Save As replaced its original input",
        )?;

        let end = final_source.len();
        let snapshot = self.source_snapshot(&final_source)?;
        window.input(&snapshot, typed(end..end, "Discard this draft."))?;
        self.changed(
            &current,
            &format!("{final_source}Discard this draft."),
            "temporary unsaved draft",
        )?;
        window.shortcut(KeyCode::KeyN, false)?;
        window.state("dirty New prompt", |s| {
            s.dirty && s.notice == "This document has unsaved changes."
        })?;
        window.click("Discard")?;
        window.state("new document after discard", |s| {
            s.current_preview() && s.source.is_empty() && s.file.is_none() && !s.dirty
        })?;
        let panel = self.panel(KeyCode::KeyO, false, FilePanelKind::Open)?;
        window
            .driver
            .file_panel_reply(window.id, panel.id, Ok(None))?;
        let state = window.state("cancelled Open", |s| s.notice == "Cancelled.")?;
        check(
            state.file.is_none() && state.source.is_empty() && !state.dirty,
            "Cancelling Open changed the untitled document",
        )?;

        let panel = self.panel(KeyCode::KeyO, false, FilePanelKind::Open)?;
        window
            .driver
            .file_panel_reply(window.id, panel.id, Ok(Some(path.clone())))?;
        let reopened = self.ready_document(&path, &final_source)?;
        check(
            reopened.diagnostics.is_empty(),
            "The reopened preview has diagnostics",
        )?;
        self.source_snapshot(&final_source)?;
        Ok(())
    }

    fn select_heading_prefix(&self, text: &str, start: usize, prefix: &str) -> Result<()> {
        let snapshot = self.source_snapshot(text)?;
        self.window.drag(
            caret_point(&snapshot, start)?,
            caret_point(&snapshot, start + prefix.len())?,
        )
    }

    fn mark(
        &self,
        snapshot: &TextInputSnapshot,
        range: Range<usize>,
        text: &str,
    ) -> Result<TextInputSnapshot> {
        let displayed = replace(&snapshot.text, range.clone(), text)?;
        let end = range.start + text.len();
        self.window.input(
            snapshot,
            TextInputEdit::SetMarked {
                range: range.clone(),
                text: text.into(),
                selection: text.len()..text.len(),
            },
        )?;
        let marked = self.window.snapshot(
            "matching marked range, selection and native acknowledgement",
            |s| {
                s.session == snapshot.session
                    && s.acknowledged == snapshot.acknowledged + 1
                    && s.text == displayed
                    && s.marked == Some(range.start..end)
                    && s.selection == (end..end)
            },
        )?;
        let state = self.window.state("marked source geometry", |s| {
            s.source_marked == marked.marked
        })?;
        check(
            marked.is_valid()
                && marked.caret.size().height() > 0.
                && inside(state.source_bounds, center(marked.caret))
                && marked.geometry.iter().any(|fragment| {
                    !fragment.range.is_empty()
                        && fragment.range.start >= range.start
                        && fragment.range.end <= end
                }),
            "Marked text and candidate caret must have valid visible rectangles",
        )?;
        Ok(marked)
    }

    fn source_snapshot(&self, text: &str) -> Result<TextInputSnapshot> {
        self.window.state("source focused", |s| {
            s.focus == Focus::Source && s.source == text
        })?;
        self.window
            .snapshot("committed source input snapshot", |s| {
                s.text == text && s.marked.is_none()
            })
    }

    fn changed(&self, previous: &State, source: &str, label: &str) -> Result<State> {
        let latest = self.window.state(label, |s| {
            s.source == source && s.current_preview() && s.published > previous.published
        })?;
        check(
            latest.submitted == previous.submitted + 1
                && latest.stats.parses == previous.stats.parses + 1,
            format!(
                "{label}: expected one committed preview, got {}/{} parses",
                previous.stats.parses, latest.stats.parses
            ),
        )?;
        Ok(latest)
    }

    fn edit(&self, previous: &State, old: &str, new: &str, label: &str) -> Result<State> {
        let snapshot = self.source_snapshot(&previous.source)?;
        let range = locate(&previous.source, old)?;
        let expected = replace(&previous.source, range.clone(), new)?;
        let end = range.start + new.len();
        self.window.input(&snapshot, typed(range, new))?;
        let edited = self.changed(previous, &expected, label)?;
        let snapshot = self.window.snapshot(
            "source kept its input session through the preview edit",
            |s| s.session == snapshot.session && s.text == expected && s.selection == (end..end),
        )?;
        check(
            inside(edited.source_bounds, center(snapshot.caret)),
            "The edited caret scrolled out of view",
        )?;
        Ok(edited)
    }

    fn find_match(&self, previous: bool, range: &Range<usize>) -> Result<()> {
        self.window.shortcut(KeyCode::KeyG, previous)?;
        self.window
            .state("literal Find selected the expected range", |s| {
                s.focus == Focus::Query
                    && s.source_selection == *range
                    && s.notice.starts_with("Match ")
            })?;
        Ok(())
    }

    fn clipboard(&self) -> Result<ClipboardRequest> {
        self.window.until("one pending clipboard operation", || {
            only(self.window.driver.clipboard_requests(self.window.id)?)
        })
    }

    fn request_paste(&self) -> Result<ClipboardRequest> {
        self.window.shortcut(KeyCode::KeyV, false)?;
        let request = self.clipboard()?;
        check(
            matches!(request, ClipboardRequest::Read { .. }),
            "Paste did not request plain text",
        )?;
        Ok(request)
    }

    fn panel(&self, key: KeyCode, shift: bool, kind: FilePanelKind) -> Result<FilePanelRequest> {
        self.window.shortcut(key, shift)?;
        let panel = self.window.until("one pending file panel", || {
            only(self.window.driver.file_panel_requests(self.window.id)?)
        })?;
        check(
            panel.kind == kind,
            format!("Expected {kind:?} file panel, got {:?}", panel.kind),
        )?;
        Ok(panel)
    }

    fn ready_document(&self, path: &Path, text: &str) -> Result<State> {
        self.window
            .state("current preview of the named clean document", |s| {
                s.source == text
                    && s.file.as_deref() == Some(path)
                    && !s.dirty
                    && s.current_preview()
                    && s.focus == Focus::Source
            })
    }
}

fn encoded(text: &str) -> String {
    format!("\u{feff}{}", text.replace('\n', "\r\n"))
}

fn typed(range: Range<usize>, text: &str) -> TextInputEdit {
    TextInputEdit::Commit {
        range,
        text: text.into(),
        kind: TextCommitKind::Typing,
    }
}

fn locate(source: &str, needle: &str) -> Result<Range<usize>> {
    let start = source
        .find(needle)
        .ok_or_else(|| format!("Missing fixture text {needle:?}"))?;
    Ok(start..start + needle.len())
}

fn replace(source: &str, range: Range<usize>, text: &str) -> Result<String> {
    if source.get(range.clone()).is_none() {
        return Err(format!("Invalid replay source byte range {range:?}").into());
    }
    let mut result = source.to_owned();
    result.replace_range(range, text);
    Ok(result)
}

fn caret_point(snapshot: &TextInputSnapshot, offset: usize) -> Result<Point> {
    snapshot
        .geometry
        .iter()
        .find(|fragment| fragment.range == (offset..offset))
        .map(|fragment| center(fragment.rect))
        .ok_or_else(|| format!("Replay could not locate visible caret at byte {offset}").into())
}

fn inside(rect: Rect, point: Point) -> bool {
    let [left, top, right, bottom] = rect.edges();
    (left..right).contains(&point.x()) && (top..bottom).contains(&point.y())
}

fn only<T>(mut requests: Vec<T>) -> Result<Option<T>> {
    if requests.len() > 1 {
        return Err(format!("Expected one outstanding request, got {}", requests.len()).into());
    }
    Ok(requests.pop())
}

fn same_pixels(left: &RgbaImage, right: &RgbaImage, bounds: Rect) -> Result<bool> {
    let [x0, y0, x1, y1] = bounds.edges();
    let (x0, y0, x1, y1) = (
        x0.ceil() as usize,
        y0.ceil() as usize,
        x1.floor() as usize,
        y1.floor() as usize,
    );
    check(
        x0 < x1
            && y0 < y1
            && x1 <= left.width().min(right.width()) as usize
            && y1 <= left.height().min(right.height()) as usize,
        "Invalid replay pixel comparison region",
    )?;
    Ok((y0..y1).all(|row| {
        let left_offset = row * left.width() as usize * 4;
        let right_offset = row * right.width() as usize * 4;
        left.pixels()[left_offset + x0 * 4..left_offset + x1 * 4]
            == right.pixels()[right_offset + x0 * 4..right_offset + x1 * 4]
    }))
}

fn check(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into().into())
    }
}
