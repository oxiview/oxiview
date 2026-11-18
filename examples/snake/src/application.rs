//! A real native game UI built with Signals, function components, `view!`, and font text.

use crate::{
    board,
    debug::{DebugOverlay, SceneInfo},
    game::{Board, Collision, Difficulty, Direction, Game, Phase, SeededRng, Snapshot, TickClock},
    presentation::{
        BOARD, BORDER, INK, MUTED, PAPER, RunSummary, ScreenProps, Slots, Ui, WHITE, blank,
        button_style, centered_label, color, column, label, phase_caption, primary_caption, px,
        row, surface,
    },
};
use oxiview_base::{Color, Rect, Size};
use oxiview_core::{
    Application, FrameSubmission, FrameTiming, LatencyRecord, NodeId, PaintedScene, Scheduler,
    UiBatchReport, UiError, UiTree, View, component, keyed_for, show,
};
use oxiview_macros::{Props, view};
use oxiview_platform::{KeyCode, PhysicalSize, WindowAttributes, WindowEvent, WindowWaker};
use oxiview_reactive::{Memo, Scope, Signal, batch, memo, signal};
use oxiview_style::{Alignment, ContentAlignment, Edges, LayoutStyle, Length, Overflow, Position};
use oxiview_text::TextSystem;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

/// Default logical width of the standalone game window.
pub const WIDTH: u32 = 1120;
/// Default logical height of the standalone game window.
pub const HEIGHT: u32 = 820;

fn app_error(error: impl std::fmt::Display) -> oxiview_core::Error {
    oxiview_core::Error::Application(error.to_string())
}

/// Reproducible application configuration.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Random seed controlling food placement.
    pub seed: u64,
    /// Initial logical viewport width.
    pub width: u32,
    /// Initial logical viewport height.
    pub height: u32,
    /// Draw live diagnostics above the UI; disabled by default.
    pub debug_overlay: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            seed: 0x5353_4e41_4b45_0001,
            width: WIDTH,
            height: HEIGHT,
            debug_overlay: false,
        }
    }
}

/// Plain application actions shared by mouse and keyboard input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Start, pause, resume, or replay depending on the current game phase.
    Primary,
    /// Start a new run, preserving this session's best score.
    NewGame,
    /// Pause an active run without toggling it back on.
    Pause,
    /// Request a cardinal turn.
    Turn(Direction),
    /// Select an idle game's initial cadence.
    Difficulty(Difficulty),
}

/// Window-independent counters for integration checks; never shown as game UI.
#[derive(Default)]
pub struct Counters {
    /// Number of completed presentations.
    pub presented: AtomicU64,
    /// Number of times the persistent screen component was mounted.
    pub screen_mounts: AtomicU64,
    /// Number of overlay branch mounts.
    pub overlay_mounts: AtomicU64,
    /// Number of newly keyed history row mounts.
    pub history_mounts: AtomicU64,
    /// Number of reactive board-content computations.
    pub board_updates: AtomicU64,
    /// Rejected deferred mutations, retained for diagnostics.
    pub rejected_commands: AtomicU64,
    /// Automatic jobs retired with disposed view scopes, distinct from rejections.
    pub canceled_bindings: AtomicU64,
}

#[derive(Props)]
struct ButtonProps {
    ui: Ui,
    action: Action,
    caption: &'static str,
    #[prop(default)]
    primary: bool,
    #[prop(default)]
    compact: bool,
    #[prop(default)]
    overlay: bool,
}
fn action_button(props: ButtonProps) -> View {
    component(move || {
        let pressed = signal(false).map_err(|error| UiError::Reactive(error.to_string()))?;
        Ok(button_content(props, pressed))
    })
}
fn button_content(props: ButtonProps, pressed: Signal<bool>) -> View {
    let ButtonProps {
        ui,
        action,
        caption,
        primary,
        compact,
        overlay,
    } = props;
    let phase = ui.phase;
    let difficulty = ui.difficulty;
    let title = if action == Action::Primary {
        label(&ui, "Start game", 15.0, 700, INK)
            .bind_text(move || Ok(primary_caption(phase.get()?).into()))
    } else {
        label(
            &ui,
            caption,
            if matches!(action, Action::Turn(_)) {
                22.0
            } else if compact {
                13.0
            } else {
                14.0
            },
            650,
            INK,
        )
    };
    let slot = if overlay {
        Rc::clone(&ui.slots.overlay_primary)
    } else {
        ui.slots.button(action)
    };
    let pointer = ui.button_pointer(action, pressed);
    view! {
        <Node
            bind_style={move || {
                let mut style = button_style(action, phase.get()?, difficulty.get()?, primary, compact);
                if pressed.get()? { style.opacity *= 0.78; }
                Ok(style)
            }}
            on_mount={move |id| slot.set(Some(id))}
            on_pointer={pointer}
        > {title} </Node>
    }
}

#[derive(Props)]
struct ScoreProps {
    ui: Ui,
    caption: &'static str,
    value: Memo<u32>,
    #[prop(default)]
    accent: bool,
}
fn score_card(props: ScoreProps) -> View {
    let ScoreProps {
        ui,
        caption,
        value,
        accent,
    } = props;
    let mut style = surface(
        LayoutStyle {
            height: px(100.0),
            flex_grow: 1.0,
            flex_basis: px(0.0),
            padding: Edges::all(px(15.0)),
            row_gap: px(6.0),
            border_width: 1.0,
            ..column()
        },
        if accent { [226, 237, 210] } else { WHITE },
        16.0,
    );
    style.border_color = color(if accent { [205, 222, 184] } else { BORDER });
    let caption = label(&ui, caption, 11.0, 650, MUTED);
    let number = label(&ui, "0", 36.0, 800, INK).bind_text(move || Ok(value.get()?.to_string()));
    view! { <Node style={style}> {caption} {number} </Node> }
}

fn overlay(ui: Ui) -> View {
    ui.counters.overlay_mounts.fetch_add(1, Ordering::SeqCst);
    let phase = ui.phase;
    let snapshot = ui.state;
    let mut background = surface(
        LayoutStyle {
            width: Length::Percent(1.0),
            height: Length::Percent(1.0),
            justify_content: ContentAlignment::Center,
            align_items: Alignment::Center,
            padding: Edges::all(px(28.0)),
            row_gap: px(18.0),
            ..column()
        },
        BOARD,
        22.0,
    );
    background.background = Color::from_rgba8(13, 28, 22, 225);
    background.hit_test = true;
    let title = centered_label(&ui, "Ready when you are.", 31.0, 800, [239, 246, 226]).bind_text(
        move || {
            Ok(match phase.get()? {
                Phase::Ready => "Ready when you are.",
                Phase::Paused => "Take a breath.",
                Phase::Over(_) => "One more round?",
                Phase::Won => "You cleared the board!",
                Phase::Running => "",
            }
            .into())
        },
    );
    let subtitle = centered_label(
        &ui,
        "Collect the coral food.\nLeave yourself room to turn.",
        15.0,
        400,
        [174, 196, 177],
    )
    .bind_text(move || {
        let game = snapshot.get()?;
        Ok(match game.phase {
            Phase::Ready => "Collect the coral food.\nLeave yourself room to turn.".into(),
            Phase::Paused => "Your next move can wait.\nPress Space or Resume to continue.".into(),
            Phase::Over(Collision::Wall) => format!(
                "The edge wins this round.\nYou collected {} points.",
                game.score
            ),
            Phase::Over(Collision::Body) => format!(
                "You crossed your own path.\nYou collected {} points.",
                game.score
            ),
            Phase::Won => format!(
                "Every cell is yours.\n{} points, beautifully played.",
                game.score
            ),
            Phase::Running => String::new(),
        })
    });
    let button = ButtonProps {
        ui,
        action: Action::Primary,
        caption: "Start game",
        primary: true,
        compact: false,
        overlay: true,
    };
    view! {
        <Node style={background}>
            <Node style={blank(LayoutStyle { width: Length::Percent(1.0), max_width: px(420.0), row_gap: px(14.0), align_items: Alignment::Center, ..column() })}>
                {title} {subtitle}
                <Node style={blank(LayoutStyle { width: px(184.0), height: px(46.0), margin: Edges { top: px(8.0), ..Edges::all(px(0.0)) }, ..row() })}>
                    <action_button props={button}/>
                </Node>
            </Node>
        </Node>
    }
}

fn game_panel(ui: Ui) -> View {
    let game = ui.state;
    let board_size = ui.board_size;
    let counters = Arc::clone(&ui.counters);
    let slot = Rc::clone(&ui.slots.board);
    let overlay_ui = ui.clone();
    let mut overlay_style = blank(LayoutStyle {
        position: Position::Absolute,
        width: Length::Percent(1.0),
        height: Length::Percent(1.0),
        inset: Edges {
            left: px(0.0),
            top: px(0.0),
            ..Edges::all(Length::Auto)
        },
        ..column()
    });
    overlay_style.z_index = Some(10);
    let conditional = show(ui.overlay_visible, move || {
        let props = overlay_ui.clone();
        Ok(view! { <overlay props={props}/> })
    })
    .style(overlay_style);
    let mut style = surface(
        LayoutStyle {
            position: Position::Relative,
            flex_grow: 1.0,
            flex_basis: px(0.0),
            min_width: px(240.0),
            border_width: 1.0,
            ..column()
        },
        BOARD,
        24.0,
    );
    style.border_color = color([38, 61, 46]);
    style.overflow = Overflow::Clip;
    style.z_index = Some(0);
    view! {
        <Node style={style} on_mount={move |id| slot.set(Some(id))}
            bind_content={move || {
                counters.board_updates.fetch_add(1, Ordering::SeqCst);
                Ok(board::draw(&game.get()?, board_size.get()?))
            }}>
            {conditional}
        </Node>
    }
}

fn history_row(ui: Ui, run: Signal<RunSummary>) -> View {
    ui.counters.history_mounts.fetch_add(1, Ordering::SeqCst);
    let title = label(&ui, "Run", 13.0, 500, MUTED)
        .bind_text(move || {
            let item = run.get()?;
            Ok(format!(
                "Run {:02}  ·  {}",
                item.id,
                if item.won {
                    "Cleared".into()
                } else {
                    format!("Level {}", item.level)
                }
            ))
        })
        .style(blank(LayoutStyle {
            flex_grow: 1.0,
            min_width: px(0.0),
            ..Default::default()
        }));
    let score = label(&ui, "0", 14.0, 700, INK).bind_text(move || Ok(run.get()?.score.to_string()));
    view! { <Node style={blank(LayoutStyle { height: px(23.0), flex_shrink: 0.0, column_gap: px(8.0), ..row() })}> {title} {score} </Node> }
}

fn sidebar(ui: Ui) -> View {
    let score = ScoreProps {
        ui: ui.clone(),
        caption: "SCORE",
        value: ui.score,
        accent: true,
    };
    let best = ScoreProps {
        ui: ui.clone(),
        caption: "SESSION BEST",
        value: ui.best,
        accent: false,
    };
    let primary = ButtonProps {
        ui: ui.clone(),
        action: Action::Primary,
        caption: "Start game",
        primary: true,
        compact: false,
        overlay: false,
    };
    let restart = ButtonProps {
        ui: ui.clone(),
        action: Action::NewGame,
        caption: "New game",
        primary: false,
        compact: false,
        overlay: false,
    };
    let pace_heading = label(&ui, "PACE", 11.0, 700, MUTED);
    let choices: Vec<_> = [Difficulty::Chill, Difficulty::Classic, Difficulty::Swift]
        .into_iter()
        .map(|difficulty| {
            let props = ButtonProps {
                ui: ui.clone(),
                action: Action::Difficulty(difficulty),
                caption: difficulty.name(),
                primary: false,
                compact: true,
                overlay: false,
            };
            view! { <action_button props={props}/> }
        })
        .collect();
    let make_direction = |direction, caption| {
        let props = ButtonProps {
            ui: ui.clone(),
            action: Action::Turn(direction),
            caption,
            primary: false,
            compact: true,
            overlay: false,
        };
        view! { <action_button props={props}/> }
    };
    let up = make_direction(Direction::Up, "↑");
    let left = make_direction(Direction::Left, "←");
    let down = make_direction(Direction::Down, "↓");
    let right = make_direction(Direction::Right, "→");
    let instructions = centered_label(&ui, "ARROWS OR WASD", 10.5, 650, MUTED);
    let history_title = label(&ui, "RECENT RUNS", 11.0, 700, MUTED);
    let empty_ui = ui.clone();
    let empty_history = show(ui.history_empty, move || {
        Ok(label(
            &empty_ui,
            "Your completed runs\nwill appear here.",
            13.0,
            400,
            MUTED,
        ))
    })
    .style(blank(column()));
    let history_ui = ui.clone();
    let history_slot = Rc::clone(&ui.slots.history);
    let history = keyed_for(
        ui.history,
        |run| run.id,
        move |run| Ok(history_row(history_ui.clone(), run)),
    )
    .style(blank(LayoutStyle {
        row_gap: px(5.0),
        ..column()
    }))
    .on_mount(move |id| history_slot.set(Some(id)));
    view! {
        <Node style={blank(LayoutStyle { width: px(256.0), flex_shrink: 0.0, row_gap: px(12.0), ..column() })}>
            <Node style={blank(LayoutStyle { height: px(100.0), flex_shrink: 0.0, column_gap: px(10.0), ..row() })}>
                <score_card props={score}/><score_card props={best}/>
            </Node>
            <action_button props={primary}/>
            <action_button props={restart}/>
            <Node style={blank(LayoutStyle { row_gap: px(8.0), flex_shrink: 0.0, margin: Edges { top: px(4.0), ..Edges::all(px(0.0)) }, ..column() })}>
                {pace_heading}
                <Node style={blank(LayoutStyle { height: px(32.0), column_gap: px(6.0), ..row() })} children={choices}/>
            </Node>
            <Node style={blank(LayoutStyle { row_gap: px(5.0), align_items: Alignment::Center, flex_shrink: 0.0, ..column() })}>
                <Node style={blank(LayoutStyle { justify_content: ContentAlignment::Center, ..row() })}>{up}</Node>
                <Node style={blank(LayoutStyle { column_gap: px(6.0), justify_content: ContentAlignment::Center, ..row() })}>{left}{down}{right}</Node>
                {instructions}
            </Node>
            <Node style={blank(LayoutStyle { flex_grow: 1.0, min_height: px(90.0), row_gap: px(9.0), padding: Edges { top: px(7.0), ..Edges::all(px(0.0)) }, ..column() })}>
                {history_title}{empty_history}{history}
            </Node>
        </Node>
    }
}

/// Builds the original Rust and inline-view Snake screen.
pub fn screen(props: ScreenProps) -> View {
    let ui = props.ui;
    ui.counters.screen_mounts.fetch_add(1, Ordering::SeqCst);
    let title = label(&ui, "Snake", 38.0, 800, INK);
    let subtitle = label(&ui, "Collect food. Keep moving.", 14.0, 400, MUTED);
    let phase = ui.phase;
    let badge_label = centered_label(&ui, "Ready", 13.0, 650, INK)
        .bind_text(move || Ok(phase_caption(phase.get()?).into()));
    let game_props = ui.clone();
    let sidebar_props = ui.clone();
    let state = ui.state;
    let notice = ui.notice;
    let note = label(&ui, "Press Enter or choose Start game.", 12.0, 400, MUTED)
        .bind_text(move || {
            let game = state.get()?;
            let note = notice.get()?;
            Ok(if note.is_empty() {
                format!("Level {}  ·  {} pace", game.level, game.difficulty.name())
            } else {
                note
            })
        })
        .style(blank(LayoutStyle {
            flex_grow: 1.0,
            min_width: px(0.0),
            ..Default::default()
        }));
    let keys = label(&ui, "SPACE  pause   ·   ENTER  new game", 12.0, 500, MUTED).style(blank(
        LayoutStyle {
            flex_shrink: 0.0,
            ..Default::default()
        },
    ));
    view! {
        <Node style={surface(LayoutStyle { width: Length::Percent(1.0), height: Length::Percent(1.0), padding: Edges::all(px(28.0)), row_gap: px(20.0), ..column() }, PAPER, 0.0)}>
            <Node style={blank(LayoutStyle { height: px(64.0), flex_shrink: 0.0, column_gap: px(20.0), ..row() })}>
                <Node style={blank(LayoutStyle { flex_grow: 1.0, row_gap: px(1.0), ..column() })}>{title}{subtitle}</Node>
                <Node bind_style={move || {
                    let phase = phase.get()?;
                    let tone = match phase { Phase::Running | Phase::Won => [226,237,210], Phase::Over(_) => [249,222,209], _ => [232,235,225] };
                    Ok(surface(LayoutStyle { width: px(124.0), height: px(32.0), justify_content: ContentAlignment::Center, align_items: Alignment::Center, flex_shrink: 0.0, ..row() }, tone, 16.0))
                }}>{badge_label}</Node>
            </Node>
            <Node style={blank(LayoutStyle { flex_grow: 1.0, min_height: px(400.0), column_gap: px(20.0), align_items: Alignment::Stretch, ..row() })}>
                <game_panel props={game_props}/><sidebar props={sidebar_props}/>
            </Node>
            <Node style={blank(LayoutStyle { height: px(22.0), flex_shrink: 0.0, column_gap: px(12.0), ..row() })}>{note}{keys}</Node>
        </Node>
    }
}

/// A Snake screen factory, mounted once per application.
pub type Screen = fn(ScreenProps) -> View;

/// The native/headless application; retained nodes are mounted once.
pub struct SnakeApp {
    // Dispose view-owned effects before the external game-data scope.
    tree: UiTree,
    _scope: Scope,
    ui: Ui,
    model: Game<SeededRng>,
    clock: TickClock,
    actions: Receiver<Action>,
    errors: Receiver<String>,
    viewport: Size,
    now: Duration,
    run_id: u64,
    recorded_run: Option<u64>,
    history: Vec<RunSummary>,
    closed: bool,
    debug: Option<DebugOverlay>,
}
impl SnakeApp {
    /// Creates the complete game UI without opening a native window.
    pub fn new(config: Config, counters: Arc<Counters>) -> Result<Self, oxiview_core::Error> {
        Self::with_screen(config, counters, screen)
    }

    /// Mounts the supplied screen over the same game model, input and rendering paths.
    pub fn with_screen(
        config: Config,
        counters: Arc<Counters>,
        screen: Screen,
    ) -> Result<Self, oxiview_core::Error> {
        let model = Game::new(
            Board::default(),
            Difficulty::Classic,
            SeededRng::new(config.seed),
        )
        .map_err(app_error)?;
        let scope = Scope::new().map_err(app_error)?;
        let initial = model.snapshot();
        let (
            state,
            phase,
            difficulty,
            score,
            best,
            board_size,
            overlay_visible,
            history,
            history_empty,
            notice,
        ) = scope
            .run(|| {
                let state = signal(initial)?;
                let phase = memo(move || Ok(state.get()?.phase))?;
                let difficulty = memo(move || Ok(state.get()?.difficulty))?;
                let score = memo(move || Ok(state.get()?.score))?;
                let best = memo(move || Ok(state.get()?.best))?;
                Ok((
                    state,
                    phase,
                    difficulty,
                    score,
                    best,
                    signal(Size::new(0.0, 0.0))?,
                    signal(true)?,
                    signal(Vec::<RunSummary>::new())?,
                    signal(true)?,
                    signal("Press Enter or choose Start game.".to_string())?,
                ))
            })
            .map_err(app_error)?;
        let (sender, receiver) = mpsc::channel();
        let (error_sender, errors) = mpsc::channel();
        let ui = Ui {
            text: Rc::new(RefCell::new(TextSystem::new())),
            state,
            phase,
            difficulty,
            score,
            best,
            board_size,
            overlay_visible,
            history,
            history_empty,
            notice,
            slots: Rc::new(Slots::default()),
            actions: sender,
            errors: error_sender,
            counters,
        };
        let viewport = Size::new(f64::from(config.width), f64::from(config.height));
        let mut tree = UiTree::new()?;
        tree.set_style(
            tree.root(),
            blank(LayoutStyle {
                width: Length::Percent(1.0),
                height: Length::Percent(1.0),
                ..column()
            }),
        )?;
        tree.layout(viewport)?;
        let props = ScreenProps::builder()
            .ui(ui.clone())
            .build()
            .map_err(app_error)?;
        let root = tree.root();
        tree.mount(root, view! { <screen props={props}/> })?;
        let mut app = Self {
            tree,
            _scope: scope,
            ui,
            model,
            clock: TickClock::default(),
            actions: receiver,
            errors,
            viewport,
            now: Duration::ZERO,
            run_id: 1,
            recorded_run: None,
            history: Vec::new(),
            closed: false,
            debug: config.debug_overlay.then(DebugOverlay::new),
        };
        app.sync_ui()?;
        Ok(app)
    }
    /// Returns immutable game state for replay/acceptance checks.
    pub fn snapshot(&self) -> Snapshot {
        self.model.snapshot()
    }
    /// Returns the board canvas' committed bounds.
    pub fn board_bounds(&self) -> Result<Rect, oxiview_core::Error> {
        let id = self
            .ui
            .slots
            .board
            .get()
            .ok_or_else(|| app_error("board was not mounted"))?;
        Ok(self.tree.bounds(id)?)
    }
    /// Locates the currently visible control for a deterministic pointer replay.
    pub fn control_bounds(&self, action: Action) -> Result<Rect, oxiview_core::Error> {
        let overlay = if action == Action::Primary {
            self.ui
                .slots
                .overlay_primary
                .get()
                .filter(|id| self.tree.contains(*id))
        } else {
            None
        };
        let id = overlay
            .or_else(|| self.ui.slots.button(action).get())
            .ok_or_else(|| app_error("control was not mounted"))?;
        Ok(self.tree.bounds(id)?)
    }
    /// Returns the next game tick deadline; idle states return none.
    pub fn next_deadline(&self) -> Option<Duration> {
        self.clock.next_deadline()
    }
    /// Returns the debug layer's one-shot idle refresh deadline, if enabled.
    pub fn debug_overlay_deadline(&self) -> Option<Duration> {
        self.debug.as_ref().and_then(DebugOverlay::deadline)
    }
    /// Returns app submissions in the last second, excluding HUD-only refreshes.
    pub fn debug_overlay_fps(&self) -> Option<f64> {
        self.debug.as_ref().map(DebugOverlay::fps)
    }
    /// Returns current logical viewport dimensions.
    pub fn viewport(&self) -> Size {
        self.viewport
    }
    /// Returns shared diagnostic counters.
    pub fn counters(&self) -> &Arc<Counters> {
        &self.ui.counters
    }
    /// Returns preserved identities for visible keyed history rows.
    pub fn history_ids(&self) -> Result<Vec<NodeId>, oxiview_core::Error> {
        let id = self
            .ui
            .slots
            .history
            .get()
            .ok_or_else(|| app_error("history was not mounted"))?;
        Ok(self.tree.children(id)?.to_vec())
    }
    /// Processes a real platform event without requiring a native display.
    pub fn process(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
    ) -> Result<(), oxiview_core::Error> {
        if self.closed {
            return Ok(());
        }
        let event_started = self.debug.as_ref().map(|_| Instant::now());
        if let Some(debug) = &mut self.debug {
            match event {
                WindowEvent::ActivationChanged { active: false, .. }
                | WindowEvent::VisibilityChanged { visible: false, .. }
                | WindowEvent::PointerInterrupted { .. } => debug.suspend(),
                WindowEvent::Resized { physical_size, .. }
                    if physical_size.width == 0 || physical_size.height == 0 =>
                {
                    debug.suspend()
                }
                WindowEvent::ActivationChanged { active: true, .. }
                | WindowEvent::VisibilityChanged { visible: true, .. }
                | WindowEvent::PointerMoved { .. }
                | WindowEvent::PointerButton { .. }
                | WindowEvent::Key { .. } => debug.resume(),
                _ => {}
            }
        }
        let timestamp = match event {
            WindowEvent::Frame {
                predicted_presentation_timestamp,
                ..
            }
            | WindowEvent::Resized {
                predicted_presentation_timestamp,
                ..
            } => Some(*predicted_presentation_timestamp),
            WindowEvent::Key { timestamp, .. }
            | WindowEvent::PointerMoved { timestamp, .. }
            | WindowEvent::PointerButton { timestamp, .. }
            | WindowEvent::PointerLeft { timestamp }
            | WindowEvent::ActivationChanged { timestamp, .. }
            | WindowEvent::VisibilityChanged { timestamp, .. }
            | WindowEvent::PointerInterrupted { timestamp } => Some(*timestamp),
            _ => None,
        };
        if let Some(timestamp) = timestamp {
            self.now = self.now.max(timestamp);
        }
        if let WindowEvent::Resized {
            physical_size,
            scale_factor,
            ..
        } = event
        {
            if physical_size.width > 0 && physical_size.height > 0 {
                self.viewport = Size::new(
                    f64::from(physical_size.width) / scale_factor,
                    f64::from(physical_size.height) / scale_factor,
                );
            } else {
                self.pause_for_interruption("Paused while the window is away.")?;
            }
        }
        if matches!(
            event,
            WindowEvent::ActivationChanged { active: false, .. }
                | WindowEvent::VisibilityChanged { visible: false, .. }
                | WindowEvent::PointerInterrupted { .. }
        ) {
            self.pause_for_interruption("Paused while the window is away.")?;
        }
        let dispatch = self.tree.dispatch(event, self.viewport, |_, _, _| {})?;
        self.observe_reports(dispatch.reports);
        if matches!(event, WindowEvent::Destroyed) {
            self.model.pause();
            self.clock.cancel();
            scheduler.cancel_timed_request();
            self.closed = true;
            if let Some(debug) = &mut self.debug {
                debug.close();
            }
            return Ok(());
        }
        if let Ok(error) = self.errors.try_recv() {
            return Err(app_error(error));
        }
        if let WindowEvent::Key {
            code,
            pressed: true,
            repeat: false,
            ..
        } = event
        {
            let action = match code {
                KeyCode::ArrowUp | KeyCode::KeyW => Some(Action::Turn(Direction::Up)),
                KeyCode::ArrowRight | KeyCode::KeyD => Some(Action::Turn(Direction::Right)),
                KeyCode::ArrowDown | KeyCode::KeyS => Some(Action::Turn(Direction::Down)),
                KeyCode::ArrowLeft | KeyCode::KeyA => Some(Action::Turn(Direction::Left)),
                KeyCode::Space => Some(Action::Primary),
                KeyCode::Enter => Some(if self.model.phase() == Phase::Ready {
                    Action::Primary
                } else {
                    Action::NewGame
                }),
                KeyCode::Escape => Some(Action::Pause),
                _ => None,
            };
            if let Some(action) = action {
                self.apply_action(action)?;
            }
        }
        let actions: Vec<_> = self.actions.try_iter().collect();
        for action in actions {
            self.apply_action(action)?;
        }
        if matches!(event, WindowEvent::Frame { .. }) {
            let advance = self
                .clock
                .advance(self.now, &mut self.model)
                .map_err(app_error)?;
            if advance.paused_for_delay {
                self.ui
                    .notice
                    .set("Paused after a display interruption. Resume when ready.".into())
                    .map_err(app_error)?;
            }
        }
        self.publish()?;
        let changed = self.sync_ui()?;
        let app_changed = dispatch.changed || changed;
        let system_frame = matches!(
            event,
            WindowEvent::Resized { .. }
                | WindowEvent::RedrawRequested
                | WindowEvent::ScaleFactorChanged { .. }
        );
        let debug_refresh = if let (Some(debug), Some(started)) = (&mut self.debug, event_started) {
            let record_cost = app_changed
                || system_frame
                || matches!(
                    event,
                    WindowEvent::Key { .. }
                        | WindowEvent::PointerMoved { .. }
                        | WindowEvent::PointerButton { .. }
                );
            debug.event_finished(
                self.now,
                app_changed || system_frame,
                started.elapsed(),
                record_cost,
            )
        } else {
            false
        };
        if app_changed || debug_refresh {
            scheduler.invalidate();
        }
        // This app owns one scheduler deadline. Recompute it so a new
        // presentation cannot leave an older HUD expiry armed.
        scheduler.cancel_timed_request();
        let next = self
            .clock
            .next_deadline()
            .into_iter()
            .chain(self.debug_overlay_deadline())
            .min();
        if let Some(deadline) = next {
            scheduler.request_at(deadline);
        }
        Ok(())
    }
    fn apply_action(&mut self, action: Action) -> Result<(), oxiview_core::Error> {
        match action {
            Action::Primary => match self.model.phase() {
                Phase::Ready | Phase::Paused => {
                    self.model.start();
                    self.clock.start(self.now, self.model.tick_interval());
                    self.ui.notice.set(String::new()).map_err(app_error)?;
                }
                Phase::Running => {
                    self.model.pause();
                    self.clock.cancel();
                    self.ui
                        .notice
                        .set("Paused. Your next move can wait.".into())
                        .map_err(app_error)?;
                }
                Phase::Over(_) | Phase::Won => self.new_run()?,
            },
            Action::NewGame => self.new_run()?,
            Action::Pause => {
                self.pause_for_interruption("Paused. Your next move can wait.")?;
            }
            Action::Turn(direction) => {
                self.model.turn(direction);
            }
            Action::Difficulty(difficulty) => {
                if self.model.set_difficulty(difficulty) {
                    self.ui
                        .notice
                        .set(format!(
                            "{} pace selected. Start when you are ready.",
                            difficulty.name()
                        ))
                        .map_err(app_error)?;
                }
            }
        }
        Ok(())
    }
    fn new_run(&mut self) -> Result<(), oxiview_core::Error> {
        let next = self
            .run_id
            .checked_add(1)
            .ok_or_else(|| app_error("run identity exhausted"))?;
        self.model.restart(true).map_err(app_error)?;
        self.run_id = next;
        self.clock.start(self.now, self.model.tick_interval());
        self.ui.notice.set(String::new()).map_err(app_error)?;
        Ok(())
    }
    fn pause_for_interruption(&mut self, message: &str) -> Result<(), oxiview_core::Error> {
        if self.model.pause() {
            self.clock.cancel();
            self.ui.notice.set(message.into()).map_err(app_error)?;
        }
        Ok(())
    }
    fn publish(&mut self) -> Result<(), oxiview_core::Error> {
        let snapshot = self.model.snapshot();
        if matches!(snapshot.phase, Phase::Over(_) | Phase::Won)
            && self.recorded_run != Some(self.run_id)
        {
            self.history.insert(
                0,
                RunSummary {
                    id: self.run_id,
                    score: snapshot.score,
                    level: snapshot.level,
                    won: snapshot.phase == Phase::Won,
                },
            );
            self.history.truncate(3);
            self.recorded_run = Some(self.run_id);
            self.clock.cancel();
            self.ui
                .notice
                .set(
                    if snapshot.phase == Phase::Won {
                        "Every cell is yours. Start a new run to play again."
                    } else {
                        "A fresh board is one Enter away."
                    }
                    .into(),
                )
                .map_err(app_error)?;
        }
        batch(|| {
            self.ui.state.set_if_changed(snapshot.clone())?;
            self.ui
                .overlay_visible
                .set_if_changed(snapshot.phase != Phase::Running)?;
            self.ui.history.set_if_changed(self.history.clone())?;
            self.ui
                .history_empty
                .set_if_changed(self.history.is_empty())?;
            Ok(())
        })
        .map_err(app_error)
    }
    fn observe_reports(&self, reports: Vec<UiBatchReport>) {
        for report in reports {
            self.ui.counters.canceled_bindings.fetch_add(
                u64::try_from(report.canceled_bindings).unwrap_or(u64::MAX),
                Ordering::SeqCst,
            );
            for outcome in report.outcomes {
                if let Err(error) = outcome.result {
                    self.ui
                        .counters
                        .rejected_commands
                        .fetch_add(1, Ordering::SeqCst);
                    eprintln!(
                        "snake: batch {} command {} rejected: {error}",
                        report.batch, outcome.sequence
                    );
                }
            }
        }
    }
    fn sync_ui(&mut self) -> Result<bool, oxiview_core::Error> {
        let updates = self.tree.flush_updates(self.viewport, |_, _, _| {})?;
        self.observe_reports(updates.reports);
        self.tree.layout(self.viewport)?;
        let size = self.board_bounds()?.size();
        if self.ui.board_size.set_if_changed(size).map_err(app_error)? {
            let board = self.tree.flush_updates(self.viewport, |_, _, _| {})?;
            self.observe_reports(board.reports);
            return Ok(updates.changed || board.changed);
        }
        Ok(updates.changed)
    }
    /// Produces a complete Scene at a real display scale, using the same path as native paint.
    pub fn render(
        &mut self,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, oxiview_core::Error> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(app_error("invalid rendering scale"));
        }
        self.viewport = Size::new(
            f64::from(size.width) / scale,
            f64::from(size.height) / scale,
        );
        self.sync_ui()?;
        let mut painted = self.tree.paint(self.viewport, scale)?;
        if let Some(debug) = &mut self.debug {
            let draws = painted
                .scene
                .encode(painted.root)
                .map_err(app_error)?
                .nodes
                .iter()
                .filter(|node| matches!(node, oxiview_scene::EncodedNode::Draw { .. }))
                .count();
            debug
                .draw(
                    &mut painted,
                    SceneInfo {
                        viewport: self.viewport,
                        scale,
                        nodes: self.tree.len(),
                        draws,
                        work: self.tree.diagnostics(),
                    },
                )
                .map_err(app_error)?;
        }
        Ok(painted)
    }
}
impl Application for SnakeApp {
    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        waker: &WindowWaker,
    ) -> Result<(), oxiview_core::Error> {
        if let Some(debug) = &mut self.debug {
            debug.set_waker(waker);
        }
        self.process(event, scheduler)
    }
    fn paint(
        &mut self,
        timing: FrameTiming,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, oxiview_core::Error> {
        if let Some(debug) = &mut self.debug {
            debug.begin_paint(timing);
        }
        self.render(size, scale)
    }
    fn presented(&mut self, _: Option<LatencyRecord>) {
        self.ui.counters.presented.fetch_add(1, Ordering::SeqCst);
    }
    fn frame_submitted(&mut self, frame: FrameSubmission) {
        self.presented(frame.latency);
        if let Some(debug) = &mut self.debug {
            debug.presented(frame.submission_timestamp);
        }
    }
}

/// Opens the original Snake screen on a supported native platform.
pub fn run_native(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    run_native_with_screen(config, screen)
}

/// Opens the supplied screen with the shared native window and game loop.
#[cfg(target_os = "macos")]
pub fn run_native_with_screen(
    config: Config,
    screen: Screen,
) -> Result<(), Box<dyn std::error::Error>> {
    let counters = Arc::new(Counters::default());
    oxiview_app::run_macos(
        oxiview_platform_macos::MacOsPlatform::default(),
        WindowAttributes {
            title: if config.debug_overlay {
                "Snake — oxiview · Debug"
            } else {
                "Snake — oxiview"
            }
            .into(),
            logical_size: Size::new(f64::from(config.width), f64::from(config.height)),
            resize_timeout: Duration::from_millis(250),
        },
        move || SnakeApp::with_screen(config, counters, screen),
    )?;
    Ok(())
}

/// Reports that only the headless game is currently available on other hosts.
#[cfg(not(target_os = "macos"))]
pub fn run_native_with_screen(_: Config, _: Screen) -> Result<(), Box<dyn std::error::Error>> {
    Err("the native game currently requires macOS 14+; use --headless on other hosts".into())
}
