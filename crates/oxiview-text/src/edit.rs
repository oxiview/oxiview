use icu_segmenter::{GraphemeClusterSegmenter, WordSegmenter, WordSegmenterBorrowed};
use std::{borrow::Cow, cell::OnceCell, ops::Range, sync::OnceLock, time::Duration};

/// Time allowed between adjacent direct edits in one undo step.
pub const TYPING_GROUP_INTERVAL: Duration = Duration::from_millis(500);

/// Which side of a logical position owns the caret at a wrap or directional boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Affinity {
    /// The caret follows the preceding logical grapheme.
    Upstream,
    /// The caret precedes the following logical grapheme.
    #[default]
    Downstream,
}

/// A UTF-8 byte offset and visual affinity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Caret {
    /// Byte offset in the corresponding text.
    pub offset: usize,
    /// Visual side of the position.
    pub affinity: Affinity,
}

impl Caret {
    /// Sets downstream affinity.
    pub const fn new(offset: usize) -> Self {
        Self::with_affinity(offset, Affinity::Downstream)
    }

    /// Sets an explicit visual affinity.
    pub const fn with_affinity(offset: usize, affinity: Affinity) -> Self {
        Self { offset, affinity }
    }
}

/// The fixed anchor and moving end of a selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// Fixed end when extending the selection.
    pub anchor: Caret,
    /// Moving end and caret location.
    pub active: Caret,
}

impl Selection {
    /// Selects the bytes between the two offsets, preserving their direction.
    pub const fn new(anchor: usize, active: usize) -> Self {
        if anchor == active {
            return Self::caret(anchor);
        }
        let start = Caret::new(if anchor < active { anchor } else { active });
        let end = Caret::with_affinity(
            if anchor < active { active } else { anchor },
            Affinity::Upstream,
        );
        if anchor < active {
            Self {
                anchor: start,
                active: end,
            }
        } else {
            Self {
                anchor: end,
                active: start,
            }
        }
    }

    /// Collapses the selection to a downstream caret.
    pub const fn caret(offset: usize) -> Self {
        Self::at(Caret::new(offset))
    }

    /// Collapses the selection to a caret with its existing affinity.
    pub const fn at(caret: Caret) -> Self {
        Self {
            anchor: caret,
            active: caret,
        }
    }

    /// The ascending UTF-8 range, regardless of selection direction.
    pub fn range(self) -> Range<usize> {
        self.anchor.offset.min(self.active.offset)..self.anchor.offset.max(self.active.offset)
    }

    /// Whether the selection has zero logical length.
    pub const fn is_empty(self) -> bool {
        self.anchor.offset == self.active.offset
    }
}

/// Editing policy shared by retained fields and standalone models.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditMode {
    /// Line breaks introduced through text input become spaces.
    SingleLine,
    /// Source line endings are retained.
    MultiLine,
}

/// An editing operation rejected before document mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    /// The range was reversed, past the end, or split a UTF-8 scalar.
    #[error("invalid UTF-8 editing range")]
    InvalidRange,
    /// The session, input revision, or event sequence no longer matches the editor.
    #[error("stale text input")]
    StaleInput,
    /// No new revision can be issued.
    #[error("editor revisions exhausted")]
    RevisionExhausted,
}

/// Model operations also usable for commands supplied by a native text system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditCommand {
    /// Move or extend the selection to committed text offsets.
    Select(Selection),
    /// Delete selection or text before the caret.
    DeleteBackward {
        /// Use a word boundary instead of a grapheme boundary.
        word: bool,
    },
    /// Delete selection or text after the caret.
    DeleteForward {
        /// Use a word boundary instead of a grapheme boundary.
        word: bool,
    },
    /// Delete the selection or reach a supplied committed text offset.
    DeleteTo(usize),
    /// Insert a newline in a multiline editor.
    InsertNewline,
}

/// An ordered update from the current native input session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeEdit {
    /// Start or update the inline preedit.
    Mark {
        /// Incoming marked text.
        text: String,
        /// Absolute byte range in the current presentation. The default is mark or selection.
        replacement: Option<Range<usize>>,
        /// Byte range relative to the incoming marked text.
        selection: Range<usize>,
    },
    /// Confirm a replacement as one undoable transaction.
    Commit {
        /// Confirmed text.
        text: String,
        /// Absolute byte range in the current presentation. The default is mark or selection.
        replacement: Option<Range<usize>>,
    },
    /// Direct typing, eligible for grouping when it inserts at an unmarked caret.
    Typing {
        /// Typed text.
        text: String,
        /// Absolute byte range in the current presentation. The default is mark or selection.
        replacement: Option<Range<usize>>,
    },
    /// Discard the mark and restore the preceding text and selection.
    Cancel,
    /// A native editing command resolved against the committed source.
    Command(EditCommand),
}

impl NativeEdit {
    fn text(&self) -> Option<&str> {
        match self {
            Self::Mark { text, .. } | Self::Commit { text, .. } | Self::Typing { text, .. } => {
                Some(text)
            }
            Self::Cancel | Self::Command(_) => None,
        }
    }
}

/// Text, selection, and mark currently presented by an editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorPresentation<'a> {
    /// Committed text with the pending replacement overlaid, if any.
    pub text: &'a str,
    /// Selection in presented UTF-8 bytes.
    pub selection: Selection,
    /// Marked range in presented UTF-8 bytes.
    pub marked: Option<Range<usize>>,
    /// Changes whenever text, mark, or selection changes in the presentation.
    pub revision: u64,
    /// Changes when the displayed text must be remeasured.
    pub text_revision: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Typing,
    DeleteBackward,
    DeleteForward,
    Atomic,
}

#[derive(Clone, Copy)]
struct HistoryPoint {
    selection: Selection,
    state: u64,
}

struct Delta {
    start: usize,
    removed: String,
    inserted: String,
}

struct HistoryEntry {
    edits: Vec<Delta>,
    before: HistoryPoint,
    after: HistoryPoint,
}

#[derive(PartialEq, Eq)]
struct Preedit {
    text: String,
    selection: Selection,
    marked: Range<usize>,
    // The original committed range covered by the current mark.
    target: Range<usize>,
}

struct NativeReplacement {
    range: Range<usize>,
    target: Range<usize>,
    affected: Range<usize>,
}

/// A UTF-8 editing model with independent committed and presented revisions.
pub struct Editor {
    source: String,
    mode: EditMode,
    selection: Selection,
    graphemes: OnceCell<Vec<usize>>,
    preedit: Option<Preedit>,
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    group: Option<(EditKind, Duration)>,
    committed_revision: u64,
    document_state: u64,
    presentation_revision: u64,
    text_revision: u64,
    input_revision: u64,
    session: Option<u64>,
    acknowledged: u64,
}

impl Editor {
    /// Creates an editor with an insertion point at the start.
    pub fn new(source: impl Into<String>, mode: EditMode) -> Self {
        let source = source.into();
        let source = match normalize(mode, &source) {
            Cow::Borrowed(_) => source,
            Cow::Owned(normalized) => normalized,
        };
        Self {
            source,
            mode,
            selection: Selection::default(),
            graphemes: OnceCell::new(),
            preedit: None,
            undo: Vec::new(),
            redo: Vec::new(),
            group: None,
            committed_revision: 0,
            document_state: 0,
            presentation_revision: 0,
            text_revision: 0,
            input_revision: 0,
            session: None,
            acknowledged: 0,
        }
    }

    /// The current committed source for preview, saving, and direct edits.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether this editor accepts hard line breaks.
    pub fn mode(&self) -> EditMode {
        self.mode
    }

    /// The selection in committed source, including while preedit is displayed.
    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// Increases on each committed content change, including undo and redo.
    pub fn committed_revision(&self) -> u64 {
        self.committed_revision
    }

    /// The content identity restored by undo or redo, suitable for a savepoint.
    pub fn document_state(&self) -> u64 {
        self.document_state
    }

    /// The currently focused input session, if any.
    pub fn session(&self) -> Option<u64> {
        self.session
    }

    /// Native callbacks must address this revision. Host edits supersede it.
    pub fn input_revision(&self) -> u64 {
        self.input_revision
    }

    /// Highest accepted native sequence in the current session.
    pub fn acknowledged(&self) -> u64 {
        self.acknowledged
    }

    /// Data currently shown to the writer and served to the text system.
    pub fn presentation(&self) -> EditorPresentation<'_> {
        let (text, selection, marked) =
            self.preedit
                .as_ref()
                .map_or((self.source.as_str(), self.selection, None), |preedit| {
                    (
                        preedit.text.as_str(),
                        preedit.selection,
                        Some(preedit.marked.clone()),
                    )
                });
        EditorPresentation {
            text,
            selection,
            marked,
            revision: self.presentation_revision,
            text_revision: self.text_revision,
        }
    }

    /// Opens a freshly allocated focus session and closes any previous one.
    pub fn begin_session(&mut self, session: u64) -> Result<bool, EditError> {
        if self.session == Some(session) {
            return Ok(false);
        }
        let changed = self.change(true, |editor| Ok(editor.discard_preedit()))?;
        self.session = Some(session);
        self.acknowledged = 0;
        self.group = None;
        Ok(changed)
    }

    /// Closes the session, restoring committed presentation and breaking the undo group.
    pub fn end_session(&mut self, session: u64) -> Result<bool, EditError> {
        if self.session != Some(session) {
            return Err(EditError::StaleInput);
        }
        let changed = self.change(true, |editor| Ok(editor.discard_preedit()))?;
        self.session = None;
        self.acknowledged = 0;
        self.group = None;
        Ok(changed)
    }

    /// Validates the native envelope without mutating text, selection, or acknowledgement.
    pub fn check_native(
        &self,
        session: u64,
        revision: u64,
        sequence: u64,
    ) -> Result<(), EditError> {
        if self.session == Some(session)
            && revision == self.input_revision
            && sequence > self.acknowledged
        {
            Ok(())
        } else {
            Err(EditError::StaleInput)
        }
    }

    /// Applies one native update. Later callbacks can address its resulting text
    /// with the same input revision and a higher sequence.
    pub fn apply_native(
        &mut self,
        session: u64,
        revision: u64,
        sequence: u64,
        edit: NativeEdit,
        now: Duration,
    ) -> Result<bool, EditError> {
        self.check_native(session, revision, sequence)?;
        // When the field policy alters the incoming bytes, publish a new input
        // revision to supersede the native speculative copy.
        let rebase = self.mode == EditMode::SingleLine && edit.text().is_some_and(has_line_break);
        let changed = self.change(rebase, |editor| match edit {
            NativeEdit::Mark {
                text,
                replacement,
                selection,
            } => editor.mark(&text, replacement, selection),
            NativeEdit::Commit { text, replacement } => {
                editor.commit(&text, replacement, false, now)
            }
            NativeEdit::Typing { text, replacement } => {
                editor.commit(&text, replacement, true, now)
            }
            NativeEdit::Cancel => {
                editor.group = None;
                Ok(editor.discard_preedit())
            }
            NativeEdit::Command(command) => editor.command(command, now),
        })?;
        // A policy-adjusted no-op still invalidates the speculative native copy.
        if rebase && !changed {
            self.input_revision += 1;
        }
        self.acknowledged = sequence;
        Ok(changed)
    }

    /// Sets a committed selection and closes the previous typing or deletion group.
    pub fn set_selection(&mut self, selection: Selection) -> Result<bool, EditError> {
        self.apply_command(EditCommand::Select(selection), Duration::ZERO)
    }

    /// Replaces an explicit committed byte range in one undo step.
    pub fn replace(
        &mut self,
        range: Range<usize>,
        text: &str,
        now: Duration,
    ) -> Result<bool, EditError> {
        self.change(true, |editor| {
            editor.replace_text(range, text, EditKind::Atomic, now)
        })
    }

    /// Replaces the current selection, grouping adjacent typing at a caret.
    pub fn insert(&mut self, text: &str, now: Duration) -> Result<bool, EditError> {
        self.change(true, |editor| {
            let kind = if editor.selection.is_empty() {
                EditKind::Typing
            } else {
                EditKind::Atomic
            };
            editor.replace_text(editor.selection.range(), text, kind, now)
        })
    }

    /// Deletes the selection or preceding grapheme or word.
    pub fn delete_backward(&mut self, word: bool, now: Duration) -> Result<bool, EditError> {
        self.apply_command(EditCommand::DeleteBackward { word }, now)
    }

    /// Deletes the selection or following grapheme or word.
    pub fn delete_forward(&mut self, word: bool, now: Duration) -> Result<bool, EditError> {
        self.apply_command(EditCommand::DeleteForward { word }, now)
    }

    /// Applies a command on behalf of the host and supersedes native speculation if it changes presentation.
    pub fn apply_command(
        &mut self,
        command: EditCommand,
        now: Duration,
    ) -> Result<bool, EditError> {
        self.change(true, |editor| editor.command(command, now))
    }

    /// Whether a committed undo step is available.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether a committed redo step is available.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Restores the text, selection, and savepoint identity before the last undo group.
    pub fn undo(&mut self) -> Result<bool, EditError> {
        self.change(true, |editor| editor.replay(false))
    }

    /// Reapplies the most recently undone group.
    pub fn redo(&mut self) -> Result<bool, EditError> {
        self.change(true, |editor| editor.replay(true))
    }

    /// Installs a document with an empty history, keeping the current focus session.
    pub fn reset(&mut self, source: impl Into<String>) -> Result<bool, EditError> {
        let source = source.into();
        let changed = self.change(true, |editor| {
            let source = normalize(editor.mode, &source);
            let text_changed = source != editor.source;
            if text_changed {
                let revision = next_revision(editor.committed_revision)?;
                editor.committed_revision = revision;
                editor.document_state = revision;
                editor.source = source.into_owned();
                editor.graphemes.take();
                editor.touch_text();
            }
            let mark_removed = editor.discard_preedit();
            let selection = std::mem::take(&mut editor.selection);
            editor.undo.clear();
            editor.redo.clear();
            editor.group = None;
            Ok(text_changed || mark_removed || selection != editor.selection)
        })?;
        if !changed {
            self.input_revision += 1;
        }
        Ok(changed)
    }

    fn change(
        &mut self,
        rebase: bool,
        apply: impl FnOnce(&mut Self) -> Result<bool, EditError>,
    ) -> Result<bool, EditError> {
        let presentation = next_revision(self.presentation_revision)?;
        let input = if rebase {
            next_revision(self.input_revision)?
        } else {
            self.input_revision
        };
        let changed = apply(self)?;
        if changed {
            self.presentation_revision = presentation;
            self.input_revision = input;
        }
        Ok(changed)
    }

    fn touch_text(&mut self) {
        // change() reserves this presentation revision before editing.
        self.text_revision = self.presentation_revision + 1;
    }

    fn discard_preedit(&mut self) -> bool {
        if let Some(preedit) = self.preedit.take() {
            if preedit.text != self.source {
                self.touch_text();
            }
            true
        } else {
            false
        }
    }

    fn command(&mut self, command: EditCommand, now: Duration) -> Result<bool, EditError> {
        match command {
            EditCommand::Select(selection) => {
                validate_range(&self.source, &selection.range())?;
                self.group = None;
                let mark_removed = self.discard_preedit();
                let old = std::mem::replace(&mut self.selection, selection);
                Ok(mark_removed || old != selection)
            }
            EditCommand::DeleteBackward { word } => self.delete(true, word, now),
            EditCommand::DeleteForward { word } => self.delete(false, word, now),
            EditCommand::DeleteTo(to) => {
                validate_range(&self.source, &(to..to))?;
                let range = if self.selection.is_empty() {
                    Selection::new(self.selection.active.offset, to).range()
                } else {
                    self.selection.range()
                };
                let range = self.expand_to_graphemes(range);
                self.apply_replacement(range, "", EditKind::Atomic, now)
            }
            EditCommand::InsertNewline => {
                if self.mode == EditMode::SingleLine {
                    return Ok(false);
                }
                let newline = self.source.find(['\n', '\r']).map_or("\n", |first| {
                    if self.source.as_bytes()[first] == b'\n' {
                        "\n"
                    } else if self.source.as_bytes().get(first + 1) == Some(&b'\n') {
                        "\r\n"
                    } else {
                        "\r"
                    }
                });
                self.apply_replacement(self.selection.range(), newline, EditKind::Atomic, now)
            }
        }
    }

    fn delete(&mut self, backward: bool, word: bool, now: Duration) -> Result<bool, EditError> {
        let offset = self.selection.active.offset;
        let (range, kind) = if self.selection.is_empty() {
            let (other, kind) = if backward {
                let previous = if word {
                    self.previous_word(offset)
                } else {
                    self.previous_grapheme(offset)
                };
                (previous, EditKind::DeleteBackward)
            } else {
                let next = if word {
                    self.next_word(offset)
                } else {
                    self.next_grapheme(offset)
                };
                (next, EditKind::DeleteForward)
            };
            (Selection::new(offset, other).range(), kind)
        } else {
            (self.selection.range(), EditKind::Atomic)
        };
        let range = self.expand_to_graphemes(range);
        self.apply_replacement(range, "", kind, now)
    }

    fn boundaries(&self) -> &[usize] {
        self.graphemes
            .get_or_init(|| grapheme_boundaries(&self.source).collect())
    }

    fn previous_grapheme(&self, offset: usize) -> usize {
        let boundaries = self.boundaries();
        let index = boundaries.partition_point(|&boundary| boundary < offset);
        boundaries
            .get(index.saturating_sub(1))
            .copied()
            .unwrap_or(0)
    }

    fn next_grapheme(&self, offset: usize) -> usize {
        let boundaries = self.boundaries();
        let index = boundaries.partition_point(|&boundary| boundary <= offset);
        boundaries.get(index).copied().unwrap_or(self.source.len())
    }

    fn expand_to_graphemes(&self, range: Range<usize>) -> Range<usize> {
        if range.is_empty() {
            return range;
        }
        let boundaries = self.boundaries();
        let start = boundaries.partition_point(|&boundary| boundary <= range.start) - 1;
        let end = boundaries.partition_point(|&boundary| boundary < range.end);
        boundaries[start]..boundaries[end]
    }

    fn previous_word(&self, offset: usize) -> usize {
        word_ranges(&self.source)
            .take_while(|word| word.start < offset)
            .last()
            .map_or(0, |word| word.start)
    }

    fn next_word(&self, offset: usize) -> usize {
        word_ranges(&self.source)
            .find(|word| word.end > offset)
            .map_or(self.source.len(), |word| word.end)
    }

    fn mark(
        &mut self,
        text: &str,
        replacement: Option<Range<usize>>,
        selection: Range<usize>,
    ) -> Result<bool, EditError> {
        validate_range(text, &selection)?;
        let NativeReplacement {
            range,
            target,
            affected,
        } = self.native_ranges(replacement)?;
        let (selection, text) = normalize_mark(self.mode, selection, text);
        let start = range.start;
        let mut displayed = self.presentation().text.to_owned();
        displayed.replace_range(range.clone(), &text);
        let preedit = Preedit {
            text: displayed,
            selection: Selection::new(start + selection.start, start + selection.end),
            marked: affected.start..affected.end - range.len() + text.len(),
            target,
        };
        self.group = None;
        if self.preedit.as_ref() == Some(&preedit) {
            return Ok(false);
        }
        if self.presentation().text != preedit.text {
            self.touch_text();
        }
        self.preedit = Some(preedit);
        Ok(true)
    }

    fn commit(
        &mut self,
        text: &str,
        replacement: Option<Range<usize>>,
        typing: bool,
        now: Duration,
    ) -> Result<bool, EditError> {
        let NativeReplacement {
            range,
            target,
            affected,
        } = self.native_ranges(replacement)?;
        let text = normalize(self.mode, text);
        let kind = if typing
            && self.preedit.is_none()
            && self.selection.is_empty()
            && target == self.selection.range()
        {
            EditKind::Typing
        } else {
            EditKind::Atomic
        };
        let caret = range.start + text.len();
        let mut combined;
        let text = if affected == range {
            text.as_ref()
        } else {
            combined = self.presentation().text[affected.clone()].to_owned();
            combined.replace_range(
                range.start - affected.start..range.end - affected.start,
                &text,
            );
            &combined
        };
        self.replace_at(target, text, caret, kind, now)
    }

    fn native_ranges(
        &self,
        replacement: Option<Range<usize>>,
    ) -> Result<NativeReplacement, EditError> {
        let presentation = self.presentation();
        let range = replacement.unwrap_or_else(|| {
            presentation
                .marked
                .unwrap_or_else(|| presentation.selection.range())
        });
        validate_range(presentation.text, &range)?;
        let Some(preedit) = &self.preedit else {
            return Ok(NativeReplacement {
                target: range.clone(),
                affected: range.clone(),
                range,
            });
        };
        let affected = range.start.min(preedit.marked.start)..range.end.max(preedit.marked.end);
        let target = affected.start..preedit.target.end + (affected.end - preedit.marked.end);
        Ok(NativeReplacement {
            range,
            target,
            affected,
        })
    }

    fn replace_text(
        &mut self,
        range: Range<usize>,
        text: &str,
        kind: EditKind,
        now: Duration,
    ) -> Result<bool, EditError> {
        let text = normalize(self.mode, text);
        self.apply_replacement(range, &text, kind, now)
    }

    fn apply_replacement(
        &mut self,
        range: Range<usize>,
        text: &str,
        kind: EditKind,
        now: Duration,
    ) -> Result<bool, EditError> {
        if range.is_empty() && text.is_empty() {
            validate_range(&self.source, &range)?;
            let had_mark = self.discard_preedit();
            if kind == EditKind::Atomic || had_mark {
                self.group = None;
            }
            return Ok(had_mark);
        }
        let caret = range
            .start
            .checked_add(text.len())
            .ok_or(EditError::InvalidRange)?;
        self.replace_at(range, text, caret, kind, now)
    }

    fn replace_at(
        &mut self,
        range: Range<usize>,
        text: &str,
        caret: usize,
        kind: EditKind,
        now: Duration,
    ) -> Result<bool, EditError> {
        let removed = self
            .source
            .get(range.clone())
            .ok_or(EditError::InvalidRange)?;
        let selection = Selection::at(Caret::with_affinity(caret, Affinity::Upstream));
        if removed == text {
            let old = std::mem::replace(&mut self.selection, selection);
            let had_mark = self.discard_preedit();
            if kind == EditKind::Atomic || old != selection || had_mark {
                self.group = None;
            }
            return Ok(had_mark || old != selection);
        }

        let revision = next_revision(self.committed_revision)?;
        let before = HistoryPoint {
            selection: self.selection,
            state: self.document_state,
        };
        let delta = Delta {
            start: range.start,
            removed: removed.to_owned(),
            inserted: text.to_owned(),
        };
        let adjacent = match kind {
            EditKind::Typing | EditKind::DeleteForward => {
                range.start == before.selection.active.offset
            }
            EditKind::DeleteBackward => range.end == before.selection.active.offset,
            EditKind::Atomic => false,
        };
        let joins_group = adjacent
            && before.selection.is_empty()
            && self.preedit.is_none()
            && self.group.is_some_and(|(last_kind, last_time)| {
                kind == last_kind
                    && now
                        .checked_sub(last_time)
                        .is_some_and(|elapsed| elapsed <= TYPING_GROUP_INTERVAL)
            });
        self.source.replace_range(range, text);
        self.graphemes.take();
        self.touch_text();
        self.selection = selection;
        self.preedit = None;
        self.committed_revision = revision;
        self.document_state = revision;
        self.redo.clear();
        let after = HistoryPoint {
            selection,
            state: revision,
        };
        if joins_group && let Some(entry) = self.undo.last_mut() {
            entry.edits.push(delta);
            entry.after = after;
        } else {
            self.undo.push(HistoryEntry {
                edits: vec![delta],
                before,
                after,
            });
        }
        self.group = (kind != EditKind::Atomic).then_some((kind, now));
        Ok(true)
    }

    fn replay(&mut self, redo: bool) -> Result<bool, EditError> {
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        if from.is_empty() {
            self.group = None;
            return Ok(self.discard_preedit());
        }
        let revision = next_revision(self.committed_revision)?;
        let Some(entry) = from.pop() else {
            return Ok(false);
        };
        // These deltas can only be replayed from their own adjacent endpoint.
        // Source mutation is private, and a new edit clears the redo branch.
        let point = if redo {
            for edit in &entry.edits {
                self.source
                    .replace_range(edit.start..edit.start + edit.removed.len(), &edit.inserted);
            }
            entry.after
        } else {
            for edit in entry.edits.iter().rev() {
                self.source
                    .replace_range(edit.start..edit.start + edit.inserted.len(), &edit.removed);
            }
            entry.before
        };
        to.push(entry);
        self.graphemes.take();
        self.touch_text();
        self.selection = point.selection;
        self.preedit = None;
        self.group = None;
        self.committed_revision = revision;
        self.document_state = point.state;
        Ok(true)
    }
}

fn next_revision(revision: u64) -> Result<u64, EditError> {
    revision.checked_add(1).ok_or(EditError::RevisionExhausted)
}

fn validate_range(text: &str, range: &Range<usize>) -> Result<(), EditError> {
    text.get(range.clone())
        .map(|_| ())
        .ok_or(EditError::InvalidRange)
}

fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

fn has_line_break(text: &str) -> bool {
    text.contains(is_line_break)
}

fn normalize(mode: EditMode, text: &str) -> Cow<'_, str> {
    if mode != EditMode::SingleLine || !has_line_break(text) {
        return Cow::Borrowed(text);
    }
    let mut normalized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' && chars.peek() == Some(&'\n') {
            chars.next();
        }
        normalized.push(if is_line_break(c) { ' ' } else { c });
    }
    Cow::Owned(normalized)
}

fn normalize_mark(mode: EditMode, range: Range<usize>, text: &str) -> (Range<usize>, Cow<'_, str>) {
    let normalized = normalize(mode, text);
    let range = if matches!(&normalized, Cow::Owned(_)) {
        normalize(mode, &text[..range.start]).len()..normalize(mode, &text[..range.end]).len()
    } else {
        range
    };
    (range, normalized)
}

pub(super) fn grapheme_boundaries(text: &str) -> impl Iterator<Item = usize> + '_ {
    GraphemeClusterSegmenter::new().segment_str(text)
}

pub(super) fn word_ranges(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    static WORDS: OnceLock<WordSegmenterBorrowed<'static>> = OnceLock::new();
    let mut previous = 0;
    WORDS
        .get_or_init(|| WordSegmenter::new_dictionary(Default::default()))
        .segment_str(text)
        .iter_with_word_type()
        .filter_map(move |(end, kind)| {
            let start = std::mem::replace(&mut previous, end);
            (start < end && kind.is_word_like()).then_some(start..end)
        })
}

pub(super) fn paragraph_ranges(text: &str) -> impl Iterator<Item = (Range<usize>, usize)> + '_ {
    let mut start = 0;
    let mut chars = text.char_indices();
    let mut finished = false;
    std::iter::from_fn(move || {
        if finished {
            return None;
        }
        for (index, c) in chars.by_ref() {
            if is_line_break(c) {
                let consumed = if c == '\r' && text.as_bytes().get(index + 1) == Some(&b'\n') {
                    2
                } else {
                    c.len_utf8()
                };
                let content_start = std::mem::replace(&mut start, index + consumed);
                // Consume the LF of CRLF before inspecting the next paragraph.
                if consumed == 2 && c == '\r' {
                    chars.next();
                }
                return Some((content_start..index, start));
            }
        }
        finished = true;
        Some((start..text.len(), text.len()))
    })
}
