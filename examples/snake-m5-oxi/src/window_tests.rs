//! Drives the acceptance demo through a real headless window and CPU presenter.

use crate::{app::Demo, settings::storage::Preferences};
use oxiview_app::{HeadlessOutput, run_headless};
use oxiview_base::Size;
use oxiview_core::{
    Application, Error, FrameKind, FrameSubmission, FrameTiming, KeyChord, Keymap, PaintedScene,
    Scheduler,
};
use oxiview_platform::{
    KeyCode, KeyModifiers, LogicalKey, PhysicalSize, SystemAppearance, SystemPreferences,
    WindowAttributes, WindowEvent, WindowId, WindowWaker,
};
use oxiview_platform_headless::{Driver, HeadlessPlatform, KeyTransition, ResizeOutcome, WaitKind};
use snake::{application::Config, game::Phase};
use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
const INTERVAL: Duration = Duration::from_millis(10);
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(10);
const SCENARIO_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, PartialEq, Eq)]
enum Focus {
    Board,
    Appearance,
    Selection,
    Record,
}

struct State {
    phase: Phase,
    focus: Option<Focus>,
    appearance: Option<SystemAppearance>,
    reduced_motion: Option<bool>,
    settings_open: bool,
    recording: bool,
    dirty_save: bool,
    status: String,
    turn_up_shortcuts: Vec<KeyChord>,
}

fn turn_up_shortcuts(map: &Keymap) -> Vec<KeyChord> {
    map.effective_bindings()
        .filter(|binding| binding.action == "snake.turn.up")
        .flat_map(|binding| binding.sequence.iter().cloned())
        .collect()
}

struct ObservedDemo {
    demo: Demo,
    events: Sender<(WindowEvent, State)>,
    submissions: Sender<FrameKind>,
}

impl ObservedDemo {
    fn state(&self) -> Result<State, Error> {
        let tree = self.demo.game.tree();
        let preferences = tree.preference_signals();
        let settings = &self.demo.settings;
        let focused = tree.focused_node();
        let focus = [
            (Some(self.demo.board()?), Focus::Board),
            (settings.controls.appearance.get(), Focus::Appearance),
            (settings.controls.selection.get(), Focus::Selection),
            (settings.controls.record.get(), Focus::Record),
        ]
        .into_iter()
        .find_map(|(node, control)| (node.is_some() && node == focused).then_some(control));
        Ok(State {
            phase: self.demo.game.snapshot().phase,
            focus,
            appearance: preferences.appearance()?,
            reduced_motion: preferences.reduced_motion()?,
            settings_open: settings.open.get()?,
            recording: settings.recording.get()?,
            dirty_save: self.demo.dirty_save,
            status: settings.status.get()?,
            turn_up_shortcuts: turn_up_shortcuts(&self.demo.map),
        })
    }
}

impl Application for ObservedDemo {
    fn initialize_preferences(&mut self, preferences: SystemPreferences) -> Result<(), Error> {
        self.demo.initialize_preferences(preferences)
    }

    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        waker: &WindowWaker,
    ) -> Result<(), Error> {
        self.demo.event(event, scheduler, waker)?;
        if matches!(
            event,
            WindowEvent::Resized { .. }
                | WindowEvent::Key { .. }
                | WindowEvent::PreferencesChanged { .. }
        ) {
            let _ = self.events.send((event.clone(), self.state()?));
        }
        Ok(())
    }

    fn paint(
        &mut self,
        timing: FrameTiming,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, Error> {
        self.demo.paint(timing, size, scale)
    }

    fn frame_submitted(&mut self, frame: FrameSubmission) {
        self.demo.frame_submitted(frame);
        let _ = self.submissions.send(frame.timing.kind);
    }

    fn should_close(&mut self) -> bool {
        self.demo.should_close()
    }
}

struct Window {
    driver: Driver,
    id: WindowId,
    output: HeadlessOutput,
    events: Receiver<(WindowEvent, State)>,
    submissions: Receiver<FrameKind>,
}

impl Window {
    fn event(&self) -> TestResult<(WindowEvent, State)> {
        self.events
            .recv_timeout(RECEIVE_TIMEOUT)
            .map_err(|error| format!("waiting for a window event: {error}").into())
    }

    fn initial_frame(&self) -> TestResult<State> {
        let (event, state) = self.event()?;
        let WindowEvent::Resized { token, .. } = event else {
            return Err(format!("expected initial resize, got {event:?}").into());
        };
        self.submitted(FrameKind::Resize)?;
        assert_eq!(
            self.driver.resize_outcome(self.id, token)?,
            Some(ResizeOutcome::Acknowledged)
        );
        assert_eq!(self.output.snapshot()?.presents, 1);
        Ok(state)
    }

    fn press(&self, code: KeyCode, modifiers: KeyModifiers) -> TestResult<State> {
        self.press_logical(code, code.us_logical_key(), modifiers)
    }

    fn press_logical(
        &self,
        code: KeyCode,
        logical: LogicalKey,
        modifiers: KeyModifiers,
    ) -> TestResult<State> {
        for pressed in [true, false] {
            self.driver.key_transition(
                self.id,
                KeyTransition {
                    code,
                    logical: logical.clone(),
                    modifiers,
                    pressed,
                    repeat: false,
                    timestamp: None,
                },
            )?;
        }
        let (press, _) = self.event()?;
        let (release, state) = self.event()?;
        for (event, pressed) in [(press, true), (release, false)] {
            assert!(
                matches!(&event, WindowEvent::Key {
                    code: delivered_code, logical: delivered_logical, modifiers: delivered_modifiers,
                    pressed: delivered_pressed, repeat: false, ..
                } if (*delivered_code, delivered_logical, *delivered_modifiers, *delivered_pressed)
                    == (code, &logical, modifiers, pressed)),
                "unexpected key transition: {event:?}"
            );
        }
        Ok(state)
    }

    fn open_settings(&self) -> TestResult<State> {
        self.press(
            KeyCode::Comma,
            KeyModifiers {
                meta: cfg!(target_os = "macos"),
                control: !cfg!(target_os = "macos"),
                ..Default::default()
            },
        )
    }

    fn blocked(&self) -> TestResult<()> {
        self.driver.wait_until_blocked(self.id, WaitKind::Event)?;
        Ok(())
    }

    fn submitted(&self, kind: FrameKind) -> TestResult<()> {
        let submitted = self
            .submissions
            .recv_timeout(RECEIVE_TIMEOUT)
            .map_err(|error| format!("waiting for a {kind:?} submission: {error}"))?;
        assert_eq!(submitted, kind);
        self.blocked()
    }

    fn present_requested_frame(&self) -> TestResult<()> {
        // The event notice precedes frame arming. Wait for the scheduler to
        // finish handling the event before advancing to the next opportunity.
        self.blocked()?;
        self.driver.advance_by(INTERVAL)?;
        self.submitted(FrameKind::Vsync)
    }

    fn background(&self) -> TestResult<[u8; 4]> {
        let image = self.output.snapshot()?.image.ok_or("no presented image")?;
        image
            .pixels()
            .first_chunk()
            .copied()
            .ok_or_else(|| "presented image has no pixels".into())
    }

    fn assert_idle(&self) -> TestResult<()> {
        self.blocked()?;
        let frames = self.driver.counters(self.id)?.frames;
        let presents = self.output.snapshot()?.presents;
        for _ in 0..1_000 {
            self.driver.advance_by(INTERVAL)?;
        }
        self.blocked()?;
        assert_eq!(
            self.driver.counters(self.id)?.frames,
            frames,
            "idle frame delivery"
        );
        assert_eq!(
            self.output.snapshot()?.presents,
            presents,
            "idle presentation"
        );
        assert!(matches!(
            self.submissions.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        Ok(())
    }
}

struct SettingsDirectory(PathBuf);

impl SettingsDirectory {
    fn new(scenario: &str) -> TestResult<Self> {
        let root = std::env::home_dir()
            .ok_or("could not locate home directory")?
            .join(".tmp");
        fs::create_dir_all(&root)?;
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = root.join(format!(
            "snake-m5-oxi-{}-{timestamp}-{scenario}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn file(&self) -> PathBuf {
        self.0.join("settings.json")
    }
}

impl Drop for SettingsDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_case(
    settings_path: PathBuf,
    preferences: SystemPreferences,
    test: impl FnOnce(Window) -> TestResult + Send + 'static,
) -> TestResult {
    let (ready, window) = mpsc::channel();
    let (stopped, shutdown) = mpsc::channel();
    thread::spawn(move || {
        let run = || -> TestResult {
            let (platform, driver) = HeadlessPlatform::new_with_preferences(INTERVAL, preferences)?;
            let config = Config::default();
            let (events, observed_events) = mpsc::channel();
            let (submissions, observed_submissions) = mpsc::channel();
            run_headless(
                platform,
                WindowAttributes {
                    title: "Snake M5 window test".into(),
                    logical_size: Size::new(f64::from(config.width), f64::from(config.height)),
                    resize_timeout: Duration::from_secs(1),
                },
                move || {
                    Ok(ObservedDemo {
                        demo: Demo::new(config, settings_path, None, false)?,
                        events,
                        submissions,
                    })
                },
                move |_, id, output| {
                    ready
                        .send(Window {
                            driver,
                            id,
                            output,
                            events: observed_events,
                            submissions: observed_submissions,
                        })
                        .map_err(|error| {
                            oxiview_platform::Error::OperationFailed(error.to_string())
                        })
                },
            )?;
            Ok(())
        };
        let _ = stopped.send(run());
    });
    let window = window
        .recv_timeout(RECEIVE_TIMEOUT)
        .map_err(|error| format!("waiting for the headless window to open: {error}"))?;
    let driver = window.driver.clone();
    let id = window.id;
    let (checked, result) = mpsc::channel();
    thread::spawn(move || {
        let _ = checked.send(test(window));
    });

    // Keep the test bounded even if a regression prevents the driver from
    // reaching its next blocked-state handshake or the window from closing.
    let assertions = result
        .recv_timeout(SCENARIO_TIMEOUT)
        .map_err(|error| format!("waiting for the window test scenario: {error}"));
    let close = driver.request_user_close(id);
    shutdown
        .recv_timeout(RECEIVE_TIMEOUT)
        .map_err(|error| format!("waiting for the headless window to close: {error}"))??;
    assertions??;
    close?;
    Ok(())
}

#[test]
fn preferences_and_keyboard_focus_drive_real_frames_then_go_idle() -> TestResult {
    let settings = SettingsDirectory::new("preferences")?;
    let initial = SystemPreferences {
        appearance: Some(SystemAppearance::Dark),
        reduced_motion: Some(true),
        ..Default::default()
    };
    let changed = SystemPreferences {
        appearance: Some(SystemAppearance::Light),
        reduced_motion: Some(false),
        ..initial
    };
    run_case(settings.file(), initial, move |window| {
        let first = window.initial_frame()?;
        assert_eq!(first.phase, Phase::Ready);
        assert_eq!(first.focus, Some(Focus::Board));
        assert_eq!(first.appearance, initial.appearance);
        assert_eq!(first.reduced_motion, initial.reduced_motion);
        assert_eq!(window.background()?, [0x15, 0x23, 0x1e, 0xff]);

        let no_modifiers = KeyModifiers::default();
        assert!(!window.press(KeyCode::Comma, no_modifiers)?.settings_open);
        assert_eq!(
            window.press(KeyCode::Enter, no_modifiers)?.phase,
            Phase::Running
        );
        let opened = window.open_settings()?;
        assert!(opened.settings_open);
        assert_eq!(opened.phase, Phase::Paused);
        assert_eq!(opened.focus, Some(Focus::Appearance));
        assert_eq!(
            window.press(KeyCode::Tab, no_modifiers)?.focus,
            Some(Focus::Selection)
        );
        let shift = KeyModifiers {
            shift: true,
            ..no_modifiers
        };
        assert_eq!(
            window.press(KeyCode::Tab, shift)?.focus,
            Some(Focus::Appearance)
        );
        window.present_requested_frame()?;

        window.driver.preferences_changed(window.id, changed)?;
        let (event, updated) = window.event()?;
        assert_eq!(
            event,
            WindowEvent::PreferencesChanged {
                preferences: changed
            }
        );
        assert_eq!(updated.appearance, changed.appearance);
        assert_eq!(updated.reduced_motion, changed.reduced_motion);
        assert_eq!(updated.focus, Some(Focus::Appearance));
        window.present_requested_frame()?;
        assert_eq!(window.background()?, [0xf5, 0xf6, 0xef, 0xff]);
        window.assert_idle()
    })
}

#[test]
fn f5_saves_the_active_shortcut_after_a_filesystem_failure() -> TestResult {
    let settings = SettingsDirectory::new("retry")?;
    let path = settings.file();
    run_case(path.clone(), SystemPreferences::default(), move |window| {
        window.initial_frame()?;
        // Block the final atomic rename with a directory at the file path.
        fs::create_dir(&path)?;
        window.open_settings()?;
        let no_modifiers = KeyModifiers::default();
        window.press(KeyCode::Tab, no_modifiers)?;
        assert_eq!(
            window.press(KeyCode::Tab, no_modifiers)?.focus,
            Some(Focus::Record)
        );
        assert!(window.press(KeyCode::Enter, no_modifiers)?.recording);

        let logical = LogicalKey::Character("i".into());
        let modifiers = KeyModifiers {
            shift: true,
            control: true,
            ..no_modifiers
        };
        let expected = [KeyChord::logical(logical.clone(), modifiers)];
        let failed = window.press_logical(KeyCode::KeyK, logical, modifiers)?;
        assert!(!failed.recording);
        assert!(failed.dirty_save);
        assert!(
            failed.status.starts_with("Unsaved · F5 to retry."),
            "{}",
            failed.status
        );
        assert_eq!(failed.turn_up_shortcuts, expected);
        assert!(path.is_dir());
        window.present_requested_frame()?;

        fs::remove_dir(&path)?;
        let saved = window.press(KeyCode::F5, no_modifiers)?;
        assert!(!saved.dirty_save);
        assert_eq!(saved.status, "Saved.");
        assert_eq!(saved.turn_up_shortcuts, expected);
        assert!(path.is_file());
        assert_eq!(
            turn_up_shortcuts(&Preferences::read(&path)?.keymap()?),
            expected
        );
        window.present_requested_frame()
    })
}
