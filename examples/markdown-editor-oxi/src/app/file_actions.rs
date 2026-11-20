//! Preserves the current document while panels and file operations complete.

use super::*;
use crate::document::FileReply;
use std::{collections::BTreeMap, io};

#[derive(Clone, Copy)]
pub(super) enum Intent {
    New,
    Open,
    Close,
}

#[derive(Clone, Copy)]
struct Prompt {
    id: u64,
    intent: Intent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SourceVersion {
    generation: u64,
    revision: u64,
}

enum Panel {
    Open(SourceVersion),
    Save { prompt: Option<u64> },
}

struct PendingPanel {
    id: RequestId,
    kind: Panel,
    focused: Option<NodeId>,
}

struct PendingSave {
    path: PathBuf,
    state: u64,
    prompt: Option<u64>,
}

#[derive(Default)]
pub(super) struct FileActions {
    prompt: Option<Prompt>,
    panel: Option<PendingPanel>,
    opening: Option<(u64, SourceVersion)>,
    saves: BTreeMap<u64, PendingSave>,
    latest_save: u64,
    saved_request: u64,
}

impl FileActions {
    pub(super) fn prompting(&self) -> bool {
        self.prompt.is_some()
    }

    fn prompt_id(&self) -> Option<u64> {
        self.prompt.map(|prompt| prompt.id)
    }
}

impl EditorApp {
    pub(super) fn request_intent(&mut self, intent: Intent) -> Result<(), Error> {
        if self.file_actions.panel.is_some() {
            return Ok(());
        }
        self.file_actions.opening = None;
        if self.dirty() {
            let id = self.token()?;
            self.tree
                .set_style(self.ui.slots.guard.get()?, view::command_bar(true, true))?;
            self.tree.clear_focus()?;
            self.set_prompt_editors_disabled(true)?;
            self.file_actions.prompt = Some(Prompt { id, intent });
            self.notice = "This document has unsaved changes.".into();
            Ok(())
        } else {
            self.resume_intent(intent)
        }
    }

    fn set_prompt_editors_disabled(&mut self, disabled: bool) -> Result<(), Error> {
        if let Some(source) = self.ui.source.node() {
            self.tree.set_disabled(source, disabled)?;
        }
        self.tree
            .set_disabled(self.ui.slots.find.get()?, disabled)?;
        Ok(())
    }

    pub(super) fn clear_prompt(&mut self) -> Result<Option<Intent>, Error> {
        let Some(prompt) = self.file_actions.prompt.take() else {
            return Ok(None);
        };
        self.set_prompt_editors_disabled(false)?;
        self.tree
            .set_style(self.ui.slots.guard.get()?, view::command_bar(false, true))?;
        Ok(Some(prompt.intent))
    }

    pub(super) fn discard_changes(&mut self) -> Result<(), Error> {
        if self.file_actions.panel.is_none()
            && let Some(intent) = self.clear_prompt()?
        {
            self.focus_source()?;
            self.resume_intent(intent)?;
        }
        Ok(())
    }

    fn resume_intent(&mut self, intent: Intent) -> Result<(), Error> {
        match intent {
            Intent::New => self.install_document(Document::default(), String::new()),
            Intent::Open => {
                self.show_panel(Panel::Open(self.source_version()))?;
                self.notice = "Choose a Markdown file to open.".into();
                Ok(())
            }
            Intent::Close => {
                self.platform.push(PlatformCommand::Close);
                Ok(())
            }
        }
    }

    fn show_panel(&mut self, kind: Panel) -> Result<(), Error> {
        let id = RequestId(self.token()?);
        let request = match kind {
            Panel::Open(_) => FilePanelKind::Open,
            Panel::Save { .. } => FilePanelKind::Save {
                suggested_name: Some(self.document.title()),
            },
        };
        self.file_actions.panel = Some(PendingPanel {
            id,
            kind,
            focused: self.tree.focused_node(),
        });
        self.platform
            .push(PlatformCommand::FilePanel(FilePanelRequest {
                id,
                kind: request,
            }));
        Ok(())
    }

    pub(super) fn save(&mut self, save_as: bool, waker: &WindowWaker) -> Result<(), Error> {
        if self.file_actions.panel.is_some() {
            return Ok(());
        }
        if !save_as {
            // A regular Save can follow a Save As that has chosen its
            // destination but has yet to finish writing it.
            let pending = self
                .file_actions
                .saves
                .last_key_value()
                .filter(|&(&token, _)| token > self.file_actions.saved_request)
                .map(|(_, save)| &save.path);
            if let Some(path) = pending.or(self.document.path.as_ref()).cloned() {
                return self.write_file(path, waker);
            }
        }
        self.show_panel(Panel::Save {
            prompt: self.file_actions.prompt_id(),
        })?;
        self.notice = "Choose a destination for this document.".into();
        Ok(())
    }

    fn write_file(&mut self, path: PathBuf, waker: &WindowWaker) -> Result<(), Error> {
        let token = self.token()?;
        let model = self.source();
        let mut editor = model.borrow_mut();
        if editor.presentation().marked.is_none() {
            // Saving in the middle of typing must leave an undo point at
            // the saved state. An active mark has already ended that group.
            let selection = editor.selection();
            editor.set_selection(selection).map_err(error)?;
        }
        let state = editor.document_state();
        let source = editor.source().to_owned();
        drop(editor);
        self.file_actions.latest_save = token;
        if let Err(reason) =
            self.files
                .save(token, path.clone(), source, self.document.format, waker)
        {
            self.save_failed(&reason);
            return Ok(());
        }
        self.file_actions.saves.insert(
            token,
            PendingSave {
                path,
                state,
                prompt: self.file_actions.prompt_id(),
            },
        );
        self.notice = "Saving…".into();
        Ok(())
    }

    fn source_version(&self) -> SourceVersion {
        SourceVersion {
            generation: self.generation,
            revision: self.source().borrow().committed_revision(),
        }
    }

    fn can_finish_open(&mut self, version: SourceVersion) -> bool {
        let current = self.source_version() == version
            && self.source().borrow().presentation().marked.is_none();
        if !current {
            self.notice = "Open cancelled because the current document changed.".into();
        }
        current
    }

    pub(super) fn open_startup(&mut self, path: PathBuf, waker: &WindowWaker) -> Result<(), Error> {
        self.start_open(path, self.source_version(), waker)
    }

    fn start_open(
        &mut self,
        path: PathBuf,
        version: SourceVersion,
        waker: &WindowWaker,
    ) -> Result<(), Error> {
        let token = self.token()?;
        match self.files.open(token, path, waker) {
            Ok(()) => {
                self.file_actions.opening = Some((token, version));
                self.notice = "Opening…".into();
            }
            Err(reason) => self.notice = format!("Could not open: {reason}"),
        }
        Ok(())
    }

    pub(super) fn finish_panel(
        &mut self,
        id: RequestId,
        result: &Result<Option<PathBuf>, String>,
        waker: &WindowWaker,
    ) -> Result<(), Error> {
        if self
            .file_actions
            .panel
            .as_ref()
            .is_none_or(|panel| panel.id != id)
        {
            return Ok(());
        }
        let Some(panel) = self.file_actions.panel.take() else {
            return Ok(());
        };
        if let Panel::Save { prompt } = panel.kind
            && prompt != self.file_actions.prompt_id()
        {
            return Ok(());
        }
        if !self.file_actions.prompting()
            && self.tree.focused_node() == panel.focused
            && self.field().is_none()
        {
            self.focus_source()?;
        }
        match result {
            Ok(Some(path)) => match panel.kind {
                Panel::Open(version) if self.can_finish_open(version) => {
                    self.start_open(path.clone(), version, waker)?;
                }
                Panel::Open(_) => {}
                Panel::Save { .. } => self.write_file(path.clone(), waker)?,
            },
            Ok(None) => {
                self.notice = "Cancelled.".into();
                if self.clear_prompt()?.is_some() {
                    self.focus_source()?;
                }
            }
            Err(reason) => {
                self.notice = format!("File panel: {reason}");
            }
        }
        Ok(())
    }

    fn install_document(&mut self, doc: Document, source: String) -> Result<(), Error> {
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| error("Document identities exhausted"))?;
        self.clear_prompt()?;
        self.tree.clear_focus()?;
        let model = self.source();
        {
            let mut editor = model.borrow_mut();
            editor.reset(source).map_err(error)?;
            self.saved_state = editor.document_state();
        }
        for field in [&self.ui.query, &self.ui.replacement] {
            field.editor().borrow_mut().reset("").map_err(error)?;
        }
        self.matches = Matches::default();
        self.find_open = false;
        self.tree
            .set_style(self.ui.slots.find.get()?, view::command_bar(false, false))?;
        self.refresh(&self.ui.source.clone())?;
        self.base_dir = doc.path.as_deref().map_or_else(
            || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            document_directory,
        );
        self.document = doc;
        self.generation = generation;
        self.clips.clear();
        self.file_actions = FileActions::default();
        self.preview.forget();
        self.presentation.document_changed();
        self.focus_source()?;
        self.notice = "Ready".into();
        Ok(())
    }

    pub(super) fn poll_files(&mut self) -> Result<bool, Error> {
        let mut changed = false;
        while let Some(reply) = self.files.poll() {
            match reply {
                FileReply::Opened { token, result } => {
                    let Some((expected, version)) = self.file_actions.opening else {
                        continue;
                    };
                    if expected != token {
                        continue;
                    }
                    self.file_actions.opening = None;
                    if self.can_finish_open(version) {
                        match result {
                            Ok((document, source)) => {
                                self.install_document(document, source)?;
                                changed = true;
                            }
                            Err(e) => self.notice = format!("Could not open: {e}"),
                        }
                    }
                }
                FileReply::Saved { token, result } => {
                    changed |= self.finish_save(token, result)?;
                }
            }
        }
        Ok(changed)
    }

    fn finish_save(&mut self, token: u64, result: io::Result<()>) -> Result<bool, Error> {
        let Some(save) = self.file_actions.saves.remove(&token) else {
            return Ok(false);
        };
        let latest = token == self.file_actions.latest_save;
        if let Err(reason) = result {
            if latest {
                self.save_failed(&reason);
            }
            return Ok(false);
        }
        if token < self.file_actions.saved_request {
            return Ok(false);
        }
        self.file_actions.saved_request = token;
        self.saved_state = save.state;
        if self.document.path.as_ref() != Some(&save.path) {
            self.base_dir = document_directory(&save.path);
            self.document.path = Some(save.path);
            self.presentation.document_changed();
        }
        let dirty = self.dirty();
        if latest {
            self.notice = if dirty {
                "Saved the previous revision. New edits are unsaved.".into()
            } else {
                "Saved.".into()
            };
        }
        if latest
            && !dirty
            && save.prompt == self.file_actions.prompt_id()
            && self.file_actions.panel.is_none()
            && let Some(intent) = self.clear_prompt()?
        {
            self.focus_source()?;
            self.resume_intent(intent)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn save_failed(&mut self, reason: &io::Error) {
        self.notice = format!("Could not save: {reason}. Choose Save or Save As to retry.");
    }
}

fn document_directory(path: &Path) -> PathBuf {
    path.parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_owned()
}
