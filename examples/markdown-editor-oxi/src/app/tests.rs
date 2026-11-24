use super::*;
use crate::document::{FileReply, FileRequest};
use oxiview_core::FrameKind;
use oxiview_platform::{
    KeyCode, KeyModifiers, PointerButton, ShortcutPlatform, TextCommitKind, TextInputSnapshot,
    WindowWake,
};
use std::{
    io,
    ops::Range,
    sync::{Arc, mpsc},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Wake(mpsc::Sender<()>);

impl WindowWake for Wake {
    fn wake(&self) -> std::result::Result<(), oxiview_platform::Error> {
        let _ = self.0.send(());
        Ok(())
    }
}

struct Session {
    app: EditorApp,
    scheduler: Scheduler,
    waker: WindowWaker,
    wakes: mpsc::Receiver<()>,
    jobs: mpsc::Receiver<(u64, FileRequest, WindowWaker)>,
    replies: mpsc::Sender<FileReply>,
    input: Option<TextInputSnapshot>,
    time: Duration,
}

impl Session {
    fn new(source: &str) -> Result<Self> {
        let mut fonts = TextSystem::without_system_fonts();
        fonts.register_font(
            include_bytes!("../../../../crates/oxiview-text/tests/fonts/NotoSans.ttf").to_vec(),
        )?;
        let mut app = EditorApp::with_text(None, source, fonts)?;
        let (files, jobs, replies) = Files::controlled();
        app.files = files;
        let (wake, wakes) = mpsc::channel();
        let mut session = Self {
            app,
            scheduler: Scheduler::default(),
            waker: WindowWaker::new(Arc::new(Wake(wake))),
            wakes,
            jobs,
            replies,
            input: None,
            time: Duration::from_secs(10),
        };
        session.dispatch(WindowEvent::ActivationChanged {
            active: true,
            timestamp: session.time,
        })?;
        Ok(session)
    }

    fn tick(&mut self) -> Duration {
        self.time += Duration::from_millis(10);
        self.time
    }

    fn dispatch(&mut self, event: WindowEvent) -> Result<Vec<PlatformCommand>> {
        self.app.event(&event, &mut self.scheduler, &self.waker)?;
        self.flush()
    }

    fn flush(&mut self) -> Result<Vec<PlatformCommand>> {
        let mut commands = Vec::new();
        self.app.flush_platform(&mut commands)?;
        for command in &commands {
            if let PlatformCommand::TextInput(input) = command {
                self.input.clone_from(input);
            }
        }
        Ok(commands)
    }

    fn key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        pressed: bool,
        repeat: bool,
    ) -> Result<Vec<PlatformCommand>> {
        let timestamp = self.tick();
        self.dispatch(WindowEvent::Key {
            code,
            logical: code.us_logical_key(),
            modifiers,
            pressed,
            repeat,
            timestamp,
        })
    }

    fn stroke(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<Vec<PlatformCommand>> {
        let mut commands = self.key(code, modifiers, true, false)?;
        commands.extend(self.key(code, modifiers, false, false)?);
        Ok(commands)
    }

    fn shortcut(&mut self, code: KeyCode, shift: bool) -> Result<Vec<PlatformCommand>> {
        self.stroke(code, primary(shift))
    }

    fn snapshot(&self) -> Result<&TextInputSnapshot> {
        self.input
            .as_ref()
            .ok_or_else(|| "Expected a focused editor".into())
    }

    fn native(&mut self, edit: TextInputEdit) -> Result<Vec<PlatformCommand>> {
        let input = self.snapshot()?;
        let mut event = TextInputEvent {
            session: input.session,
            revision: input.revision,
            sequence: input.acknowledged + 1,
            timestamp: self.time,
            edit,
        };
        event.timestamp = self.tick();
        self.dispatch(WindowEvent::TextInput(event))
    }

    fn type_text(&mut self, text: &str) -> Result {
        let range = self.snapshot()?.selection.clone();
        self.native(TextInputEdit::Commit {
            range,
            text: text.into(),
            kind: TextCommitKind::Typing,
        })?;
        Ok(())
    }

    fn source(&self) -> String {
        self.app.source().borrow().source().into()
    }

    fn selection(&self) -> Range<usize> {
        self.app.source().borrow().selection().range()
    }

    fn focused_source(&self) -> bool {
        self.app.tree.focused_node() == self.app.ui.source.node()
    }

    fn click(&mut self, node: NodeId) -> Result<Vec<PlatformCommand>> {
        let bounds = self.app.tree.bounds(node)?;
        let [left, top, right, bottom] = bounds.edges();
        assert!(left < right && top < bottom);
        let position = Point::new((left + right) * 0.5, (top + bottom) * 0.5);
        let mut commands = Vec::new();
        for pressed in [true, false] {
            let timestamp = self.tick();
            commands.extend(self.dispatch(WindowEvent::PointerButton {
                button: PointerButton::Primary,
                pressed,
                position,
                timestamp,
            })?);
        }
        Ok(commands)
    }

    fn click_button(&mut self, bar: NodeId, command: Command) -> Result<Vec<PlatformCommand>> {
        for &child in self.app.tree.children(bar)? {
            if self
                .app
                .tree
                .actions(child)?
                .iter()
                .any(|action| action.id == command.id())
            {
                return self.click(child);
            }
        }
        Err(format!("Missing {} button", command.label()).into())
    }

    fn click_prompt(&mut self, command: Command) -> Result<Vec<PlatformCommand>> {
        self.click_button(self.app.ui.slots.guard.get()?, command)
    }

    fn panel_reply(
        &mut self,
        id: RequestId,
        result: std::result::Result<Option<PathBuf>, String>,
    ) -> Result<Vec<PlatformCommand>> {
        self.dispatch(WindowEvent::FilePanel { id, result })
    }

    fn file_reply(&mut self, reply: FileReply) -> Result<Vec<PlatformCommand>> {
        self.replies.send(reply)?;
        self.dispatch(WindowEvent::Wake)
    }

    fn pending_save(&self, path: &Path, text: &str) -> Result<u64> {
        let (token, request, _) = self.jobs.try_recv()?;
        let FileRequest::Save(destination, source, _) = request else {
            return Err("Expected a write request".into());
        };
        assert_eq!(destination, path);
        assert_eq!(source, text);
        Ok(token)
    }

    fn pending_open(&self, path: &Path) -> Result<u64> {
        let (token, request, _) = self.jobs.try_recv()?;
        let FileRequest::Open(destination) = request else {
            return Err("Expected a read request".into());
        };
        assert_eq!(destination, path);
        Ok(token)
    }

    fn save_as(&mut self, path: &Path, text: &str) -> Result<u64> {
        let request = panel(self.shortcut(KeyCode::KeyS, true)?)?;
        assert!(matches!(request.kind, FilePanelKind::Save { .. }));
        self.panel_reply(request.id, Ok(Some(path.into())))?;
        self.pending_save(path, text)
    }

    fn paint(&mut self) -> Result<Vec<PlatformCommand>> {
        self.scheduler.schedule();
        self.scheduler.frame();
        let timing = FrameTiming {
            kind: FrameKind::Vsync,
            predicted_presentation_timestamp: self.time,
            refresh_interval: Duration::from_millis(16),
        };
        self.app.paint(timing, self.app.physical_size, 1.0)?;
        self.scheduler.produced(timing, self.time);
        self.flush()
    }

    fn wait_preview(&mut self) -> Result {
        while self.app.preview.published != self.app.preview.submitted {
            self.wakes.recv_timeout(Duration::from_secs(10))?;
            self.dispatch(WindowEvent::Wake)?;
        }
        Ok(())
    }
}

fn primary(shift: bool) -> KeyModifiers {
    KeyModifiers {
        shift,
        ..ShortcutPlatform::current().primary_modifier()
    }
}

fn panel(commands: Vec<PlatformCommand>) -> Result<FilePanelRequest> {
    let mut requests = commands.into_iter().filter_map(|command| match command {
        PlatformCommand::FilePanel(request) => Some(request),
        _ => None,
    });
    let request = requests.next().ok_or("Expected a file panel request")?;
    assert!(requests.next().is_none());
    Ok(request)
}

fn clipboard(commands: Vec<PlatformCommand>) -> Result<ClipboardRequest> {
    let mut requests = commands.into_iter().filter_map(|command| match command {
        PlatformCommand::Clipboard(request) => Some(request),
        _ => None,
    });
    let request = requests.next().ok_or("Expected a clipboard request")?;
    assert!(requests.next().is_none());
    Ok(request)
}

#[test]
fn dirty_intents_preserve_text_selection_and_history_until_resolved() -> Result {
    let mut window = Session::new("original")?;
    window.type_text("new ")?;
    let first = window.snapshot()?.clone();
    let commands = window.shortcut(KeyCode::KeyN, false)?;
    assert_eq!(commands, [PlatformCommand::TextInput(None)]);
    assert!(window.app.file_actions.prompting());
    assert_eq!(window.source(), "new original");

    window.click(window.app.ui.source.node().ok_or("Missing source")?)?;
    assert!(window.input.is_none());
    window.dispatch(WindowEvent::TextInput(TextInputEvent {
        session: first.session,
        revision: first.revision,
        sequence: first.acknowledged + 1,
        timestamp: window.time,
        edit: TextInputEdit::Commit {
            range: first.selection.clone(),
            text: "stale".into(),
            kind: TextCommitKind::Typing,
        },
    }))?;
    window.stroke(KeyCode::Escape, KeyModifiers::default())?;
    assert!(window.focused_source());
    assert_eq!(window.snapshot()?.selection, first.selection);
    assert_eq!(window.source(), "new original");
    window.shortcut(KeyCode::KeyZ, false)?;
    assert_eq!(window.source(), "original");
    assert!(!window.app.dirty());
    window.shortcut(KeyCode::KeyZ, true)?;

    assert_eq!(
        window.shortcut(KeyCode::KeyO, false)?,
        [PlatformCommand::TextInput(None)]
    );
    let request = panel(window.click_prompt(Command::Discard)?)?;
    assert_eq!(request.kind, FilePanelKind::Open);
    window.panel_reply(request.id, Ok(None))?;
    assert!(window.focused_source());
    assert_eq!(window.source(), "new original");
    assert!(window.app.dirty());

    assert_eq!(
        window.dispatch(WindowEvent::CloseRequested)?,
        [PlatformCommand::TextInput(None)]
    );
    assert!(window.app.file_actions.prompting());
    assert!(
        !window
            .click_prompt(Command::KeepWriting)?
            .contains(&PlatformCommand::Close)
    );
    assert!(window.focused_source());
    assert_eq!(window.source(), "new original");
    window.dispatch(WindowEvent::CloseRequested)?;
    assert!(
        window
            .click_prompt(Command::Discard)?
            .contains(&PlatformCommand::Close)
    );

    let mut clean = Session::new("clean")?;
    assert_eq!(
        clean.dispatch(WindowEvent::CloseRequested)?,
        [PlatformCommand::Close]
    );
    assert!(!clean.app.file_actions.prompting());
    Ok(())
}

#[test]
fn cancelled_and_failed_save_as_leave_the_draft_available_for_retry() -> Result {
    let mut window = Session::new("")?;
    window.type_text("标题 😀")?;
    window.shortcut(KeyCode::KeyN, false)?;

    let first = panel(window.click_prompt(Command::Save)?)?;
    assert!(window.click_prompt(Command::Discard)?.is_empty());
    assert!(window.app.file_actions.prompting());
    window.panel_reply(RequestId(first.id.0 + 1000), Ok(Some("wrong.md".into())))?;
    assert!(window.jobs.try_recv().is_err());
    window.panel_reply(first.id, Ok(None))?;
    assert_eq!(window.source(), "标题 😀");
    assert!(window.app.document.path.is_none());
    assert!(window.app.dirty());
    assert!(window.focused_source());
    assert!(!window.app.file_actions.prompting());

    window.shortcut(KeyCode::KeyN, false)?;
    let request = panel(window.click_prompt(Command::Save)?)?;
    window.panel_reply(request.id, Err("panel failed".into()))?;
    assert!(window.app.notice.contains("panel failed"));
    assert!(window.app.file_actions.prompting());

    let failed = panel(window.click_prompt(Command::Save)?)?;
    window.panel_reply(failed.id, Ok(Some("failed.md".into())))?;
    let failed = window.pending_save(Path::new("failed.md"), "标题 😀")?;
    window.file_reply(FileReply::Saved {
        token: failed,
        result: Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "read-only destination",
        )),
    })?;
    assert!(window.app.file_actions.prompting());
    assert!(window.app.notice.contains("read-only destination"));
    assert_eq!(window.source(), "标题 😀");
    assert!(window.app.document.path.is_none());

    let retry = panel(window.click_prompt(Command::Save)?)?;
    window.panel_reply(retry.id, Ok(Some("correct.md".into())))?;
    let retry = window.pending_save(Path::new("correct.md"), "标题 😀")?;
    window.file_reply(FileReply::Saved {
        token: failed,
        result: Ok(()),
    })?;
    assert!(window.app.file_actions.prompting());
    assert_eq!(window.source(), "标题 😀");
    window.file_reply(FileReply::Saved {
        token: retry,
        result: Ok(()),
    })?;
    assert!(window.focused_source());
    assert!(!window.app.file_actions.prompting());
    assert!(window.app.document.path.is_none());
    assert_eq!(window.source(), "");
    assert!(!window.app.source().borrow().can_undo());
    assert!(!window.app.dirty());
    Ok(())
}

#[test]
fn delayed_open_preserves_edits_made_in_the_panel_and_during_the_read() -> Result {
    let mut window = Session::new("original")?;
    let target = PathBuf::from("selected.md");
    let request = panel(window.shortcut(KeyCode::KeyO, false)?)?;
    window.type_text("written during panel ")?;
    window.panel_reply(request.id, Ok(Some(target.clone())))?;
    assert!(window.jobs.try_recv().is_err());
    assert_eq!(window.source(), "written during panel original");
    assert!(window.app.notice.contains("document changed"));
    assert!(window.focused_source());

    window.shortcut(KeyCode::KeyZ, false)?;
    let request = panel(window.shortcut(KeyCode::KeyO, false)?)?;
    window.panel_reply(request.id, Ok(Some(target.clone())))?;
    let token = window.pending_open(&target)?;
    window.type_text("written during read ")?;
    window.file_reply(FileReply::Opened {
        token,
        result: Ok((
            Document {
                path: Some(target.clone()),
                ..Document::default()
            },
            "different".into(),
        )),
    })?;
    assert_eq!(window.source(), "written during read original");
    assert!(window.app.notice.contains("document changed"));
    assert!(window.app.document.path.is_none());
    window.shortcut(KeyCode::KeyZ, false)?;

    let request = panel(window.shortcut(KeyCode::KeyO, false)?)?;
    window.panel_reply(request.id, Ok(Some(target.clone())))?;
    let token = window.pending_open(&target)?;
    window.file_reply(FileReply::Opened {
        token,
        result: Err(io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8")),
    })?;
    assert_eq!(window.source(), "original");
    assert!(window.app.notice.contains("invalid UTF-8"));
    assert!(window.app.document.path.is_none());

    let request = panel(window.shortcut(KeyCode::KeyO, false)?)?;
    window.panel_reply(request.id, Ok(Some(target.clone())))?;
    let token = window.pending_open(&target)?;
    window.file_reply(FileReply::Opened {
        token,
        result: Ok((
            Document {
                path: Some(target.clone()),
                ..Document::default()
            },
            "different".into(),
        )),
    })?;
    assert_eq!(window.source(), "different");
    assert_eq!(window.app.document.path.as_deref(), Some(target.as_path()));
    assert!(window.focused_source());
    assert!(!window.app.dirty());
    assert!(!window.app.source().borrow().can_undo());
    Ok(())
}

#[test]
fn completing_a_save_for_an_older_edit_sets_an_undo_savepoint() -> Result {
    let mut window = Session::new("")?;
    let path = Path::new("notes.md");
    window.type_text("one")?;
    let save = window.save_as(path, "one")?;
    window.type_text(" two")?;
    window.file_reply(FileReply::Saved {
        token: save,
        result: Ok(()),
    })?;
    assert_eq!(window.source(), "one two");
    assert_eq!(window.app.document.path.as_deref(), Some(path));
    assert!(window.app.dirty());
    assert!(window.app.notice.contains("previous revision"));
    assert_eq!(
        window
            .app
            .tree
            .text_source(window.app.ui.slots.title.get()?)?,
        "notes.md  •"
    );

    window.shortcut(KeyCode::KeyZ, false)?;
    assert_eq!(window.source(), "one");
    assert!(!window.app.dirty());
    assert_eq!(
        window
            .app
            .tree
            .text_source(window.app.ui.slots.title.get()?)?,
        "notes.md"
    );
    window.shortcut(KeyCode::KeyZ, true)?;
    assert_eq!(window.source(), "one two");
    assert!(window.app.dirty());
    assert!(window.shortcut(KeyCode::KeyS, false)?.is_empty());
    let latest = window.pending_save(path, "one two")?;
    window.file_reply(FileReply::Saved {
        token: latest,
        result: Ok(()),
    })?;
    assert!(!window.app.dirty());
    Ok(())
}

#[test]
fn ordered_save_paths_survive_reverse_replies_and_a_failed_newer_write() -> Result {
    let mut window = Session::new("")?;
    window.type_text("first")?;
    let older = window.save_as(Path::new("first.md"), "first")?;
    window.type_text(" second")?;
    let second = Path::new("second.md");
    let newer = window.save_as(second, "first second")?;
    window.type_text(" third")?;
    window.shortcut(KeyCode::KeyS, false)?;
    let latest = window.pending_save(second, "first second third")?;
    for token in [latest, older, newer] {
        window.file_reply(FileReply::Saved {
            token,
            result: Ok(()),
        })?;
        assert_eq!(window.app.document.path.as_deref(), Some(second));
        assert!(!window.app.dirty());
        assert_eq!(window.app.notice, "Saved.");
    }

    window.type_text(" revised")?;
    let recovery = Path::new("recovery.md");
    let fallback = window.save_as(recovery, "first second third revised")?;
    let failed = window.save_as(Path::new("failed.md"), "first second third revised")?;
    window.file_reply(FileReply::Saved {
        token: failed,
        result: Err(io::Error::other("write failed")),
    })?;
    assert_eq!(window.app.document.path.as_deref(), Some(second));
    window.file_reply(FileReply::Saved {
        token: fallback,
        result: Ok(()),
    })?;
    assert_eq!(window.app.document.path.as_deref(), Some(recovery));
    assert!(!window.app.dirty());
    assert!(window.app.notice.contains("write failed"));
    Ok(())
}

#[test]
fn a_save_from_an_earlier_prompt_cannot_complete_a_later_close() -> Result {
    let mut window = Session::new("")?;
    let path = Path::new("notes.md");
    window.type_text("first")?;
    let save = window.save_as(path, "first")?;
    window.file_reply(FileReply::Saved {
        token: save,
        result: Ok(()),
    })?;
    window.type_text(" second")?;
    window.shortcut(KeyCode::KeyN, false)?;
    window.click_prompt(Command::Save)?;
    let old_prompt = window.pending_save(path, "first second")?;
    window.click_prompt(Command::KeepWriting)?;
    window.dispatch(WindowEvent::CloseRequested)?;
    assert!(window.app.file_actions.prompting());
    let commands = window.file_reply(FileReply::Saved {
        token: old_prompt,
        result: Ok(()),
    })?;
    assert!(!commands.contains(&PlatformCommand::Close));
    assert!(window.app.file_actions.prompting());
    assert_eq!(window.source(), "first second");
    window.click_prompt(Command::Save)?;
    let close = window.pending_save(path, "first second")?;
    let commands = window.file_reply(FileReply::Saved {
        token: close,
        result: Ok(()),
    })?;
    assert!(commands.contains(&PlatformCommand::Close));
    Ok(())
}

#[test]
fn clipboard_replies_apply_only_to_the_requesting_field_and_revision() -> Result {
    let mut window = Session::new("plain 👩‍💻")?;
    window.shortcut(KeyCode::KeyA, false)?;
    let range = window.snapshot()?.selection.clone();
    let cut = clipboard(window.key(KeyCode::KeyX, primary(false), true, false)?)?;
    assert!(matches!(&cut, ClipboardRequest::Write { text, .. } if text == "plain 👩‍💻"));
    assert!(
        window
            .key(KeyCode::KeyX, primary(false), true, true)?
            .is_empty()
    );
    window.key(KeyCode::KeyX, primary(false), false, false)?;
    window.dispatch(WindowEvent::Clipboard {
        id: cut.id(),
        result: Err("clipboard denied".into()),
    })?;
    assert_eq!(window.source(), "plain 👩‍💻");
    assert_eq!(window.selection(), range);
    assert!(window.app.notice.contains("clipboard denied"));
    assert!(!window.app.source().borrow().can_undo());

    let cut = clipboard(window.shortcut(KeyCode::KeyX, false)?)?;
    window.dispatch(WindowEvent::Clipboard {
        id: cut.id(),
        result: Ok(ClipboardResponse::Written),
    })?;
    assert_eq!(window.source(), "");
    window.shortcut(KeyCode::KeyZ, false)?;
    assert_eq!(window.source(), "plain 👩‍💻");

    let paste = clipboard(window.shortcut(KeyCode::KeyV, false)?)?;
    window.shortcut(KeyCode::KeyF, false)?;
    assert_eq!(window.app.tree.focused_node(), window.app.ui.query.node());
    window.dispatch(WindowEvent::Clipboard {
        id: paste.id(),
        result: Ok(ClipboardResponse::Read(Some("stale source".into()))),
    })?;
    assert_eq!(window.snapshot()?.text, "");
    assert_eq!(window.source(), "plain 👩‍💻");

    let paste = clipboard(window.shortcut(KeyCode::KeyV, false)?)?;
    window.type_text("typed")?;
    window.dispatch(WindowEvent::Clipboard {
        id: paste.id(),
        result: Ok(ClipboardResponse::Read(Some("stale query".into()))),
    })?;
    assert_eq!(window.snapshot()?.text, "typed");
    let paste = clipboard(window.shortcut(KeyCode::KeyV, false)?)?;
    window.dispatch(WindowEvent::Clipboard {
        id: paste.id(),
        result: Ok(ClipboardResponse::Read(Some("one\r\n二".into()))),
    })?;
    assert_eq!(window.snapshot()?.text, "typedone 二");

    window.stroke(KeyCode::Escape, KeyModifiers::default())?;
    assert!(window.focused_source());
    assert_eq!(window.selection(), range);
    let copy = clipboard(window.shortcut(KeyCode::KeyC, false)?)?;
    window.dispatch(WindowEvent::Clipboard {
        id: copy.id(),
        result: Ok(ClipboardResponse::Read(Some("wrong reply type".into()))),
    })?;
    assert_eq!(window.source(), "plain 👩‍💻");
    assert_eq!(window.selection(), range);

    window.app.tree.clear_focus()?;
    window.dispatch(WindowEvent::Wake)?;
    assert!(window.shortcut(KeyCode::KeyV, false)?.is_empty());
    assert!(window.app.clips.is_empty());
    Ok(())
}

#[test]
fn find_actions_keep_field_scope_and_replace_all_is_one_undo_step() -> Result {
    let initial = "é x é x é";
    let mut window = Session::new(initial)?;
    window.shortcut(KeyCode::KeyF, false)?;
    assert_eq!(window.app.tree.focused_node(), window.app.ui.query.node());
    window.type_text("é")?;
    window.shortcut(KeyCode::KeyB, false)?;
    assert_eq!(window.source(), initial);
    assert_eq!(window.snapshot()?.text, "é");

    window.key(KeyCode::Enter, KeyModifiers::default(), true, false)?;
    assert_eq!(window.selection(), 0..2);
    window.key(KeyCode::Enter, KeyModifiers::default(), true, true)?;
    assert_eq!(window.selection(), 5..7);
    window.key(KeyCode::Enter, KeyModifiers::default(), false, false)?;
    window.key(KeyCode::KeyG, primary(true), true, false)?;
    assert_eq!(window.selection(), 0..2);
    window.key(KeyCode::KeyG, primary(true), true, true)?;
    assert_eq!(window.selection(), 10..12);
    window.key(KeyCode::KeyG, primary(true), false, false)?;
    window.native(TextInputEdit::Command(TextEditingCommand::InsertNewline))?;
    assert_eq!(window.selection(), 0..2);
    assert_eq!(window.snapshot()?.text, "é");
    assert_eq!(window.source(), initial);

    window.native(TextInputEdit::SetMarked {
        range: 0..2,
        text: "pending".into(),
        selection: 7..7,
    })?;
    window.stroke(KeyCode::Enter, KeyModifiers::default())?;
    assert_eq!(window.selection(), 0..2);
    assert!(window.snapshot()?.marked.is_some());
    window.stroke(KeyCode::Escape, KeyModifiers::default())?;
    assert!(window.app.find_open);
    assert!(window.snapshot()?.marked.is_none());
    assert_eq!(window.snapshot()?.text, "é");

    window.click(
        window
            .app
            .ui
            .replacement
            .node()
            .ok_or("Missing replacement field")?,
    )?;
    window.type_text("z")?;
    window.shortcut(KeyCode::KeyZ, false)?;
    assert_eq!(window.snapshot()?.text, "");
    assert_eq!(window.source(), initial);
    window.type_text("z")?;
    let find = window.app.ui.slots.find.get()?;
    window.click_button(find, Command::Replace)?;
    assert_eq!(window.source(), "z x é x é");
    assert!(window.input.is_none());
    window.key(KeyCode::KeyG, primary(false), true, false)?;
    assert_eq!(window.selection(), 4..6);
    window.key(KeyCode::KeyG, primary(false), true, true)?;
    assert_eq!(window.selection(), 9..11);
    window.key(KeyCode::KeyG, primary(false), false, false)?;
    window.click_button(find, Command::ReplaceAll)?;
    assert_eq!(window.source(), "z x z x z");
    assert_eq!(window.app.notice, "Replaced 2 matches.");
    window.stroke(KeyCode::Escape, KeyModifiers::default())?;
    assert!(window.focused_source());
    assert!(!window.app.find_open);
    window.shortcut(KeyCode::KeyZ, false)?;
    assert_eq!(window.source(), "z x é x é");
    assert_eq!(window.snapshot()?.selection, 9..11);
    window.shortcut(KeyCode::KeyZ, false)?;
    assert_eq!(window.source(), initial);

    window.stroke(KeyCode::Enter, KeyModifiers::default())?;
    assert_eq!(window.source(), "\n x é x é");
    window.native(TextInputEdit::Command(TextEditingCommand::InsertNewline))?;
    assert_eq!(window.source(), "\n\n x é x é");
    Ok(())
}

#[test]
fn find_reveals_an_offscreen_source_match_while_the_query_retains_focus() -> Result {
    let mut initial = "one more line\n".repeat(120);
    let start = initial.len();
    initial.push_str("target");
    let mut window = Session::new(&initial)?;
    window.shortcut(KeyCode::KeyF, false)?;
    window.type_text("target")?;
    window.stroke(KeyCode::Enter, KeyModifiers::default())?;
    assert_eq!(window.selection(), start..initial.len());
    assert_eq!(window.app.tree.focused_node(), window.app.ui.query.node());
    window.paint()?;

    let source = window.app.ui.source.node().ok_or("Missing source")?;
    assert!(
        !window
            .app
            .tree
            .scroll_editor(source, Point::new(0., 10000.))?
    );
    assert_eq!(window.app.tree.focused_node(), window.app.ui.query.node());
    window.click_button(window.app.ui.slots.find.get()?, Command::Dismiss)?;
    assert!(window.focused_source());
    assert_eq!(window.snapshot()?.selection, start..initial.len());
    Ok(())
}

#[test]
fn formatting_shortcuts_and_toolbar_wrap_source_as_one_edit() -> Result {
    let mut window = Session::new("a note")?;
    window.shortcut(KeyCode::KeyA, false)?;
    window.key(KeyCode::KeyB, primary(false), true, false)?;
    window.key(KeyCode::KeyB, primary(false), true, true)?;
    window.key(KeyCode::KeyB, primary(false), false, false)?;
    assert_eq!(window.source(), "**a note**");
    assert_eq!(window.selection(), 2..8);
    window.shortcut(KeyCode::KeyZ, false)?;
    assert_eq!(window.source(), "a note");
    assert_eq!(window.selection(), 0..6);

    window.shortcut(KeyCode::KeyF, false)?;
    let title = window.app.ui.slots.title.get()?;
    let tree = &window.app.tree;
    let toolbar = tree
        .parent(tree.parent(title)?.ok_or("Missing title container")?)?
        .ok_or("Missing toolbar")?;
    window.click_button(toolbar, Command::Italic)?;
    assert!(window.focused_source());
    assert_eq!(window.source(), "*a note*");
    assert_eq!(window.selection(), 1..7);
    window.shortcut(KeyCode::KeyZ, false)?;
    assert_eq!(window.source(), "a note");
    Ok(())
}

#[test]
fn input_publication_tracks_geometry_and_caret_clock_without_idle_refreshes() -> Result {
    let input = "another line\n".repeat(120);
    let mut window = Session::new(&input)?;
    let first = window.snapshot()?.clone();
    assert!(!first.geometry.is_empty());
    let deadline = window.time + Duration::from_millis(500);
    assert_eq!(window.scheduler.schedule().deadline, Some(deadline));
    window.wait_preview()?;
    assert!(window.paint()?.is_empty());
    assert!(window.flush()?.is_empty());
    assert!(window.dispatch(WindowEvent::Wake)?.is_empty());
    assert!(!window.scheduler.schedule().request_frame);

    window.native(TextInputEdit::Command(TextEditingCommand::MoveLeft {
        select: false,
    }))?;
    let acknowledged = window.snapshot()?;
    assert_eq!(acknowledged.acknowledged, first.acknowledged + 1);
    assert_eq!(acknowledged.geometry, first.geometry);
    assert_eq!(acknowledged.selection, first.selection);
    assert!(window.flush()?.is_empty());

    window.time = deadline;
    assert!(
        window
            .dispatch(WindowEvent::Frame {
                predicted_presentation_timestamp: window.time,
                refresh_interval: Duration::from_millis(16),
            })?
            .is_empty()
    );
    assert_eq!(
        window.scheduler.schedule().deadline,
        Some(deadline + Duration::from_millis(500))
    );

    window.dispatch(WindowEvent::ScaleFactorChanged { scale_factor: 1.5 })?;
    let resized = window.snapshot()?.clone();
    assert_eq!(resized.session, first.session);
    assert_ne!(resized.geometry, first.geometry);
    let source = window.app.ui.source.node().ok_or("Missing source")?;
    let origin = window.app.tree.bounds(source)?.origin();
    let timestamp = window.tick();
    window.dispatch(WindowEvent::Wheel {
        position: Point::new(origin.x() + 20., origin.y() + 20.),
        delta_x: 0.,
        delta_y: 200.,
        timestamp,
    })?;
    assert!(window.snapshot()?.caret.origin().y() < resized.caret.origin().y());
    let after_wheel = window.snapshot()?.caret.origin().y();
    let timestamp = window.tick();
    window.dispatch(WindowEvent::Scroll {
        position: Point::new(origin.x() + 20., origin.y() + 20.),
        delta_x: 0.,
        delta_y: -100.,
        phase: oxiview_platform::ScrollPhase::Begin(oxiview_platform::ScrollMomentum::Native),
        timestamp,
    })?;
    assert!(window.snapshot()?.caret.origin().y() > after_wheel);
    assert!(window.scheduler.schedule().deadline.is_none());
    assert!(window.flush()?.is_empty());

    let timestamp = window.tick();
    assert_eq!(
        window.dispatch(WindowEvent::VisibilityChanged {
            visible: false,
            timestamp,
        })?,
        [PlatformCommand::TextInput(None)]
    );
    let timestamp = window.tick();
    window.dispatch(WindowEvent::ActivationChanged {
        active: false,
        timestamp,
    })?;
    let timestamp = window.tick();
    assert!(
        window
            .dispatch(WindowEvent::ActivationChanged {
                active: true,
                timestamp
            })?
            .is_empty()
    );
    assert!(window.input.is_none());
    assert!(window.scheduler.schedule().deadline.is_none());

    let timestamp = window.tick();
    window.dispatch(WindowEvent::VisibilityChanged {
        visible: true,
        timestamp,
    })?;
    assert_ne!(window.snapshot()?.session, first.session);
    assert_eq!(
        window.scheduler.schedule().deadline,
        Some(timestamp + Duration::from_millis(500))
    );
    let timestamp = window.tick();
    assert_eq!(
        window.dispatch(WindowEvent::ActivationChanged {
            active: false,
            timestamp,
        })?,
        [PlatformCommand::TextInput(None)]
    );
    assert!(window.paint()?.is_empty());
    for _ in 0..10 {
        assert!(window.dispatch(WindowEvent::Wake)?.is_empty());
        let schedule = window.scheduler.schedule();
        assert!(!schedule.request_frame);
        assert!(schedule.deadline.is_none());
    }
    Ok(())
}

#[test]
fn status_tracks_unicode_source_positions_and_published_diagnostics() -> Result {
    let prefix = "é".repeat(409);
    let source = format!("{prefix}\r\n二\u{2028}ok\n\nA bad $\\frac{{1}}{{$ and more text.\n");
    let mut window = Session::new(&source)?;
    window.wait_preview()?;
    let status = window.app.ui.slots.status.get()?;
    assert_eq!(window.app.preview.diagnostics(), 1);
    assert!(
        window
            .app
            .tree
            .text_source(status)?
            .contains("1 preview issue")
    );
    let preview_revision = window.app.preview.submitted;

    let position = window.app.ui.slots.position.get()?;
    for (offset, expected) in [
        (prefix.len(), "Ln 1   Col 410"),
        (prefix.len() + 2, "Ln 2   Col 1"),
        (prefix.len() + 5, "Ln 2   Col 2"),
        (prefix.len() + 8, "Ln 3   Col 1"),
    ] {
        window
            .app
            .source()
            .borrow_mut()
            .set_selection(Selection::caret(offset))?;
        window.app.refresh(&window.app.ui.source.clone())?;
        window.dispatch(WindowEvent::Wake)?;
        assert!(window.app.tree.text_source(position)?.starts_with(expected));
    }
    window.native(TextInputEdit::SetMarked {
        range: window.snapshot()?.selection.clone(),
        text: "shan".into(),
        selection: 4..4,
    })?;
    assert_eq!(window.app.preview.submitted, preview_revision);
    window.native(TextInputEdit::Cancel)?;
    assert_eq!(window.app.preview.submitted, preview_revision);
    window.shortcut(KeyCode::KeyA, false)?;
    window.type_text("A good $x^2$ formula.")?;
    window.wait_preview()?;
    assert_eq!(window.app.preview.submitted, preview_revision + 1);
    assert_eq!(window.app.preview.diagnostics(), 0);
    assert_eq!(window.app.tree.text_source(status)?, "Ready");
    Ok(())
}
