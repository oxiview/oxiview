//! Deterministic acceptance through the application's normal key and pointer dispatch.

use crate::{
    app::Demo,
    shortcuts::{CLOSE, OPEN, shortcut_label},
};
use oxiview_base::Point;
use oxiview_core::{Application, FrameTiming, NodeId, Scheduler};
use oxiview_platform::{
    KeyCode, KeyModifiers, LogicalKey, PhysicalSize, PointerButton, SystemAppearance,
    SystemPreferences, WindowEvent,
};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_scene::PixelSize;
use oxiview_style::AppearanceMode;
use serde::Serialize;
use snake::{
    application::Config,
    game::{Direction, Phase},
};
use std::{
    error::Error,
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, PartialEq, Serialize)]
/// Named checks and output from one complete game and settings replay.
pub struct Report {
    /// True after every assertion and image write succeeds.
    pub passed: bool,
    /// Behaviors exercised by the run.
    pub checks: Vec<&'static str>,
    /// Owned image filenames relative to the output directory.
    pub images: Vec<String>,
    /// Actual points collected using the configured keyboard bindings.
    pub score: u32,
    /// Number of screen mounts before the simulated restart.
    pub screen_mounts: u64,
    /// Additional frame requests across the idle interval.
    pub idle_frames: u64,
    /// Active shortcut after restarting with the saved settings.
    pub restored_shortcut: String,
}

fn require(ok: bool, message: &'static str) -> Result<()> {
    if ok { Ok(()) } else { Err(message.into()) }
}

struct Driver {
    app: Demo,
    scheduler: Scheduler,
    now: Duration,
}
impl Driver {
    fn new(config: Config, prefs: &Path, theme: Option<&Path>, builder: bool) -> Result<Self> {
        let mut app = Demo::new(config, prefs.into(), None, builder)?;
        app.initialize_preferences(SystemPreferences {
            appearance: Some(SystemAppearance::Light),
            ..Default::default()
        })?;
        if let Some(path) = theme {
            app.game
                .tree_mut()
                .set_theme(oxiview_style::Theme::from_toml(&fs::read_to_string(path)?)?)?;
        }
        Ok(Self {
            app,
            scheduler: Scheduler::default(),
            now: Duration::ZERO,
        })
    }
    fn event(&mut self, event: WindowEvent) -> Result<()> {
        self.app.process(&event, &mut self.scheduler)?;
        Ok(())
    }
    fn key(&mut self, code: KeyCode) -> Result<()> {
        self.stroke(code, code.us_logical_key(), KeyModifiers::default())
    }
    fn stroke(
        &mut self,
        code: KeyCode,
        logical: LogicalKey,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        for pressed in [true, false] {
            self.now += Duration::from_millis(1);
            self.event(WindowEvent::Key {
                code,
                logical: logical.clone(),
                modifiers,
                pressed,
                repeat: false,
                timestamp: self.now,
            })?;
        }
        Ok(())
    }
    fn tab_to(&mut self, target: Option<NodeId>) -> Result<()> {
        let target = target.ok_or("Tab target has not mounted")?;
        for _ in 0..=self.app.game.tree().len() {
            if self.app.game.tree().focused_node() == Some(target) {
                return Ok(());
            }
            self.key(KeyCode::Tab)?;
        }
        Err("Tab traversal never reached its target".into())
    }
    fn click(&mut self, id: Option<NodeId>) -> Result<()> {
        let rect = self
            .app
            .game
            .tree()
            .bounds(id.ok_or("click target has not mounted")?)?;
        let position = Point::new(
            rect.origin().x() + rect.size().width() / 2.0,
            rect.origin().y() + rect.size().height() / 2.0,
        );
        self.event(WindowEvent::PointerMoved {
            position,
            timestamp: self.now,
        })?;
        for pressed in [true, false] {
            self.now += Duration::from_millis(1);
            self.event(WindowEvent::PointerButton {
                button: PointerButton::Primary,
                pressed,
                position,
                timestamp: self.now,
            })?;
        }
        Ok(())
    }
    fn tick(&mut self) -> Result<()> {
        self.now = self
            .app
            .game
            .next_deadline()
            .ok_or("game has no tick deadline")?;
        self.event(WindowEvent::Frame {
            predicted_presentation_timestamp: self.now,
            refresh_interval: Duration::from_nanos(16_666_667),
        })
    }
    fn image(&mut self, config: Config, scale: f64, path: &Path) -> Result<[u8; 4]> {
        let width = (f64::from(config.width) * scale).round() as u32;
        let height = (f64::from(config.height) * scale).round() as u32;
        let frame = self
            .app
            .game
            .render(PhysicalSize { width, height }, scale)?;
        let image = CpuRenderer.render(&frame.scene, frame.root, PixelSize { width, height })?;
        let mut encoder = png::Encoder::new(BufWriter::new(fs::File::create(path)?), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(image.pixels())?;
        writer.finish()?;
        Ok(image.pixels()[..4].try_into()?)
    }
    fn settle(&mut self, config: Config) -> Result<()> {
        if self.scheduler.schedule().request_frame && self.scheduler.frame() {
            self.app.game.render(
                PhysicalSize {
                    width: config.width,
                    height: config.height,
                },
                1.0,
            )?;
            self.scheduler.produced(
                FrameTiming {
                    kind: oxiview_core::FrameKind::Vsync,
                    predicted_presentation_timestamp: self.now,
                    refresh_interval: Duration::from_millis(16),
                },
                self.now,
            );
        }
        Ok(())
    }
}

struct PlayerDir(PathBuf);
impl PlayerDir {
    fn new(output: &Path) -> std::io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let dir = output.join(format!(
                ".replay-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }
}
impl Drop for PlayerDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn checks(config: Config, output: &Path, theme: Option<&Path>, builder: bool) -> Result<Report> {
    let dir = PlayerDir::new(output)?;
    let settings = dir.0.join("settings.json");
    let mut d = Driver::new(config, &settings, theme, builder)?;
    let mut checks = Vec::new();
    let mut images = Vec::new();
    let ready = "ready.png";
    d.image(config, 1.0, &output.join(ready))?;
    images.push(ready.into());
    d.key(KeyCode::Enter)?;
    require(
        d.app.game.snapshot().phase == Phase::Running,
        "Enter did not start the board through its Action",
    )?;
    while d.app.game.snapshot().score < 30 {
        if d.now > Duration::from_secs(240) {
            return Err("gameplay replay exceeded its time budget".into());
        }
        let snapshot = d.app.game.snapshot();
        let direction = snake::replay::route(&snapshot).ok_or("no safe path to food")?;
        d.key(match direction {
            Direction::Up => KeyCode::ArrowUp,
            Direction::Right => KeyCode::ArrowRight,
            Direction::Down => KeyCode::ArrowDown,
            Direction::Left => KeyCode::ArrowLeft,
        })?;
        d.tick()?;
    }
    let score = d.app.game.snapshot().score;
    checks.push("board Actions steer, collect three foods and preserve the mounted screen");

    let primary = oxiview_platform::ShortcutPlatform::current().primary_modifier();
    d.stroke(KeyCode::KeyK, LogicalKey::Character("k".into()), primary)?;
    require(
        d.app.game.snapshot().phase == Phase::Running,
        "a sequence prefix fired an Action",
    )?;
    d.key(KeyCode::KeyP)?;
    require(
        d.app.game.snapshot().phase == Phase::Paused,
        "the two-stroke Pause shortcut did not fire",
    )?;
    d.key(KeyCode::Space)?;
    require(
        d.app.game.snapshot().phase == Phase::Running,
        "board failed to resume after the sequence",
    )?;
    checks.push("the board resolves a two-stroke shortcut once at its final stroke");

    let root = d.app.game.tree().root();
    require(
        d.app
            .game
            .tree()
            .actions(root)?
            .iter()
            .any(|a| a.id == OPEN),
        "window settings Action missing",
    )?;
    d.stroke(KeyCode::Comma, LogicalKey::Character(",".into()), primary)?;
    let controls = d.app.settings.controls.clone();
    require(
        d.app.settings.open.get()?
            && d.app.game.snapshot().phase == Phase::Paused
            && d.app.game.tree().focused_node() == controls.appearance.get(),
        "Open Settings did not pause and focus Appearance",
    )?;
    let before = d.app.game.snapshot();
    let light = d.image(config, 1.0, &output.join("settings-system.png"))?;
    images.push("settings-system.png".into());
    d.key(KeyCode::ArrowLeft)?;
    require(
        d.app.game.tree().appearance_mode() == AppearanceMode::Dark
            && d.app.game.snapshot() == before,
        "Appearance arrow leaked into the board",
    )?;
    for (name, scale) in [
        ("settings-dark-1x.png", 1.0),
        ("settings-dark-1_5x.png", 1.5),
        ("settings-dark-2x.png", 2.0),
    ] {
        let dark = d.image(config, scale, &output.join(name))?;
        if theme.is_none() {
            require(
                dark != light,
                "the visible window background did not follow the palette",
            )?;
        }
        images.push(name.into());
    }
    checks.push("nested arrow bindings switch the visible palette without changing game state");
    d.tab_to(controls.record.get())?;
    d.key(KeyCode::Enter)?;
    require(
        d.app.settings.recording.get()?,
        "record control did not arm",
    )?;
    let original = shortcut_label(&d.app.map, "snake.turn.up");
    d.key(KeyCode::ArrowRight)?;
    require(
        d.app.settings.recording.get()?
            && shortcut_label(&d.app.map, "snake.turn.up") == original
            && d.app.settings.status.get()?.contains("Already assigned"),
        "a conflicting override changed the map",
    )?;
    d.stroke(KeyCode::KeyK, LogicalKey::Character("k".into()), primary)?;
    require(
        d.app.settings.recording.get()? && d.app.settings.status.get()?.contains("Pause"),
        "recording accepted the prefix of the two-stroke Pause binding",
    )?;
    d.key(KeyCode::KeyI)?;
    require(
        !d.app.settings.recording.get()? && !d.app.dirty_save && settings.is_file(),
        "valid shortcut was not saved",
    )?;
    checks.push("shortcut recording consumes the key, rejects conflicts and saves the replacement");

    let board = d.app.board()?;
    d.tab_to(Some(board))?;
    require(
        d.app.settings.open.get()?,
        "Tab unexpectedly closed settings",
    )?;
    let panel = controls.panel.get().ok_or("settings panel missing")?;
    require(
        d.app.game.tree_mut().invoke_action(panel, CLOSE)?,
        "programmatic close was not delivered",
    )?;
    d.event(WindowEvent::Wake)?;
    require(
        d.app.game.tree().focused_node() == Some(board)
            && d.app.game.snapshot().phase == Phase::Paused,
        "closing settings stole focus or resumed play",
    )?;
    d.click(controls.opener.get())?;
    d.key(KeyCode::Escape)?;
    require(
        !d.app.settings.open.get()? && d.app.game.tree().focused_node() == controls.opener.get(),
        "Escape did not restore the opener",
    )?;
    checks
        .push("Tab enters and leaves inline settings and closing restores only an in-panel focus");

    d.settle(config)?;
    let board_updates = d.app.game.counters().board_updates.load(Ordering::SeqCst);
    let mut idle_frames = 0;
    for _ in 0..1000 {
        d.now += Duration::from_millis(16);
        d.event(WindowEvent::Frame {
            predicted_presentation_timestamp: d.now,
            refresh_interval: Duration::from_millis(16),
        })?;
        if d.scheduler.schedule().request_frame {
            idle_frames += 1;
        }
    }
    require(
        idle_frames == 0
            && d.app.game.counters().board_updates.load(Ordering::SeqCst) == board_updates,
        "idle window requested work",
    )?;
    let mounts = d.app.game.counters().screen_mounts.load(Ordering::SeqCst);
    require(
        mounts == 1
            && d.app
                .game
                .counters()
                .rejected_commands
                .load(Ordering::SeqCst)
                == 0,
        "replay remounted or rejected mutations",
    )?;
    checks.push("1,000 idle opportunities produce zero frame requests and zero board updates");
    drop(d);

    let mut restarted = Driver::new(config, &settings, theme, builder)?;
    let restored_shortcut = shortcut_label(&restarted.app.map, "snake.turn.up");
    require(
        restored_shortcut == "I"
            && restarted.app.game.tree().appearance_mode() == AppearanceMode::Dark,
        "fresh instance did not restore saved preferences",
    )?;
    restarted.key(KeyCode::Enter)?;
    let initial_direction = restarted.app.game.snapshot().direction;
    restarted.key(KeyCode::ArrowUp)?;
    restarted.tick()?;
    require(
        restarted.app.game.snapshot().direction == initial_direction,
        "old shortcut still steers",
    )?;
    restarted.key(KeyCode::KeyI)?;
    restarted.tick()?;
    require(
        restarted.app.game.snapshot().direction == Direction::Up,
        "restored shortcut does not steer",
    )?;
    let running = "playing-restored.png";
    restarted.image(config, 1.0, &output.join(running))?;
    images.push(running.into());
    for _ in 0..128 {
        if restarted.app.game.snapshot().phase != Phase::Running {
            break;
        }
        restarted.tick()?;
    }
    require(
        matches!(restarted.app.game.snapshot().phase, Phase::Over(_))
            && !restarted.app.game.history_ids()?.is_empty(),
        "collision or keyed history failed",
    )?;
    checks.push(
        "a fresh instance restores actual bindings, collision ends the run and history mounts",
    );
    Ok(Report {
        passed: true,
        checks,
        images,
        score,
        screen_mounts: mounts,
        idle_frames,
        restored_shortcut,
    })
}

/// Runs the precompiled settings screen. The output contains images and a pass/fail manifest.
pub fn run(config: Config, output: &Path, theme: Option<&Path>) -> Result<Report> {
    run_with_panel(config, output, theme, false)
}

/// Runs the identical acceptance journey with either the Rust or precompiled settings panel.
pub fn run_with_panel(
    config: Config,
    output: &Path,
    theme: Option<&Path>,
    builder: bool,
) -> Result<Report> {
    fs::create_dir_all(output)?;
    let manifest = output.join("report.json");
    fs::write(&manifest, b"{\"passed\":false,\"status\":\"running\"}")?;
    match checks(config, output, theme, builder) {
        Ok(report) => {
            fs::write(manifest, serde_json::to_vec_pretty(&report)?)?;
            Ok(report)
        }
        Err(e) => {
            let _ = fs::write(
                manifest,
                serde_json::to_vec_pretty(
                    &serde_json::json!({"passed":false,"error":e.to_string()}),
                )?,
            );
            Err(e)
        }
    }
}
