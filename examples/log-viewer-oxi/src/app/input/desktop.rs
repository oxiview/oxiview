//! Native file panel requests and clipboard operations.

use crate::{
    app::{Mode, ViewerApp, error, presentation::column},
    model::Column,
};
use oxiview_core::Error;
use oxiview_platform::{
    ClipboardRequest, ClipboardResponse, FilePanelKind, FilePanelRequest, PlatformCommand,
    RequestId, WindowWaker,
};
use std::{borrow::Cow, collections::HashMap, ops::Range, path::PathBuf};

#[derive(Default)]
pub(in crate::app) struct Desktop {
    pub(in crate::app) commands: Vec<PlatformCommand>,
    next: u64,
    panel: Option<RequestId>,
    clipboard: HashMap<RequestId, Clip>,
}

impl Desktop {
    fn id(&mut self) -> Result<RequestId, Error> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| error("Request IDs exhausted"))?;
        Ok(RequestId(self.next))
    }
}

enum Clip {
    Records,
    SearchCopy,
    SearchEdit {
        session: u64,
        presentation: u64,
        range: Range<usize>,
        paste: bool,
    },
}

impl ViewerApp {
    pub(super) fn request_panel(&mut self) -> Result<(), Error> {
        if self.desktop.panel.is_none() {
            let id = self.desktop.id()?;
            self.desktop.panel = Some(id);
            self.desktop
                .commands
                .push(PlatformCommand::FilePanel(FilePanelRequest {
                    id,
                    kind: FilePanelKind::Open,
                }));
        }
        Ok(())
    }

    pub(in crate::app) fn finish_panel(
        &mut self,
        id: RequestId,
        result: &Result<Option<PathBuf>, String>,
        waker: &WindowWaker,
    ) {
        if self.desktop.panel != Some(id) {
            return;
        }
        self.desktop.panel = None;
        match result {
            Ok(Some(path)) => self.open_path(path.clone(), waker),
            Ok(None) => {}
            Err(error) => self.notice = format!("Open failed: {error}"),
        }
    }

    pub(in crate::app) fn open_path(&mut self, path: PathBuf, waker: &WindowWaker) {
        let label = path.display().to_string();
        self.notice = match self.source.open(path, waker) {
            Ok(()) => format!("Opening {label}…"),
            Err(error) => format!("Could not open {label}: {error}"),
        };
    }

    pub(super) fn copy_records(&mut self) -> Result<(), Error> {
        let Some(row) = self.active_key() else {
            return Ok(());
        };
        let row_header = self.table.selection().is_some_and(|selection| {
            selection.active == selection.anchor && selection.active.column == Column::Number as u64
        });
        let text = if self.mode == Mode::List || row_header {
            self.book
                .index_of(row)
                .and_then(|index| self.book.record_at(index))
                .map(|row| row.raw.clone())
        } else {
            let revision = self.book.revision();
            self.table.copy_tsv(|selected_revision, cell| {
                if selected_revision == revision
                    && let (Some(index), Some(column)) =
                        (self.book.index_of(cell.row), column(cell.column))
                {
                    return self.book.cell(index, column);
                }
                Cow::Borrowed("")
            })
        };
        if let Some(text) = text {
            let id = self.desktop.id()?;
            self.desktop.clipboard.insert(id, Clip::Records);
            self.desktop
                .commands
                .push(PlatformCommand::Clipboard(ClipboardRequest::Write {
                    id,
                    text,
                }));
        }
        Ok(())
    }

    pub(super) fn clip_search(&mut self, paste: bool, cut: bool) -> Result<(), Error> {
        if self.tree.focused_node() != self.ui.search.node() {
            return Ok(());
        }
        let model = self.ui.search.editor();
        let editor = model.borrow();
        let Some(session) = editor.session() else {
            return Ok(());
        };
        if editor.presentation().marked.is_some() {
            return Ok(());
        }
        let range = editor.selection().range();
        let id = self.desktop.id()?;
        let request = if paste {
            ClipboardRequest::Read { id }
        } else if let Some(text) = editor
            .source()
            .get(range.clone())
            .filter(|text| !text.is_empty())
        {
            ClipboardRequest::Write {
                id,
                text: text.into(),
            }
        } else {
            return Ok(());
        };
        self.desktop.clipboard.insert(
            id,
            if paste || cut {
                Clip::SearchEdit {
                    session,
                    presentation: editor.presentation().revision,
                    range,
                    paste,
                }
            } else {
                Clip::SearchCopy
            },
        );
        self.desktop
            .commands
            .push(PlatformCommand::Clipboard(request));
        Ok(())
    }

    pub(in crate::app) fn finish_clipboard(
        &mut self,
        id: RequestId,
        result: &Result<ClipboardResponse, String>,
    ) -> Result<(), Error> {
        let Some(target) = self.desktop.clipboard.remove(&id) else {
            return Ok(());
        };
        let reply = match result {
            Ok(reply) => reply,
            Err(reason) => {
                self.notice = format!("Clipboard: {reason}");
                return Ok(());
            }
        };
        match target {
            Clip::Records if matches!(reply, ClipboardResponse::Written) => {
                self.notice = "Copied to the clipboard".into();
            }
            Clip::SearchEdit {
                session,
                presentation,
                range,
                paste,
            } if let Some(node) = self.ui.search.node()
                && self.tree.focused_node() == Some(node) =>
            {
                let model = self.ui.search.editor();
                let mut editor = model.borrow_mut();
                if editor.session() != Some(session)
                    || editor.presentation().revision != presentation
                {
                    return Ok(());
                }
                let insertion = match (paste, reply) {
                    (true, ClipboardResponse::Read(Some(value))) => value,
                    (false, ClipboardResponse::Written) => "",
                    _ => return Ok(()),
                };
                editor.replace(range, insertion, self.now).map_err(error)?;
                drop(editor);
                self.tree.refresh_editor(node)?;
            }
            _ => {}
        }
        Ok(())
    }
}
