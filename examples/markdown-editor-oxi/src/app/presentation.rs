//! Window chrome, caret deadlines and the platform's focused input view.

use super::*;

#[derive(Default)]
pub(super) struct Presentation {
    source_revision: Option<u64>,
    document_changed: bool,
    dirty: bool,
    lines: SourceLines,
    position: Option<usize>,
    notice: String,
    issues: usize,
    source_scroll: u64,
    last_input: Option<InputStamp>,
    caret_geometry: Option<EditorGeometry>,
    caret_deadline: Option<Duration>,
}

#[derive(Clone, Copy, PartialEq)]
struct EditorGeometry {
    session: u64,
    presentation_revision: u64,
    // The shell uses fixed field padding. The field bounds and window
    // viewport capture its layout and clipping, while wheel scroll is local.
    bounds: Rect,
    viewport: Size,
    scroll: u64,
}

#[derive(PartialEq)]
struct InputStamp {
    geometry: EditorGeometry,
    input_revision: u64,
    acknowledged: u64,
}

impl Presentation {
    pub(super) fn document_changed(&mut self) {
        self.document_changed = true;
    }

    pub(super) fn source_scrolled(&mut self) {
        self.source_scroll = self.source_scroll.wrapping_add(1);
    }
}

impl EditorApp {
    pub(super) fn sync(
        &mut self,
        waker: &WindowWaker,
        scheduler: &mut Scheduler,
        commands_ran: bool,
    ) -> Result<(), Error> {
        let file_changed = self.poll_files()?;
        let labels_changed = self.update_labels(waker, scheduler)?;
        // Dispatch already drained the tree's queues. Only settle again if an
        // application change followed it.
        if (commands_ran || file_changed || labels_changed)
            && self
                .tree
                .flush_updates(self.viewport, |_, _, _| {})?
                .changed
        {
            scheduler.invalidate();
        }
        self.update_caret(scheduler)
    }

    fn update_labels(
        &mut self,
        waker: &WindowWaker,
        scheduler: &mut Scheduler,
    ) -> Result<bool, Error> {
        let model = self.source();
        let editor = model.borrow();
        let revision = editor.committed_revision();
        let dirty = editor.document_state() != self.saved_state;
        let changed_source = self.presentation.source_revision != Some(revision);
        let changed_document = self.presentation.document_changed;
        if changed_source || changed_document {
            self.preview.submit(
                editor.source().into(),
                self.base_dir.clone(),
                self.document.path.clone(),
                waker,
            )?;
        }
        // Register the new request before draining completions so an old
        // result cannot publish after an edit or a document replacement.
        if self.preview.poll() {
            scheduler.invalidate();
        }
        if changed_source {
            self.presentation.lines.rebuild(editor.source());
            self.presentation.source_revision = Some(revision);
            self.presentation.position = None;
        }
        let mut changed = false;
        let offset = editor.selection().active.offset;
        if self.presentation.position != Some(offset) {
            let (line, column) = self.presentation.lines.position(editor.source(), offset);
            self.tree.set_text(
                self.ui.slots.position.get()?,
                format!("Ln {line}   Col {column}   ·   UTF-8"),
            )?;
            self.presentation.position = Some(offset);
            changed = true;
        }
        if changed_document || self.presentation.dirty != dirty {
            let mut title = self.document.title();
            if dirty {
                title.push_str("  •");
            }
            self.tree.set_text(self.ui.slots.title.get()?, title)?;
            self.presentation.dirty = dirty;
            self.presentation.document_changed = false;
            changed = true;
        }
        let issues = self.preview.diagnostics();
        if self.presentation.notice != self.notice || self.presentation.issues != issues {
            self.presentation.notice.clone_from(&self.notice);
            self.presentation.issues = issues;
            let message = if issues == 0 {
                self.notice.clone()
            } else {
                format!("{}   ·   {issues} preview issue(s)", self.notice)
            };
            self.tree.set_text(self.ui.slots.status.get()?, message)?;
            changed = true;
        }
        Ok(changed)
    }

    fn focused_input_stamp(&self) -> Result<Option<InputStamp>, Error> {
        let Some((field, node)) = self
            .field()
            .and_then(|field| field.node().map(|node| (field, node)))
        else {
            return Ok(None);
        };
        let model = field.editor();
        let editor = model.borrow();
        let Some(session) = editor.session() else {
            return Ok(None);
        };
        let scroll = if self.ui.source.node() == Some(node) {
            self.presentation.source_scroll
        } else {
            0
        };
        Ok(Some(InputStamp {
            geometry: EditorGeometry {
                session,
                presentation_revision: editor.presentation().revision,
                bounds: self.tree.bounds(node)?,
                viewport: self.viewport,
                scroll,
            },
            input_revision: editor.input_revision(),
            acknowledged: editor.acknowledged(),
        }))
    }

    fn update_caret(&mut self, scheduler: &mut Scheduler) -> Result<(), Error> {
        let geometry = self.focused_input_stamp()?.map(|stamp| stamp.geometry);
        if self.presentation.caret_geometry == geometry
            && self
                .presentation
                .caret_deadline
                .is_none_or(|t| self.now < t)
        {
            return Ok(());
        }
        let (blink, deadline) = self.tree.advance_editor_caret(self.now)?;
        self.presentation.caret_geometry = geometry;
        self.presentation.caret_deadline = deadline;
        if blink {
            scheduler.invalidate();
        }
        scheduler.cancel_timed_request();
        if let Some(deadline) = deadline {
            scheduler.request_at(deadline);
        }
        Ok(())
    }

    pub(super) fn publish_input(
        &mut self,
        commands: &mut Vec<PlatformCommand>,
    ) -> Result<(), Error> {
        self.tree.layout(self.viewport)?;
        let stamp = self.focused_input_stamp()?;
        if self.presentation.last_input != stamp {
            let input = self.tree.focused_text_input_snapshot(self.viewport)?;
            commands.push(PlatformCommand::TextInput(input));
            self.presentation.last_input = stamp;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn input_snapshot(
        &mut self,
    ) -> Result<Option<oxiview_platform::TextInputSnapshot>, Error> {
        Ok(self.tree.focused_text_input_snapshot(self.viewport)?)
    }
}

#[derive(Clone, Copy, Default)]
struct SourceOffset {
    byte: usize,
    chars: usize,
}

#[derive(Default)]
struct SourceLines {
    starts: Vec<SourceOffset>,
    // Bound the column lookup even in a document with one long line.
    checkpoints: Vec<SourceOffset>,
}

impl SourceLines {
    fn rebuild(&mut self, source: &str) {
        const STRIDE: usize = 256;
        self.starts.clear();
        self.checkpoints.clear();
        self.starts.push(SourceOffset::default());
        self.checkpoints.push(SourceOffset::default());
        let mut next = STRIDE;
        for (chars, (byte, ch)) in source.char_indices().enumerate() {
            if byte >= next {
                self.checkpoints.push(SourceOffset { byte, chars });
                next = byte + STRIDE;
            }
            if matches!(
                ch,
                '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
            ) && !(ch == '\r' && source.as_bytes().get(byte + 1) == Some(&b'\n'))
            {
                self.starts.push(SourceOffset {
                    byte: byte + ch.len_utf8(),
                    chars: chars + 1,
                });
            }
        }
    }

    fn position(&self, source: &str, byte: usize) -> (usize, usize) {
        let line = self.starts.partition_point(|offset| offset.byte <= byte);
        let start = self.starts[line - 1];
        let index = self
            .checkpoints
            .partition_point(|offset| offset.byte <= byte);
        let checkpoint = self.checkpoints[index - 1];
        let base = if checkpoint.byte > start.byte {
            checkpoint
        } else {
            start
        };
        let column = base.chars - start.chars + source[base.byte..byte].chars().count() + 1;
        (line, column)
    }
}
