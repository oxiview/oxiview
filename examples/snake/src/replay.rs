//! Deterministic, display-independent acceptance replay and real-font images.

use crate::{
    application::{Action, Config, Counters, Screen, SnakeApp, screen},
    game::{Cell, Difficulty, Direction, Phase, Snapshot},
};
use oxiview_base::Point;
use oxiview_core::Scheduler;
use oxiview_platform::{KeyCode, PhysicalSize, PointerButton, WindowEvent};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_scene::{EncodedNode, Primitive};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    error::Error,
    fs::File,
    io::BufWriter,
    path::Path,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

/// One image generated from the application's actual text/Scene pipeline.
#[derive(Debug, PartialEq, Serialize)]
pub struct ImageEvidence {
    /// Output image path.
    pub path: String,
    /// Physical-pixel width.
    pub width: u32,
    /// Physical-pixel height.
    pub height: u32,
    /// Physical pixels per logical unit.
    pub scale: f64,
    /// Number of raster glyph-image draw operations.
    pub glyph_images: usize,
    /// Number of vector glyphs, including synthesized outline paths.
    pub vector_glyphs: usize,
    /// Pixel area of raster glyph draws, including repeated placements.
    pub glyph_pixels: u64,
}
/// A machine-readable acceptance result, independent of a native window.
#[derive(Debug, PartialEq, Serialize)]
pub struct Report {
    /// True only after every replay assertion passed.
    pub passed: bool,
    /// Input seed used for reproducibility.
    pub seed: u64,
    /// Whether the exported scenes include the optional diagnostics layer.
    pub debug_overlay: bool,
    /// Named checks that actually ran.
    pub checks: Vec<&'static str>,
    /// Generated real-font scenes.
    pub images: Vec<ImageEvidence>,
    /// Screen component factory invocations; expected to remain one.
    pub screen_mounts: u64,
    /// Conditional overlay factory invocations.
    pub overlay_mounts: u64,
    /// New keyed history-row component invocations.
    pub history_mounts: u64,
    /// Reactive board painting computations.
    pub board_updates: u64,
    /// Rejected deferred commands during ordinary replay interaction.
    pub rejected_commands: u64,
    /// Automatic jobs retired with conditional view scopes; these were not applied.
    pub canceled_bindings: u64,
    /// Best score produced by the replay's real gameplay inputs.
    pub best_score: u32,
}

fn require(condition: bool, message: &'static str) -> Result<(), Box<dyn Error>> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn key(
    app: &mut SnakeApp,
    scheduler: &mut Scheduler,
    code: KeyCode,
    now: Duration,
) -> Result<(), Box<dyn Error>> {
    app.process(
        &WindowEvent::Key {
            code,
            logical: code.us_logical_key(),
            modifiers: Default::default(),
            pressed: true,
            repeat: false,
            timestamp: now,
        },
        scheduler,
    )?;
    app.process(
        &WindowEvent::Key {
            code,
            logical: code.us_logical_key(),
            modifiers: Default::default(),
            pressed: false,
            repeat: false,
            timestamp: now,
        },
        scheduler,
    )?;
    Ok(())
}
fn direction_key(direction: Direction) -> KeyCode {
    match direction {
        Direction::Up => KeyCode::ArrowUp,
        Direction::Right => KeyCode::ArrowRight,
        Direction::Down => KeyCode::ArrowDown,
        Direction::Left => KeyCode::ArrowLeft,
    }
}
fn frame(
    app: &mut SnakeApp,
    scheduler: &mut Scheduler,
    now: Duration,
) -> Result<(), Box<dyn Error>> {
    app.process(
        &WindowEvent::Frame {
            predicted_presentation_timestamp: now,
            refresh_interval: Duration::from_nanos(16_666_667),
        },
        scheduler,
    )?;
    Ok(())
}
fn click(
    app: &mut SnakeApp,
    scheduler: &mut Scheduler,
    action: Action,
    now: &mut Duration,
) -> Result<(), Box<dyn Error>> {
    let bounds = app.control_bounds(action)?;
    let position = Point::new(
        bounds.origin().x() + bounds.size().width() / 2.0,
        bounds.origin().y() + bounds.size().height() / 2.0,
    );
    *now = now.saturating_add(Duration::from_millis(1));
    app.process(
        &WindowEvent::PointerMoved {
            position,
            timestamp: *now,
        },
        scheduler,
    )?;
    for pressed in [true, false] {
        *now = now.saturating_add(Duration::from_millis(1));
        app.process(
            &WindowEvent::PointerButton {
                button: PointerButton::Primary,
                pressed,
                position,
                timestamp: *now,
            },
            scheduler,
        )?;
    }
    Ok(())
}
fn draw(
    app: &mut SnakeApp,
    output: &Path,
    name: &str,
    logical_width: u32,
    logical_height: u32,
    scale: u32,
) -> Result<ImageEvidence, Box<dyn Error>> {
    let width = logical_width
        .checked_mul(scale)
        .ok_or("image width overflow")?;
    let height = logical_height
        .checked_mul(scale)
        .ok_or("image height overflow")?;
    let painted = app.render(PhysicalSize { width, height }, f64::from(scale))?;
    let encoded = painted.scene.encode(painted.root)?;
    let mut glyph_images = 0;
    let mut vector_glyphs = 0;
    let mut glyph_pixels = 0u64;
    for command in &encoded.nodes {
        if let EncodedNode::Draw { primitive, .. } = command {
            // Snake's board and controls use rectangle primitives. Images and
            // outline paths come from the text views and the text overlay.
            match primitive {
                Primitive::Image(_, image) => {
                    glyph_images += 1;
                    glyph_pixels = glyph_pixels
                        .saturating_add(u64::from(image.width()) * u64::from(image.height()));
                }
                Primitive::Glyph(_) | Primitive::FillPath(..) => vector_glyphs += 1,
                _ => {}
            }
        }
    }
    require(
        glyph_images > 80 && glyph_pixels > 1000,
        "the actual UI did not emit enough real font glyph images",
    )?;
    let image = CpuRenderer.render(
        &painted.scene,
        painted.root,
        oxiview_scene::PixelSize { width, height },
    )?;
    let path = output.join(name);
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(&path)?), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(image.pixels())?;
    writer.finish()?;
    Ok(ImageEvidence {
        path: path.display().to_string(),
        width,
        height,
        scale: f64::from(scale),
        glyph_images,
        vector_glyphs,
        glyph_pixels,
    })
}

/// Chooses a safe next turn for reproducible Snake acceptance runs.
pub fn route(snapshot: &Snapshot) -> Option<Direction> {
    let start = *snapshot.body.first()?;
    let food = snapshot.food?;
    let mut blocked: HashSet<Cell> = snapshot.body.iter().copied().collect();
    if let Some(tail) = snapshot.body.last() {
        blocked.remove(tail);
    }
    blocked.remove(&start);
    let mut visited = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    let mut previous = HashMap::<Cell, (Cell, Direction)>::new();
    while let Some(cell) = queue.pop_front() {
        if cell == food {
            let mut cursor = food;
            while let Some((parent, direction)) = previous.get(&cursor).copied() {
                if parent == start {
                    return Some(direction);
                }
                cursor = parent;
            }
        }
        for direction in [
            Direction::Up,
            Direction::Right,
            Direction::Down,
            Direction::Left,
        ] {
            if cell == start && direction == snapshot.direction.opposite() {
                continue;
            }
            let Some(next) = direction.step(cell, snapshot.board) else {
                continue;
            };
            if blocked.contains(&next) || !visited.insert(next) {
                continue;
            }
            previous.insert(next, (cell, direction));
            queue.push_back(next);
        }
    }
    [
        snapshot.direction,
        Direction::Up,
        Direction::Right,
        Direction::Down,
        Direction::Left,
    ]
    .into_iter()
    .filter(|direction| *direction != snapshot.direction.opposite())
    .find(|direction| {
        direction
            .step(start, snapshot.board)
            .is_some_and(|next| !blocked.contains(&next))
    })
}
fn collect_food(
    app: &mut SnakeApp,
    scheduler: &mut Scheduler,
    score: u32,
    now: &mut Duration,
) -> Result<(), Box<dyn Error>> {
    for _ in 0..2000 {
        let snapshot = app.snapshot();
        if snapshot.score >= score {
            return Ok(());
        }
        require(
            snapshot.phase == Phase::Running,
            "replay stopped before collecting the requested food",
        )?;
        let direction = route(&snapshot).ok_or("replay could not find a safe turn")?;
        let deadline = app
            .next_deadline()
            .ok_or("running game has no tick deadline")?;
        key(
            app,
            scheduler,
            direction_key(direction),
            deadline.saturating_sub(Duration::from_millis(1)),
        )?;
        *now = deadline;
        frame(app, scheduler, *now)?;
    }
    Err("food replay exceeded its deterministic step budget".into())
}
fn finish_run(
    app: &mut SnakeApp,
    scheduler: &mut Scheduler,
    now: &mut Duration,
) -> Result<(), Box<dyn Error>> {
    for _ in 0..128 {
        if matches!(app.snapshot().phase, Phase::Over(_) | Phase::Won) {
            return Ok(());
        }
        *now = app
            .next_deadline()
            .ok_or("running game lost its deadline")?;
        frame(app, scheduler, *now)?;
    }
    Err("straight-line replay failed to reach a board boundary".into())
}

/// Runs reproducible real-UI inputs and writes ready/playing/paused/end/resize images.
pub fn run(config: Config, output: &Path) -> Result<Report, Box<dyn Error>> {
    run_with_screen(config, output, screen)
}

/// Drives the same replay against an alternate presentation of the game.
pub fn run_with_screen(
    config: Config,
    output: &Path,
    screen: Screen,
) -> Result<Report, Box<dyn Error>> {
    std::fs::create_dir_all(output)?;
    let manifest = output.join("report.json");
    std::fs::write(
        &manifest,
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed": false, "status": "running", "seed": config.seed
        }))?,
    )?;
    match run_checks(config, output, screen) {
        Ok(report) => {
            std::fs::write(&manifest, serde_json::to_vec_pretty(&report)?)?;
            Ok(report)
        }
        Err(error) => {
            // Preserve the original error if the output directory also becomes unwritable.
            if let Ok(failure) = serde_json::to_vec_pretty(&serde_json::json!({
                "passed": false, "status": "failed", "seed": config.seed, "error": error.to_string()
            })) {
                let _ = std::fs::write(&manifest, failure);
            }
            Err(error)
        }
    }
}

fn run_checks(config: Config, output: &Path, screen: Screen) -> Result<Report, Box<dyn Error>> {
    let counters = Arc::new(Counters::default());
    let mut app = SnakeApp::with_screen(config, Arc::clone(&counters), screen)?;
    let mut scheduler = Scheduler::default();
    let mut checks = Vec::new();
    let mut images = Vec::new();
    require(
        app.snapshot().phase == Phase::Ready && app.next_deadline().is_none(),
        "initial game is not idle and ready",
    )?;
    let ready = draw(
        &mut app,
        output,
        "ready.png",
        config.width,
        config.height,
        1,
    )?;
    let ready_hidpi = draw(
        &mut app,
        output,
        "ready@2x.png",
        config.width,
        config.height,
        2,
    )?;
    if ready.glyph_images + ready.vector_glyphs
        != ready_hidpi.glyph_images + ready_hidpi.vector_glyphs
        || ready_hidpi.vector_glyphs < ready.vector_glyphs
        || ready_hidpi.glyph_pixels <= ready.glyph_pixels * 2
    {
        return Err(format!(
            "DPI mismatch: 1x has {} raster / {} vector glyphs / {} raster pixels, 2x has {} raster / {} vector glyphs / {} raster pixels",
            ready.glyph_images,
            ready.vector_glyphs,
            ready.glyph_pixels,
            ready_hidpi.glyph_images,
            ready_hidpi.vector_glyphs,
            ready_hidpi.glyph_pixels
        )
        .into());
    }
    images.extend([ready, ready_hidpi]);
    checks.push("real-font shaping and DPI-aware raster and vector glyph selection");
    let mut now = Duration::from_millis(1);
    click(
        &mut app,
        &mut scheduler,
        Action::Difficulty(Difficulty::Chill),
        &mut now,
    )?;
    require(
        app.snapshot().difficulty == Difficulty::Chill,
        "mouse pace selection failed",
    )?;
    click(
        &mut app,
        &mut scheduler,
        Action::Difficulty(Difficulty::Classic),
        &mut now,
    )?;
    click(&mut app, &mut scheduler, Action::Primary, &mut now)?;
    require(
        app.snapshot().phase == Phase::Running && app.next_deadline().is_some(),
        "mouse start did not start the game clock",
    )?;
    click(
        &mut app,
        &mut scheduler,
        Action::Difficulty(Difficulty::Swift),
        &mut now,
    )?;
    require(
        app.snapshot().difficulty == Difficulty::Classic,
        "disabled pace control changed a running game",
    )?;
    checks.push("native-style pointer buttons, conditional overlay, and disabled controls");

    key(
        &mut app,
        &mut scheduler,
        KeyCode::ArrowLeft,
        now.saturating_add(Duration::from_millis(1)),
    )?;
    require(
        app.snapshot().direction == Direction::Right,
        "opposite-direction key reversed the snake",
    )?;
    collect_food(&mut app, &mut scheduler, 30, &mut now)?;
    require(
        app.snapshot().body.len() == 7 && app.snapshot().best >= 30,
        "food did not update real game state",
    )?;
    images.push(draw(
        &mut app,
        output,
        "playing@2x.png",
        config.width,
        config.height,
        2,
    )?);
    checks.push("keyboard turns, deterministic food collection, growth, and reactive score");

    now = now.saturating_add(Duration::from_millis(1));
    key(&mut app, &mut scheduler, KeyCode::Space, now)?;
    require(
        app.snapshot().phase == Phase::Paused && app.next_deadline().is_none(),
        "pause retained a periodic game deadline",
    )?;
    images.push(draw(
        &mut app,
        output,
        "paused.png",
        config.width,
        config.height,
        1,
    )?);
    let paused = app.snapshot();
    let computations = counters.board_updates.load(Ordering::SeqCst);
    for _ in 0..1000 {
        now = now.saturating_add(Duration::from_millis(16));
        frame(&mut app, &mut scheduler, now)?;
    }
    require(
        app.snapshot() == paused && counters.board_updates.load(Ordering::SeqCst) == computations,
        "paused virtual time changed the model or its board binding",
    )?;
    checks.push("paused game remains unchanged through 1000 virtual frame opportunities");
    now = now.saturating_add(Duration::from_millis(1));
    key(&mut app, &mut scheduler, KeyCode::Space, now)?;
    require(
        app.snapshot().phase == Phase::Running
            && app.next_deadline().is_some_and(|deadline| deadline > now),
        "resume did not schedule a fresh future tick",
    )?;
    let before = app.snapshot().body;
    let far_future = now.saturating_add(Duration::from_secs(20));
    frame(&mut app, &mut scheduler, far_future)?;
    now = far_future;
    require(
        app.snapshot().phase == Phase::Paused && app.snapshot().body == before,
        "display interruption fast-forwarded the game",
    )?;
    checks.push("large display-clock gaps pause instead of fast-forwarding");

    now = now.saturating_add(Duration::from_millis(1));
    key(&mut app, &mut scheduler, KeyCode::Space, now)?;
    now = now.saturating_add(Duration::from_millis(1));
    app.process(
        &WindowEvent::ActivationChanged {
            active: false,
            timestamp: now,
        },
        &mut scheduler,
    )?;
    require(
        app.snapshot().phase == Phase::Paused && app.next_deadline().is_none(),
        "activation loss did not pause the game",
    )?;
    now = now.saturating_add(Duration::from_secs(5));
    app.process(
        &WindowEvent::ActivationChanged {
            active: true,
            timestamp: now,
        },
        &mut scheduler,
    )?;
    require(
        app.snapshot().phase == Phase::Paused,
        "activation restoration resumed without player intent",
    )?;
    key(
        &mut app,
        &mut scheduler,
        KeyCode::Space,
        now.saturating_add(Duration::from_millis(1)),
    )?;
    checks.push("activation loss cancels ticks and restoration waits for explicit resume");
    finish_run(&mut app, &mut scheduler, &mut now)?;
    require(
        app.next_deadline().is_none(),
        "game-over retained a tick deadline",
    )?;
    let first_history = app.history_ids()?;
    require(
        first_history.len() == 1,
        "finished run did not create a keyed history row",
    )?;
    images.push(draw(
        &mut app,
        output,
        "game-over.png",
        config.width,
        config.height,
        1,
    )?);
    images.push(draw(&mut app, output, "compact.png", 900, 760, 1)?);
    checks.push("collision, game-over overlay, keyed run history, and responsive text layout");

    let best = app.snapshot().best;
    for run in 0..3 {
        now = now.saturating_add(Duration::from_millis(1));
        key(&mut app, &mut scheduler, KeyCode::Enter, now)?;
        require(
            app.snapshot().score == 0
                && app.snapshot().body.len() == 4
                && app.snapshot().best == best,
            "new game failed to reset score/body or preserve session best",
        )?;
        finish_run(&mut app, &mut scheduler, &mut now)?;
        let rows = app.history_ids()?;
        if run == 0 {
            require(
                rows.get(1) == first_history.first(),
                "keyed history remounted a retained row",
            )?;
        }
    }
    require(
        app.history_ids()?.len() == 3,
        "recent history was not bounded to three rows",
    )?;
    require(
        counters.screen_mounts.load(Ordering::SeqCst) == 1,
        "state updates re-executed the screen component",
    )?;
    checks.push("restart preserves best; keyed rows retain identity; screen mounts exactly once");
    let rejected = counters.rejected_commands.load(Ordering::SeqCst);
    require(
        rejected == 0,
        "ordinary game interactions produced rejected UI mutations",
    )?;
    let report = Report {
        passed: true,
        seed: config.seed,
        debug_overlay: config.debug_overlay,
        checks,
        images,
        screen_mounts: counters.screen_mounts.load(Ordering::SeqCst),
        overlay_mounts: counters.overlay_mounts.load(Ordering::SeqCst),
        history_mounts: counters.history_mounts.load(Ordering::SeqCst),
        board_updates: counters.board_updates.load(Ordering::SeqCst),
        rejected_commands: rejected,
        canceled_bindings: counters.canceled_bindings.load(Ordering::SeqCst),
        best_score: best,
    };
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mouse_start_and_real_gameplay_update_bound_score_without_remounting()
    -> Result<(), Box<dyn Error>> {
        let counters = Arc::new(Counters::default());
        let mut app = SnakeApp::new(Config::default(), Arc::clone(&counters))?;
        let mut scheduler = Scheduler::default();
        let mut now = Duration::from_millis(1);
        click(&mut app, &mut scheduler, Action::Primary, &mut now)?;
        assert_eq!(app.snapshot().phase, Phase::Running);
        collect_food(&mut app, &mut scheduler, 30, &mut now)?;
        assert_eq!(app.snapshot().score, 30);
        assert_eq!(app.snapshot().body.len(), 7);
        assert_eq!(counters.screen_mounts.load(Ordering::SeqCst), 1);
        assert_eq!(counters.rejected_commands.load(Ordering::SeqCst), 0);
        Ok(())
    }
    #[test]
    fn ready_screen_is_real_text_and_does_not_start_periodic_work() -> Result<(), Box<dyn Error>> {
        let counters = Arc::new(Counters::default());
        let mut app = SnakeApp::new(Config::default(), Arc::clone(&counters))?;
        let scene = app.render(
            PhysicalSize {
                width: 1120,
                height: 820,
            },
            1.0,
        )?;
        let encoded = scene.scene.encode(scene.root)?;
        assert!(
            encoded
                .nodes
                .iter()
                .filter(|node| matches!(
                    node,
                    EncodedNode::Draw {
                        primitive: Primitive::Image(_, _),
                        ..
                    }
                ))
                .count()
                > 80
        );
        assert_eq!(app.snapshot().phase, Phase::Ready);
        assert!(app.next_deadline().is_none());
        assert_eq!(counters.screen_mounts.load(Ordering::SeqCst), 1);
        Ok(())
    }
}
