//! Wires Snake's simulation to the window's focus, preferences and Action contexts.

use crate::{
    Ui, screen,
    settings::{Command, Settings, storage::Preferences},
    shortcuts::{
        self, DIRECTIONS, EDITABLE, ENTER, OPEN, PAUSE, PRIMARY, action_id, shortcut_label,
    },
};
use oxiview_app::ThemeFile;
use oxiview_core::{
    Action, Application, Error, FrameSubmission, FrameTiming, KeyChord, Keymap, NodeId,
    PaintedScene, Scheduler, ShortcutKey,
};
use oxiview_platform::{
    KeyCode, LogicalKey, PhysicalSize, SystemPreferences, WindowEvent, WindowWaker,
};
use oxiview_reactive::{Scope, batch};
use oxiview_style::Theme;
use snake::{
    application::{Action as GameAction, Config, Counters, SnakeApp},
    game::Phase,
};
use std::{
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
};

pub(crate) fn error(e: impl std::fmt::Display) -> Error {
    Error::Application(e.to_string())
}

pub(crate) struct Demo {
    pub(crate) game: SnakeApp,
    pub(crate) settings: Settings,
    _scope: Scope,
    commands: Receiver<Command>,
    pub(crate) map: Keymap,
    prefs: Preferences,
    path: PathBuf,
    selected: usize,
    pub(crate) dirty_save: bool,
    theme_path: Option<PathBuf>,
    watcher: Option<ThemeFile>,
}

impl Demo {
    pub(crate) fn new(
        config: Config,
        path: PathBuf,
        theme_path: Option<PathBuf>,
        builder_panel: bool,
    ) -> Result<Self, Error> {
        let (prefs, load_error) = match Preferences::read(&path) {
            Ok(prefs) => (prefs, None),
            Err(message) => (
                Preferences::default(),
                Some(format!("Could not load {}: {message}", path.display())),
            ),
        };
        let map = prefs.keymap().map_err(error)?;
        let (send, commands) = mpsc::channel();
        let scope = Scope::new().map_err(error)?;
        let settings = scope
            .run(|| Settings::new(send, builder_panel))
            .map_err(error)?;
        let screen_settings = settings.clone();
        let mut game =
            SnakeApp::with_action_screen(config, Arc::new(Counters::default()), move |props| {
                screen::screen(Ui {
                    game: props.ui,
                    settings: screen_settings,
                })
            })?;
        game.tree_mut()
            .set_theme(Theme::from_toml(include_str!("../theme.toml")).map_err(error)?)?;
        game.tree_mut()
            .set_appearance_mode(prefs.appearance.mode())?;
        let mut app = Self {
            game,
            settings,
            _scope: scope,
            commands,
            map,
            prefs,
            path,
            selected: 0,
            dirty_save: false,
            theme_path,
            watcher: None,
        };
        app.install_game_actions()?;
        app.update_labels()?;
        if let Some(message) = load_error {
            eprintln!("snake-m5-oxi: {message}");
            app.settings.status.set(message).map_err(error)?;
        }
        Ok(app)
    }
    pub(crate) fn board(&self) -> Result<NodeId, Error> {
        self.game
            .ui()
            .slots
            .board
            .get()
            .ok_or_else(|| error("board has not mounted"))
    }
    fn install_game_actions(&mut self) -> Result<(), Error> {
        let board = self.board()?;
        let ui = Ui {
            game: self.game.ui().clone(),
            settings: self.settings.clone(),
        };
        let root = self.game.tree().root();
        let tree = self.game.tree_mut();
        tree.set_keymap(root, shortcuts::window_keymap().map_err(error)?)?;
        let open = ui.settings.send.clone();
        tree.register_action(root, Action::new(OPEN, "Open settings"), move |_, _, _| {
            let _ = open.send(Command::Open);
        })?;
        for (direction, _, _, caption) in DIRECTIONS {
            let action = GameAction::Turn(direction);
            let target = ui.clone();
            tree.register_action(
                board,
                Action::new(action_id(action), caption),
                move |_, _, commands| target.queue_game(action, commands),
            )?;
        }
        for (id, caption, action) in [
            (PRIMARY, "Start / pause / resume", GameAction::Primary),
            (PAUSE, "Pause", GameAction::Pause),
        ] {
            let target = ui.clone();
            tree.register_action(board, Action::new(id, caption), move |_, _, commands| {
                target.queue_game(action, commands)
            })?;
        }
        tree.register_action(
            board,
            Action::new(ENTER, "Start / new game"),
            move |_, _, commands| {
                let action = if ui.phase.get().is_ok_and(|p| p == Phase::Ready) {
                    GameAction::Primary
                } else {
                    GameAction::NewGame
                };
                ui.queue_game(action, commands);
            },
        )?;
        tree.set_keymap(board, self.map.clone())?;
        tree.focus(board)?;
        Ok(())
    }

    fn update_labels(&self) -> Result<(), Error> {
        let (id, caption) = EDITABLE[self.selected];
        let keys = shortcut_label(&self.map, id);
        let primary = shortcut_label(&self.map, PRIMARY);
        let restart = shortcut_label(&self.map, ENTER);
        batch(|| {
            self.settings
                .palette_label
                .set_if_changed(self.prefs.appearance.label().into())?;
            self.settings.action_label.set_if_changed(caption.into())?;
            self.settings.shortcut_label.set_if_changed(keys)?;
            self.settings
                .footer
                .set_if_changed(format!("{primary}  pause   ·   {restart}  new game"))?;
            Ok(())
        })
        .map_err(error)
    }
    fn save(&mut self) -> Result<(), Error> {
        let message = match self.prefs.save(&self.path) {
            Ok(()) => {
                self.dirty_save = false;
                "Saved.".into()
            }
            Err(e) => {
                self.dirty_save = true;
                format!("Unsaved · F5 to retry. {e}")
            }
        };
        self.settings
            .status
            .set_if_changed(message)
            .map_err(error)?;
        Ok(())
    }
    fn change_section(&mut self, open: bool, scheduler: &mut Scheduler) -> Result<(), Error> {
        let controls = Rc::clone(&self.settings.controls);
        if !self.settings.open.set_if_changed(open).map_err(error)? {
            return Ok(());
        }
        if open && self.game.snapshot().phase == Phase::Running {
            self.game.ui().request_action(GameAction::Pause);
            self.game.process(&WindowEvent::Wake, scheduler)?;
        }
        self.settings
            .recording
            .set_if_changed(false)
            .map_err(error)?;
        self.game.flush_screen(scheduler)?;
        if open {
            let panel = controls
                .panel
                .get()
                .ok_or_else(|| error("settings did not mount"))?;
            self.game
                .tree_mut()
                .set_focus_return_target(panel, controls.opener.get())?;
            if let Some(id) = controls.appearance.get() {
                self.game.tree_mut().focus(id)?;
            }
        }
        Ok(())
    }
    fn perform(&mut self, command: Command, scheduler: &mut Scheduler) -> Result<(), Error> {
        match command {
            Command::Open => self.change_section(true, scheduler)?,
            Command::Close => self.change_section(false, scheduler)?,
            Command::Appearance(forward) => {
                self.prefs.appearance = self.prefs.appearance.step(forward);
                if self
                    .game
                    .tree_mut()
                    .set_appearance_mode(self.prefs.appearance.mode())?
                {
                    scheduler.invalidate();
                }
                self.save()?;
            }
            Command::SelectAction(forward) => {
                let len = EDITABLE.len();
                self.selected = (self.selected + if forward { 1 } else { len - 1 }) % len;
                self.settings
                    .recording
                    .set_if_changed(false)
                    .map_err(error)?;
            }
            Command::Record => {
                self.settings
                    .recording
                    .set_if_changed(true)
                    .map_err(error)?;
                self.settings
                    .status
                    .set_if_changed("Press a shortcut. Escape cancels.".into())
                    .map_err(error)?;
            }
            Command::Retry => self.save()?,
        }
        self.update_labels()?;
        Ok(())
    }

    fn record(
        &mut self,
        code: KeyCode,
        logical: &LogicalKey,
        modifiers: oxiview_platform::KeyModifiers,
    ) -> Result<(), Error> {
        let action = EDITABLE[self.selected].0;
        // Reject an effective collision even if a physical default and a recorded
        // logical key have different identities on another keyboard layout.
        let collision = self.map.effective_bindings().find(|binding| {
            binding.action != action
                && binding.sequence.first().is_some_and(|first| {
                    first.modifiers == modifiers
                        && match &first.key {
                            ShortcutKey::Physical(key) => *key == code,
                            ShortcutKey::Logical(key) => key == logical,
                        }
                })
        });
        if let Some(binding) = collision {
            let caption = EDITABLE
                .iter()
                .find(|entry| entry.0 == binding.action)
                .map_or(binding.action.as_str(), |entry| entry.1);
            self.settings
                .status
                .set_if_changed(format!("Already assigned to {caption}. Try another key."))
                .map_err(error)?;
            return Ok(());
        }
        let chord = if *logical == LogicalKey::Unidentified {
            KeyChord::physical(code, modifiers)
        } else {
            KeyChord::logical(logical.clone(), modifiers)
        };
        match self.prefs.rebind(action, &chord) {
            Ok((prefs, map)) => {
                let board = self.board()?;
                self.game.tree_mut().set_keymap(board, map.clone())?;
                self.prefs = prefs;
                self.map = map;
                self.settings
                    .recording
                    .set_if_changed(false)
                    .map_err(error)?;
                self.save()?;
                self.update_labels()?;
            }
            Err(message) => {
                self.settings
                    .status
                    .set_if_changed(message)
                    .map_err(error)?;
            }
        }
        Ok(())
    }
    pub(crate) fn process(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
    ) -> Result<(), Error> {
        let mut consume = false;
        if self.settings.recording.get().map_err(error)?
            && let WindowEvent::Key {
                code,
                logical,
                modifiers,
                pressed,
                repeat,
                ..
            } = event
        {
            if *pressed && !*repeat {
                match logical {
                    LogicalKey::Named(KeyCode::Tab | KeyCode::Escape) => {
                        self.settings.recording.set(false).map_err(error)?;
                        self.settings
                            .status
                            .set_if_changed("Recording canceled.".into())
                            .map_err(error)?;
                    }
                    _ => self.record(*code, logical, *modifiers)?,
                }
            }
            consume = !matches!(logical, LogicalKey::Named(KeyCode::Tab));
        }
        if consume {
            self.game.process_captured_key(event, scheduler)?;
        } else {
            self.game.process(event, scheduler)?;
        }
        if matches!(event, WindowEvent::Destroyed) {
            self.watcher.take();
            return Ok(());
        }
        if self.settings.recording.get().map_err(error)?
            && (matches!(
                event,
                WindowEvent::ActivationChanged { active: false, .. }
                    | WindowEvent::VisibilityChanged { visible: false, .. }
            ) || self.game.tree().focused_node() != self.settings.controls.record.get())
        {
            self.settings.recording.set(false).map_err(error)?;
            self.settings
                .status
                .set_if_changed("Recording canceled.".into())
                .map_err(error)?;
        }
        while let Ok(command) = self.commands.try_recv() {
            self.perform(command, scheduler)?;
        }
        self.game.flush_screen(scheduler)
    }
}
impl Application for Demo {
    fn initialize_preferences(&mut self, prefs: SystemPreferences) -> Result<(), Error> {
        self.game.tree_mut().set_system_preferences(prefs)?;
        Ok(())
    }
    fn event(
        &mut self,
        e: &WindowEvent,
        scheduler: &mut Scheduler,
        waker: &WindowWaker,
    ) -> Result<(), Error> {
        self.game.set_waker(waker);
        if let Some(path) = self.theme_path.take() {
            match ThemeFile::watch(&path, waker) {
                Ok(source) => self.watcher = Some(source),
                Err(err) => {
                    self.settings
                        .theme_error
                        .set(Some(format!("Theme: {err}")))
                        .map_err(error)?;
                    eprintln!("snake-m5-oxi: {err}");
                }
            }
        }
        if matches!(e, WindowEvent::Wake)
            && let Some(source) = &mut self.watcher
        {
            match source.apply_latest(self.game.tree_mut()) {
                Ok(Some(changed)) => {
                    self.settings
                        .theme_error
                        .set_if_changed(None)
                        .map_err(error)?;
                    if changed {
                        scheduler.invalidate();
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    self.settings
                        .theme_error
                        .set(Some(format!("Theme: {err}")))
                        .map_err(error)?;
                    eprintln!("snake-m5-oxi: {err}");
                }
            }
        }
        self.process(e, scheduler)
    }
    fn paint(
        &mut self,
        timing: FrameTiming,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, Error> {
        self.game.paint(timing, size, scale)
    }
    fn frame_submitted(&mut self, frame: FrameSubmission) {
        self.game.frame_submitted(frame);
    }
}
