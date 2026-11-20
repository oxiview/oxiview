//! The main-thread projection of a focused editor. This module does not call AppKit.

use objc2_foundation::{NSNotFound, NSRange};
use oxiview_base::{Point, Rect};
use oxiview_platform::{TextCommitKind, TextInputEdit, TextInputEvent, TextInputSnapshot};
use std::{collections::VecDeque, ops::Range, time::Duration};

pub(super) const MISSING: NSRange = NSRange::new(NSNotFound as usize, 0);

pub(super) fn byte_range(text: &str, native: NSRange, clip_end: bool) -> Option<Range<usize>> {
    if native.location == MISSING.location {
        return None;
    }
    super::utf16::bytes(text, native.location, native.length, clip_end)
}

pub(super) fn native_range(text: &str, range: Range<usize>) -> Option<NSRange> {
    let (start, length) = super::utf16::units(text, range)?;
    Some(NSRange::new(start, length))
}

#[derive(Clone)]
struct BeforeMark {
    replaced: String,
    selection: Range<usize>,
    caret: Rect,
}

struct Projection {
    snapshot: TextInputSnapshot,
    before_mark: Option<BeforeMark>,
}
impl Projection {
    fn can_replace(&self, range: &Range<usize>) -> bool {
        self.snapshot.text.get(range.clone()).is_some()
            && self
                .snapshot
                .marked
                .as_ref()
                .is_none_or(|marked| marked.start <= range.start && range.end <= marked.end)
    }

    fn apply(&mut self, edit: &TextInputEdit) -> Option<()> {
        let snapshot = &mut self.snapshot;
        match edit {
            TextInputEdit::SetMarked {
                range,
                text,
                selection,
            } => {
                if !self.can_replace(range) || text.get(selection.clone()).is_none() {
                    return None;
                }
                let snapshot = &mut self.snapshot;
                let mark = snapshot.marked.as_ref().map_or(range.clone(), Clone::clone);
                let end = (mark.end - range.len()).checked_add(text.len())?;
                if snapshot.marked.is_none() {
                    self.before_mark = Some(BeforeMark {
                        replaced: snapshot.text.get(range.clone())?.to_owned(),
                        selection: snapshot.selection.clone(),
                        caret: snapshot.caret,
                    });
                }
                let caret = first_rect(snapshot, range.start..range.start).1;
                snapshot.text.replace_range(range.clone(), text);
                snapshot.marked = Some(mark.start..end);
                snapshot.selection = range.start + selection.start..range.start + selection.end;
                snapshot.caret = caret;
                snapshot.geometry.clear();
            }
            TextInputEdit::Commit { range, text, .. } => {
                if !self.can_replace(range) {
                    return None;
                }
                let snapshot = &mut self.snapshot;
                let end = range.start.checked_add(text.len())?;
                let caret = first_rect(snapshot, range.start..range.start).1;
                snapshot.text.replace_range(range.clone(), text);
                snapshot.marked = None;
                self.before_mark = None;
                snapshot.selection = end..end;
                snapshot.caret = caret;
                snapshot.geometry.clear();
            }
            TextInputEdit::Cancel => {
                let range = snapshot.marked.clone()?;
                let original = self.before_mark.take()?;
                snapshot.text.replace_range(range, &original.replaced);
                snapshot.marked = None;
                snapshot.selection = original.selection;
                snapshot.caret = original.caret;
                snapshot.geometry.clear();
            }
            // The window thread resolves movement/deletion against its layout.
            TextInputEdit::Command(_) => {}
            _ => return None,
        }
        Some(())
    }
}

struct Pending {
    sequence: u64,
    edit: TextInputEdit,
    before_mark: Option<BeforeMark>,
}

pub(super) struct TextState {
    projected: Projection,
    pending: VecDeque<Pending>,
    acknowledged: u64,
    acknowledged_before_mark: Option<BeforeMark>,
    sequence: u64,
}

#[derive(Default)]
pub(super) struct SnapshotChange {
    pub(super) invalidate_coordinates: bool,
    pub(super) discard_mark: bool,
}

impl TextState {
    pub(super) fn new(snapshot: TextInputSnapshot) -> Self {
        let acknowledged = snapshot.acknowledged;
        Self {
            projected: Projection {
                snapshot,
                before_mark: None,
            },
            pending: VecDeque::new(),
            acknowledged,
            acknowledged_before_mark: None,
            sequence: acknowledged,
        }
    }

    pub(super) fn snapshot(&self) -> &TextInputSnapshot {
        &self.projected.snapshot
    }

    pub(super) fn set_snapshot(&mut self, snapshot: TextInputSnapshot) -> SnapshotChange {
        if snapshot.session != self.snapshot().session
            || snapshot.revision < self.snapshot().revision
            || snapshot.acknowledged < self.acknowledged
        {
            return SnapshotChange::default();
        }
        let had_mark = self.snapshot().marked.is_some();
        let newer_revision = snapshot.revision != self.snapshot().revision;
        let invalidate_coordinates = newer_revision
            || self.snapshot().caret != snapshot.caret
            || self.snapshot().selection != snapshot.selection
            || self.snapshot().marked != snapshot.marked
            || self.snapshot().geometry != snapshot.geometry;
        self.acknowledged = snapshot.acknowledged;
        self.sequence = self.sequence.max(self.acknowledged);
        while self
            .pending
            .front()
            .is_some_and(|pending| pending.sequence <= self.acknowledged)
        {
            self.acknowledged_before_mark = self
                .pending
                .pop_front()
                .and_then(|pending| pending.before_mark);
        }
        if snapshot.marked.is_none() {
            self.acknowledged_before_mark = None;
        }
        if newer_revision {
            self.pending.clear();
        }
        self.projected = Projection {
            snapshot,
            before_mark: self.acknowledged_before_mark.clone(),
        };
        for pending in &mut self.pending {
            if self.projected.apply(&pending.edit).is_some() {
                pending.before_mark = self.projected.before_mark.clone();
            }
        }
        SnapshotChange {
            invalidate_coordinates,
            discard_mark: had_mark && self.snapshot().marked.is_none() && newer_revision,
        }
    }

    pub(super) fn mark(
        &mut self,
        text: String,
        selected: NSRange,
        replacement: NSRange,
        timestamp: Duration,
    ) -> Option<TextInputEvent> {
        let selection = byte_range(&text, selected, false)?;
        let range = self.replacement(replacement, true)?;
        if text.is_empty() {
            let mark = self.snapshot().marked.as_ref()?;
            if mark == &range {
                return self.edit(TextInputEdit::Cancel, timestamp);
            }
        }
        self.edit(
            TextInputEdit::SetMarked {
                range,
                text,
                selection,
            },
            timestamp,
        )
    }

    pub(super) fn commit(
        &mut self,
        text: String,
        replacement: NSRange,
        timestamp: Duration,
    ) -> Option<TextInputEvent> {
        let range = self.replacement(replacement, false)?;
        let kind = if self.snapshot().marked.is_some() || replacement.location != MISSING.location {
            TextCommitKind::Composition
        } else {
            TextCommitKind::Typing
        };
        self.edit(TextInputEdit::Commit { range, text, kind }, timestamp)
    }

    pub(super) fn unmark(&mut self, timestamp: Duration) -> Option<TextInputEvent> {
        let range = self.snapshot().marked.clone()?;
        let text = self.snapshot().text.get(range.clone())?.to_owned();
        self.edit(
            TextInputEdit::Commit {
                range,
                text,
                kind: TextCommitKind::Composition,
            },
            timestamp,
        )
    }

    pub(super) fn edit(
        &mut self,
        edit: TextInputEdit,
        timestamp: Duration,
    ) -> Option<TextInputEvent> {
        let sequence = self.sequence.checked_add(1)?;
        self.projected.apply(&edit)?;
        self.sequence = sequence;
        self.pending.push_back(Pending {
            sequence,
            edit: edit.clone(),
            before_mark: self.projected.before_mark.clone(),
        });
        let snapshot = self.snapshot();
        Some(TextInputEvent {
            session: snapshot.session,
            revision: snapshot.revision,
            sequence,
            timestamp,
            edit,
        })
    }

    fn replacement(&self, range: NSRange, relative_to_mark: bool) -> Option<Range<usize>> {
        let snapshot = self.snapshot();
        if range.location == MISSING.location {
            return (range.length == 0).then(|| {
                snapshot
                    .marked
                    .as_ref()
                    .unwrap_or(&snapshot.selection)
                    .clone()
            });
        }
        // Cocoa's setMarkedText: replacement is based at the start of the mark.
        // insertText:replacementRange: addresses the text storage itself.
        if relative_to_mark && let Some(mark) = &snapshot.marked {
            let units = snapshot.text.get(..mark.start)?.encode_utf16().count();
            let range = NSRange::new(units.checked_add(range.location)?, range.length);
            return byte_range(&snapshot.text, range, false);
        }
        byte_range(&snapshot.text, range, false)
    }
}

fn same_visual_line(a: Rect, b: Rect) -> bool {
    let [_, a_top, _, a_bottom] = a.edges();
    let [_, b_top, _, b_bottom] = b.edges();
    a_top < b_bottom && b_top < a_bottom
}

fn union(a: Rect, b: Rect) -> Rect {
    let [x0, y0, x1, y1] = a.edges();
    let [u0, v0, u1, v1] = b.edges();
    Rect::new(x0.min(u0), y0.min(v0), x1.max(u1), y1.max(v1))
}

pub(super) fn first_rect(
    snapshot: &TextInputSnapshot,
    range: Range<usize>,
) -> (Range<usize>, Rect) {
    let [x0, y0, _, y1] = snapshot.caret.edges();
    if range.is_empty() {
        if range == snapshot.selection {
            return (range, Rect::new(x0, y0, x0, y1));
        }
        if let Some(fragment) = snapshot
            .geometry
            .iter()
            .find(|fragment| fragment.range == range)
        {
            return (range, fragment.rect);
        }
        if range.start != snapshot.selection.end
            && let Some(fragment) = snapshot
                .geometry
                .iter()
                .find(|fragment| fragment.range.contains(&range.start))
        {
            let [x, y0, _, y1] = fragment.rect.edges();
            return (range, Rect::new(x, y0, x, y1));
        }
        return (range, Rect::new(x0, y0, x0, y1));
    }
    let mut fragments = snapshot.geometry.iter().filter_map(|fragment| {
        let start = range.start.max(fragment.range.start);
        let end = range.end.min(fragment.range.end);
        (start < end).then_some((start..end, fragment.rect))
    });
    if let Some((mut actual, mut rect)) = fragments.next() {
        for (fragment, next) in fragments {
            if fragment.start > actual.end || !same_visual_line(rect, next) {
                break;
            }
            actual.end = actual.end.max(fragment.end);
            rect = union(rect, next);
        }
        return (actual, rect);
    }
    // No layout is available for a speculative edit. Use the nearest known
    // caret until the window thread publishes the newly shaped geometry.
    (range.start..range.start, Rect::new(x0, y0, x0, y1))
}

pub(super) fn index_at_point(snapshot: &TextInputSnapshot, point: Point) -> Option<usize> {
    let contains = |rect: Rect| {
        let [x0, y0, x1, y1] = rect.edges();
        point.x() >= x0 && point.x() <= x1 && point.y() >= y0 && point.y() < y1
    };
    snapshot
        .geometry
        .iter()
        .find(|fragment| !fragment.range.is_empty() && contains(fragment.rect))
        .map(|fragment| fragment.range.start)
        .or_else(|| contains(snapshot.caret).then_some(snapshot.selection.end))
}

#[cfg(test)]
#[allow(clippy::expect_used)] // Fixture ranges and projected edits must be valid.
mod tests {
    use super::*;
    use oxiview_platform::{TextInputGeometry, TextInputSession};

    fn snapshot() -> TextInputSnapshot {
        TextInputSnapshot {
            session: TextInputSession(17),
            revision: 3,
            acknowledged: 0,
            text: "a😀旧\nline".into(),
            selection: 5..8,
            marked: None,
            caret: Rect::new(20.0, 12.0, 21.0, 30.0),
            geometry: vec![
                TextInputGeometry {
                    range: 0..5,
                    rect: Rect::new(2.0, 12.0, 14.0, 30.0),
                },
                TextInputGeometry {
                    range: 5..8,
                    rect: Rect::new(14.0, 12.0, 20.0, 30.0),
                },
                TextInputGeometry {
                    range: 9..13,
                    rect: Rect::new(2.0, 30.0, 28.0, 48.0),
                },
            ],
        }
    }

    #[test]
    fn utf16_ranges_reject_surrogate_splits_and_overflow() {
        let text = "a😀旧\0é";
        assert_eq!(native_range(text, 5..8), Some(NSRange::new(3, 1)));
        assert_eq!(byte_range(text, NSRange::new(1, 2), false), Some(1..5));
        assert_eq!(byte_range(text, NSRange::new(3, 50), true), Some(5..11));
        assert_eq!(byte_range(text, NSRange::new(6, 0), false), Some(11..11));
        for invalid in [
            MISSING,
            NSRange::new(2, 0),
            NSRange::new(1, 1),
            NSRange::new(2, 50),
            NSRange::new(20, 0),
            NSRange::new(3, usize::MAX),
        ] {
            assert_eq!(byte_range(text, invalid, false), None);
            assert_eq!(byte_range(text, invalid, true), None);
        }
        assert_eq!(native_range(text, 2..5), None);
    }

    #[test]
    fn ascii_and_unicode_ranges_round_trip_at_every_scalar_boundary() {
        for text in ["", "abc", "👩🏽‍💻", "\u{301}aع\0\r\n𝓓漢字"] {
            let boundaries: Vec<_> = text
                .char_indices()
                .map(|(byte, _)| byte)
                .chain([text.len()])
                .collect();
            for (index, &start) in boundaries.iter().enumerate() {
                for &end in boundaries.iter().skip(index) {
                    let native = native_range(text, start..end).expect("valid byte range");
                    assert_eq!(byte_range(text, native, false), Some(start..end));
                }
            }
        }
    }

    #[test]
    fn partial_mark_updates_remain_visible_and_cancel_restores_selection() {
        let baseline = snapshot();
        let mut state = TextState::new(baseline.clone());
        let marked = state
            .mark("nih".into(), NSRange::new(3, 0), MISSING, Duration::ZERO)
            .expect("started composition");
        assert_eq!(
            (marked.session, marked.revision, marked.sequence),
            (baseline.session, 3, 1)
        );
        assert_eq!(state.snapshot().text, "a😀nih\nline");
        assert_eq!(state.snapshot().marked, Some(5..8));

        let updated = state
            .mark(
                "ihao".into(),
                NSRange::new(4, 0),
                NSRange::new(1, 2),
                Duration::ZERO,
            )
            .expect("updated range relative to the mark");
        assert_eq!(
            updated.edit,
            TextInputEdit::SetMarked {
                range: 6..8,
                text: "ihao".into(),
                selection: 4..4
            }
        );
        assert_eq!(state.snapshot().text, "a😀nihao\nline");
        assert_eq!(state.snapshot().marked, Some(5..10));

        let mut accepted = baseline.clone();
        accepted.acknowledged = 1;
        accepted.text = "a😀nih\nline".into();
        accepted.marked = Some(5..8);
        accepted.selection = 8..8;
        state.set_snapshot(accepted);
        assert_eq!(state.snapshot().text, "a😀nihao\nline");
        assert!(
            state
                .commit("bad".into(), NSRange::new(2, 1), Duration::ZERO)
                .is_none()
        );
        assert!(
            state
                .commit("outside".into(), NSRange::new(0, 1), Duration::ZERO)
                .is_none()
        );
        assert_eq!(state.snapshot().text, "a😀nihao\nline");

        let cancel = state
            .mark(String::new(), NSRange::new(0, 0), MISSING, Duration::ZERO)
            .expect("clearing composition cancels");
        assert_eq!(cancel.sequence, 3);
        assert_eq!(cancel.edit, TextInputEdit::Cancel);
        assert_eq!(state.snapshot().text, baseline.text);
        assert_eq!(state.snapshot().selection, baseline.selection);
        assert_eq!(state.snapshot().marked, None);
    }

    #[test]
    fn unmark_commits_and_new_revision_supersedes_projection() {
        let original = snapshot();
        let mut state = TextState::new(original.clone());
        state
            .mark("候補".into(), NSRange::new(2, 0), MISSING, Duration::ZERO)
            .expect("mark");
        let event = state.unmark(Duration::ZERO).expect("unmark commits");
        assert_eq!(
            event.edit,
            TextInputEdit::Commit {
                range: 5..11,
                text: "候補".into(),
                kind: TextCommitKind::Composition
            }
        );
        assert_eq!(state.snapshot().text, "a😀候補\nline");
        assert_eq!(state.snapshot().selection, 11..11);
        assert_eq!(state.snapshot().marked, None);

        state
            .mark("ni".into(), NSRange::new(2, 0), MISSING, Duration::ZERO)
            .expect("a new mark");
        let mut host = original.clone();
        host.revision = 4;
        host.selection = 0..0;
        let change = state.set_snapshot(host);
        assert!(change.discard_mark && change.invalidate_coordinates);

        state.set_snapshot(original);
        assert_eq!(state.snapshot().revision, 4);
        assert_eq!(state.snapshot().selection, 0..0);
        let current = state
            .commit("q".into(), MISSING, Duration::ZERO)
            .expect("type after rebase");
        assert_eq!((current.sequence, current.revision), (4, 4));
        assert_eq!(state.snapshot().text, "qa😀旧\nline");
    }

    #[test]
    fn a_range_spanning_lines_returns_the_first_visual_fragment() {
        let snapshot = snapshot();
        assert_eq!(
            first_rect(&snapshot, 1..13),
            (1..8, Rect::new(2.0, 12.0, 20.0, 30.0))
        );
        assert_eq!(index_at_point(&snapshot, Point::new(15.0, 17.0)), Some(5));
        assert_eq!(index_at_point(&snapshot, Point::new(3.0, 31.0)), Some(9));
        assert_eq!(index_at_point(&snapshot, Point::new(3.0, 80.0)), None);
    }
}
