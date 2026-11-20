//! Commands that edit native fields, interact with the clipboard, or search source.

use super::*;
use crate::commands::Command::*;
use std::ops::Range;

#[derive(Default)]
pub(super) struct Matches {
    revisions: Option<(u64, u64)>,
    ranges: Vec<Range<usize>>,
}

impl Matches {
    fn find(&mut self, source: &Editor, query: &Editor) -> &[Range<usize>] {
        let revisions = (source.committed_revision(), query.committed_revision());
        if self.revisions != Some(revisions) {
            self.ranges.clear();
            self.ranges.extend(
                source
                    .source()
                    .match_indices(query.source())
                    .map(|(i, value)| i..i + value.len()),
            );
            self.revisions = Some(revisions);
        }
        &self.ranges
    }
}

impl EditorApp {
    pub(super) fn perform(&mut self, command: Command, waker: &WindowWaker) -> Result<(), Error> {
        if self.file_actions.prompting()
            && !matches!(command, Save | SaveAs | Discard | KeepWriting | Dismiss)
        {
            return Ok(());
        }
        match command {
            New => self.request_intent(Intent::New)?,
            Open => self.request_intent(Intent::Open)?,
            Close => self.request_intent(Intent::Close)?,
            Save | SaveAs => self.save(command == SaveAs, waker)?,
            KeepWriting | Dismiss if self.file_actions.prompting() => {
                self.clear_prompt()?;
                self.focus_source()?;
                self.notice = "Ready".into();
            }
            Discard => self.discard_changes()?,
            Find => {
                self.find_open = true;
                self.tree
                    .set_style(self.ui.slots.find.get()?, view::command_bar(true, false))?;
                self.focus(&self.ui.query.clone())?;
            }
            Dismiss => {
                if !self.cancel_mark()? && self.find_open {
                    self.find_open = false;
                    self.tree
                        .set_style(self.ui.slots.find.get()?, view::command_bar(false, false))?;
                    self.focus_source()?;
                }
            }
            Next | Previous => self.find_match(command == Previous)?,
            Replace | ReplaceAll => self.replace_matches(command == ReplaceAll)?,
            Bold | Italic => {
                let marker = if command == Bold { "**" } else { "*" };
                let model = self.source();
                let mut editor = model.borrow_mut();
                let range = editor.selection().range();
                if let Some(value) = editor.source().get(range.clone()) {
                    let count = value.len();
                    let insertion = format!("{marker}{value}{marker}");
                    let begin = range.start + marker.len();
                    editor.replace(range, &insertion, self.now).map_err(error)?;
                    editor
                        .set_selection(Selection::new(begin, begin + count))
                        .map_err(error)?;
                }
                drop(editor);
                self.refresh(&self.ui.source.clone())?;
                self.focus_source()?;
            }
            Copy | Cut | Paste => self.clipboard(command)?,
            KeepWriting => {}
        }
        Ok(())
    }

    fn cancel_mark(&mut self) -> Result<bool, Error> {
        let Some(field) = self.field() else {
            return Ok(false);
        };
        let model = field.editor();
        let mut editor = model.borrow_mut();
        if editor.presentation().marked.is_none() {
            return Ok(false);
        }
        let selection = editor.selection();
        editor.set_selection(selection).map_err(error)?;
        drop(editor);
        self.refresh(&field)?;
        Ok(true)
    }

    fn find_match(&mut self, previous: bool) -> Result<(), Error> {
        let query_model = self.ui.query.editor();
        let query = query_model.borrow();
        if query.source().is_empty() {
            self.notice = "Enter text to find.".into();
            return Ok(());
        }
        let model = self.source();
        let mut editor = model.borrow_mut();
        let selected = editor.selection().range();
        let ranges = self.matches.find(&editor, &query);
        if ranges.is_empty() {
            self.notice = "No matches.".into();
            return Ok(());
        }
        let index = if previous {
            ranges
                .partition_point(|r| r.end <= selected.start)
                .checked_sub(1)
                .unwrap_or(ranges.len() - 1)
        } else {
            ranges.partition_point(|r| r.start < selected.end) % ranges.len()
        };
        let range = &ranges[index];
        let changed = editor
            .set_selection(Selection::new(range.start, range.end))
            .map_err(error)?;
        self.notice = format!("Match {} of {}", index + 1, ranges.len());
        drop(editor);
        if changed {
            self.refresh(&self.ui.source.clone())?;
        }
        Ok(())
    }
    fn replace_matches(&mut self, all: bool) -> Result<(), Error> {
        let query = self.ui.query.editor().borrow().source().to_owned();
        let replacement = self.ui.replacement.editor().borrow().source().to_owned();
        if query.is_empty() {
            self.notice = "Enter text to find.".into();
            return Ok(());
        }
        if !all {
            let model = self.source();
            let editor = model.borrow();
            if editor.source().get(editor.selection().range()) != Some(&query) {
                drop(editor);
                self.find_match(false)?;
            }
        }
        let model = self.source();
        let mut editor = model.borrow_mut();
        if all {
            let count = editor.source().matches(&query).count();
            if count > 0 {
                let result = editor.source().replace(&query, &replacement);
                let len = editor.source().len();
                editor.replace(0..len, &result, self.now).map_err(error)?;
            }
            self.notice = format!("Replaced {count} matches.");
        } else {
            let range = editor.selection().range();
            if editor.source().get(range.clone()) == Some(&query) {
                editor
                    .replace(range, &replacement, self.now)
                    .map_err(error)?;
                self.notice = "Replaced one match.".into();
            }
        }
        drop(editor);
        self.refresh(&self.ui.source.clone())
    }
    fn clipboard(&mut self, command: Command) -> Result<(), Error> {
        let Some(field) = self.field() else {
            return Ok(());
        };
        let Some(node) = field.node() else {
            return Ok(());
        };
        let model = field.editor();
        let editor = model.borrow();
        let Some(session) = editor.session() else {
            return Ok(());
        };
        let range = editor.selection().range();
        let text = match command {
            Paste => None,
            Copy | Cut => {
                let Some(text) = editor
                    .source()
                    .get(range.clone())
                    .filter(|text| !text.is_empty())
                else {
                    return Ok(());
                };
                Some(text.to_owned())
            }
            _ => {
                return Ok(());
            }
        };
        let id = RequestId(self.token()?);
        let request = match text {
            Some(text) => ClipboardRequest::Write { id, text },
            None => ClipboardRequest::Read { id },
        };
        self.clips.insert(
            id,
            ClipTarget {
                node,
                field,
                revision: editor.presentation().revision,
                session,
                range,
                command,
            },
        );
        self.platform.push(PlatformCommand::Clipboard(request));
        Ok(())
    }
    pub(super) fn finish_clip(
        &mut self,
        id: RequestId,
        result: &Result<ClipboardResponse, String>,
    ) -> Result<(), Error> {
        let Some(target) = self.clips.remove(&id) else {
            return Ok(());
        };
        let model = target.field.editor();
        let mut editor = model.borrow_mut();
        if self.tree.focused_node() != Some(target.node)
            || editor.session() != Some(target.session)
            || editor.presentation().revision != target.revision
        {
            return Ok(());
        }
        let value = match (target.command, result) {
            (_, Err(reason)) => {
                self.notice = format!("Clipboard: {reason}");
                return Ok(());
            }
            (Paste, Ok(ClipboardResponse::Read(Some(text)))) => text.as_str(),
            (Cut, Ok(ClipboardResponse::Written)) => "",
            _ => return Ok(()),
        };
        editor
            .replace(target.range, value, self.now)
            .map_err(error)?;
        drop(editor);
        self.refresh(&target.field)
    }
}
