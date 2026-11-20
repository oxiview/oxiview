//! Owned text-input and desktop-service messages shared by the window thread
//! and platform adapters. Source file bytes and editor models stay on the
//! application side of the port.

use oxiview_base::Rect;
use std::{ops::Range, time::Duration};

/// Identity of one continuous focus lifetime in a window.
///
/// Allocate a fresh value when an editor gains focus, even for the same field.
/// Never reuse an identity within the window. An event for a different session
/// is obsolete and must not be applied to the focused editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextInputSession(pub u64);

/// Identity allocated by the application for one outstanding desktop request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RequestId(pub u64);

/// Editor services available on a window endpoint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EditorCapabilities {
    /// The backend accepts snapshots and delivers owned text input.
    pub text_input: bool,
    /// The backend supports plain-text clipboard requests.
    pub clipboard: bool,
    /// The backend supports single-file Open and Save requests.
    pub file_panels: bool,
}

impl EditorCapabilities {
    /// All editor services implemented by the macOS and headless adapters.
    pub const fn all() -> Self {
        Self {
            text_input: true,
            clipboard: true,
            file_panels: true,
        }
    }
}

/// A fragment of text geometry in one visual line.
#[derive(Clone, Debug, PartialEq)]
pub struct TextInputGeometry {
    /// Byte range in the snapshot's displayed text. An empty range is a caret.
    pub range: Range<usize>,
    /// Top-left-origin coordinates in logical units relative to the content view.
    pub rect: Rect,
}

/// The window thread's current view of one focused editor.
///
/// Strings and ranges describe the displayed text, including preedit. Endpoints
/// convert native offsets at the platform boundary. During a native input turn the
/// backend can apply queued callbacks to this view before the editor receives
/// them, so synchronous queries read the pending presentation.
#[derive(Clone, Debug, PartialEq)]
pub struct TextInputSnapshot {
    /// Current focus lifetime.
    pub session: TextInputSession,
    /// Input-state revision, independent of the committed document's revision.
    /// Advance it on text or selection changes outside accepted native input.
    /// Accepted native callbacks advance `acknowledged` on the same revision.
    pub revision: u64,
    /// Highest processed native sequence in this session, or zero. Gaps are
    /// allowed when earlier sequences belonged to an obsolete revision.
    pub acknowledged: u64,
    /// Surrounding text as displayed, including the marked replacement.
    pub text: String,
    /// Ordered UTF-8 byte range of the presented selection.
    pub selection: Range<usize>,
    /// UTF-8 byte range of inline preedit, if present.
    pub marked: Option<Range<usize>>,
    /// Active caret in top-left-origin logical content coordinates.
    pub caret: Rect,
    /// Optional geometry in logical text order. Split fragments at visual line
    /// boundaries. Smaller fragments and caret entries improve point queries.
    pub geometry: Vec<TextInputGeometry>,
}

impl TextInputSnapshot {
    /// Checks UTF-8 boundaries and that every rect is finite and ordered.
    pub fn is_valid(&self) -> bool {
        self.text.get(self.selection.clone()).is_some()
            && self.marked.as_ref().is_none_or(|marked| {
                self.text.get(marked.clone()).is_some()
                    && marked.start <= self.selection.start
                    && self.selection.end <= marked.end
            })
            && valid_rect(self.caret)
            && self.geometry.iter().all(|fragment| {
                self.text.get(fragment.range.clone()).is_some() && valid_rect(fragment.rect)
            })
    }
}

fn valid_rect(rect: Rect) -> bool {
    let [x0, y0, x1, y1] = rect.edges();
    [x0, y0, x1, y1, x1 - x0, y1 - y0]
        .into_iter()
        .all(f64::is_finite)
        && x0 <= x1
        && y0 <= y1
}

/// One ordered native callback, scoped to the captured focus and input revision.
///
/// Sequences start at one for each session. Multiple callbacks from one native
/// turn can share a revision and later callbacks can address earlier edits in
/// that turn. Discard obsolete callbacks and validate replacements in the
/// editor before changing state. If field policy changes the resulting text,
/// publish a new revision so dependent callbacks cannot use the old ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextInputEvent {
    /// Focus lifetime captured by the producing text input client.
    pub session: TextInputSession,
    /// Input-state revision from which the native callback originated.
    pub revision: u64,
    /// Arrival order within this session.
    pub sequence: u64,
    /// Monotonic input capture time.
    pub timestamp: Duration,
    /// Callback payload.
    pub edit: TextInputEdit,
}

/// An owned native editing operation. Replacement ranges index the displayed
/// UTF-8 text immediately before that callback. A replacement within a mark
/// retains the committed document's original target until commit or cancel.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextInputEdit {
    /// Start or update inline preedit without changing the committed document.
    SetMarked {
        /// Range replaced in the preceding presentation.
        range: Range<usize>,
        /// Replacement marked text.
        text: String,
        /// Selection relative to the replacement's UTF-8 bytes.
        selection: Range<usize>,
    },
    /// Replace the range in one committed edit.
    Commit {
        /// Range replaced in the preceding presentation.
        range: Range<usize>,
        /// Confirmed text.
        text: String,
        /// Distinguishes ordinary typing from a complete IME undo step.
        kind: TextCommitKind,
    },
    /// Discard the pending mark and restore the preedit target and selection.
    Cancel,
    /// A semantic command generated by the operating system's text bindings.
    Command(TextEditingCommand),
}

/// Origin of a committed text edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCommitKind {
    /// Direct typing, eligible for adjacent typing groups.
    Typing,
    /// Input-method confirmation, which delimits a typing group.
    Composition,
}

/// Editing commands translated from system text bindings. Ordinary application
/// shortcuts and unclaimed physical keys are still delivered through Keymaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextEditingCommand {
    /// Move visually left.
    MoveLeft {
        /// Extend the selection from its anchor.
        select: bool,
    },
    /// Move visually right.
    MoveRight {
        /// Extend the selection from its anchor.
        select: bool,
    },
    /// Move to the preceding visual line.
    MoveUp {
        /// Extend the selection from its anchor.
        select: bool,
    },
    /// Move to the next visual line.
    MoveDown {
        /// Extend the selection from its anchor.
        select: bool,
    },
    /// Move left a word.
    MoveWordLeft {
        /// Extend the selection from its anchor.
        select: bool,
    },
    /// Move right a word.
    MoveWordRight {
        /// Extend the selection from its anchor.
        select: bool,
    },
    /// Move to the beginning of the line.
    MoveLineStart {
        /// Extend the selection from its anchor.
        select: bool,
    },
    /// Move to the end of the line.
    MoveLineEnd {
        /// Extend the selection from its anchor.
        select: bool,
    },
    /// Move to the beginning of the document.
    MoveDocumentStart {
        /// Extend the selection from its anchor.
        select: bool,
    },
    /// Move to the end of the document.
    MoveDocumentEnd {
        /// Extend the selection from its anchor.
        select: bool,
    },
    /// Delete the selection or the preceding grapheme.
    DeleteBackward,
    /// Delete the selection or the following grapheme.
    DeleteForward,
    /// Delete the selection or the preceding word.
    DeleteWordBackward,
    /// Delete the selection or the following word.
    DeleteWordForward,
    /// Delete the selection or text back to the beginning of the line.
    DeleteToLineStart,
    /// Delete the selection or text to the end of the line.
    DeleteToLineEnd,
    /// Insert a line break, subject to the focused field's editing policy.
    InsertNewline,
}

/// Request a plain-text clipboard operation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClipboardRequest {
    /// Read the current plain-text contents, if any.
    Read {
        /// Application-assigned identity.
        id: RequestId,
    },
    /// Replace the contents with the supplied plain text.
    Write {
        /// Application-assigned identity.
        id: RequestId,
        /// UTF-8 contents.
        text: String,
    },
}

impl ClipboardRequest {
    /// Returns the identity to use for the reply.
    pub const fn id(&self) -> RequestId {
        match self {
            Self::Read { id } | Self::Write { id, .. } => *id,
        }
    }
}

/// Successful clipboard reply, tagged in the containing event.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClipboardResponse {
    /// Current plain text, if available.
    Read(Option<String>),
    /// The clipboard accepted the text. A pending cut may now edit the source.
    Written,
}

/// Request for one native Open or Save panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePanelRequest {
    /// Application-assigned identity.
    pub id: RequestId,
    /// Kind and initial contents of the panel.
    pub kind: FilePanelKind,
}

/// A panel chooses one path. The application owns file reads and writes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FilePanelKind {
    /// Choose one existing file.
    Open,
    /// Choose a destination file.
    Save {
        /// Optional initial file name shown to the user.
        suggested_name: Option<String>,
    },
}

/// Window-thread work to send to the platform at the end of an application turn.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PlatformCommand {
    /// Publish the current editor, or end the current focus lifetime.
    TextInput(Option<TextInputSnapshot>),
    /// Read or write system plain text.
    Clipboard(ClipboardRequest),
    /// Show a single-file panel.
    FilePanel(FilePanelRequest),
    /// Close this window after the application resolved any outstanding intent.
    Close,
}
