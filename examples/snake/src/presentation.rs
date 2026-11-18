//! Shared Snake view inputs, typography, colors and control behavior.

pub use crate::board::draw as draw_board;
use crate::{
    application::{Action, Counters},
    game::{Difficulty, Direction, Phase, Snapshot},
};
use oxiview_base::{Color, CornerRadii, Point, Rect, Size};
use oxiview_core::{NodeId, PointerEventKind, UiCommands, UiPointerEvent, UiTree, View, text};
use oxiview_macros::Props;
use oxiview_platform::PointerButton;
use oxiview_reactive::{Memo, Signal};
use oxiview_style::{
    Alignment, ContentAlignment, Edges, FlexDirection, LayoutStyle, Length, Style,
};
use oxiview_text::{TextAlign, TextStyle, TextSystem};
use std::{
    cell::{Cell as LocalCell, RefCell},
    rc::Rc,
    sync::{Arc, mpsc::Sender},
};

/// Window background.
pub const PAPER: [u8; 3] = [245, 246, 239];
/// Light panel surface.
pub const WHITE: [u8; 3] = [255, 255, 250];
/// Primary text.
pub const INK: [u8; 3] = [31, 49, 39];
/// Secondary text.
pub const MUTED: [u8; 3] = [105, 120, 108];
/// Primary button surface.
const GREEN: [u8; 3] = [184, 232, 118];
/// Panel border.
pub const BORDER: [u8; 3] = [216, 223, 211];
/// Board background.
pub const BOARD: [u8; 3] = [18, 35, 30];

/// Converts a palette entry to an opaque color.
pub fn color([r, g, b]: [u8; 3]) -> Color {
    Color::from_rgba8(r, g, b, 255)
}
/// Converts logical points to a layout length.
pub fn px(value: f64) -> Length {
    Length::Points(value)
}
/// Column layout with shrinkable minimum dimensions.
pub fn column() -> LayoutStyle {
    LayoutStyle {
        flex_direction: FlexDirection::Column,
        min_width: px(0.0),
        min_height: px(0.0),
        ..Default::default()
    }
}
/// Row layout with centered children.
pub fn row() -> LayoutStyle {
    LayoutStyle {
        min_width: px(0.0),
        min_height: px(0.0),
        align_items: Alignment::Center,
        ..Default::default()
    }
}
/// Creates a filled, rounded, pointer-transparent panel.
pub fn surface(layout: LayoutStyle, background: [u8; 3], radius: f64) -> Style {
    Style {
        layout,
        background: color(background),
        border_radii: CornerRadii::uniform(radius),
        hit_test: false,
        ..Default::default()
    }
}
/// Creates a pointer-transparent layout container.
pub fn blank(layout: LayoutStyle) -> Style {
    Style {
        layout,
        hit_test: false,
        ..Default::default()
    }
}

/// One completed game, keyed by its session run number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSummary {
    /// Stable session run number.
    pub id: u64,
    /// Score for this run.
    pub score: u32,
    /// Level reached.
    pub level: u32,
    /// Whether the board was cleared.
    pub won: bool,
}

/// Retained node identities used for input and replay.
#[derive(Default)]
pub struct Slots {
    /// Mounted game board.
    pub board: Rc<LocalCell<Option<NodeId>>>,
    primary: Rc<LocalCell<Option<NodeId>>>,
    /// Primary control on the active overlay.
    pub overlay_primary: Rc<LocalCell<Option<NodeId>>>,
    restart: Rc<LocalCell<Option<NodeId>>>,
    chill: Rc<LocalCell<Option<NodeId>>>,
    classic: Rc<LocalCell<Option<NodeId>>>,
    swift: Rc<LocalCell<Option<NodeId>>>,
    up: Rc<LocalCell<Option<NodeId>>>,
    right: Rc<LocalCell<Option<NodeId>>>,
    down: Rc<LocalCell<Option<NodeId>>>,
    left: Rc<LocalCell<Option<NodeId>>>,
    /// Mounted keyed history container.
    pub history: Rc<LocalCell<Option<NodeId>>>,
}
impl Slots {
    /// Returns the mount point for an action in the sidebar.
    pub fn button(&self, action: Action) -> Rc<LocalCell<Option<NodeId>>> {
        match action {
            Action::Primary | Action::Pause => Rc::clone(&self.primary),
            Action::NewGame => Rc::clone(&self.restart),
            Action::Difficulty(Difficulty::Chill) => Rc::clone(&self.chill),
            Action::Difficulty(Difficulty::Classic) => Rc::clone(&self.classic),
            Action::Difficulty(Difficulty::Swift) => Rc::clone(&self.swift),
            Action::Turn(Direction::Up) => Rc::clone(&self.up),
            Action::Turn(Direction::Right) => Rc::clone(&self.right),
            Action::Turn(Direction::Down) => Rc::clone(&self.down),
            Action::Turn(Direction::Left) => Rc::clone(&self.left),
        }
    }
}

/// Signals, mount points and services available to either Snake screen.
#[derive(Clone)]
pub struct Ui {
    pub(crate) text: Rc<RefCell<TextSystem>>,
    /// Current game snapshot.
    pub state: Signal<Snapshot>,
    /// Current game phase.
    pub phase: Memo<Phase>,
    /// Selected pace.
    pub difficulty: Memo<Difficulty>,
    /// Current game score.
    pub score: Memo<u32>,
    /// Best session score.
    pub best: Memo<u32>,
    /// Committed logical board size.
    pub board_size: Signal<Size>,
    /// Visibility of the game overlay.
    pub overlay_visible: Signal<bool>,
    /// Recent runs.
    pub history: Signal<Vec<RunSummary>>,
    /// Whether the empty history message is visible.
    pub history_empty: Signal<bool>,
    /// Status message below the game.
    pub notice: Signal<String>,
    /// Mount points shared with input and replay.
    pub slots: Rc<Slots>,
    pub(crate) actions: Sender<Action>,
    pub(crate) errors: Sender<String>,
    /// Component and binding diagnostics.
    pub counters: Arc<Counters>,
}

/// Creates a left-aligned game label.
pub fn label(ui: &Ui, source: impl Into<String>, size: f64, weight: u16, ink: [u8; 3]) -> View {
    text(
        Rc::clone(&ui.text),
        source,
        TextStyle {
            font_size: size,
            weight,
            color: color(ink),
            line_height: 1.25,
            ..Default::default()
        },
    )
    .style(blank(LayoutStyle {
        flex_shrink: 0.0,
        min_width: px(0.0),
        ..Default::default()
    }))
}
/// Creates a centered, full-width game label.
pub fn centered_label(
    ui: &Ui,
    source: impl Into<String>,
    size: f64,
    weight: u16,
    ink: [u8; 3],
) -> View {
    text(
        Rc::clone(&ui.text),
        source,
        TextStyle {
            font_size: size,
            weight,
            color: color(ink),
            align: TextAlign::Center,
            line_height: 1.3,
            ..Default::default()
        },
    )
    .style(blank(LayoutStyle {
        width: Length::Percent(1.0),
        flex_shrink: 0.0,
        min_width: px(0.0),
        ..Default::default()
    }))
}
/// Returns the primary button caption for a game phase.
pub fn primary_caption(phase: Phase) -> &'static str {
    match phase {
        Phase::Ready => "Start game",
        Phase::Running => "Pause",
        Phase::Paused => "Resume",
        Phase::Over(_) | Phase::Won => "Play again",
    }
}
/// Returns the status badge caption.
pub fn phase_caption(phase: Phase) -> &'static str {
    match phase {
        Phase::Ready => "Ready",
        Phase::Running => "Playing",
        Phase::Paused => "Paused",
        Phase::Over(_) => "Game over",
        Phase::Won => "Board cleared",
    }
}
fn button_enabled(action: Action, phase: Phase) -> bool {
    match action {
        Action::Difficulty(_) => !matches!(phase, Phase::Running | Phase::Paused),
        Action::Turn(_) => phase == Phase::Running,
        _ => true,
    }
}
/// Resolves the shared game button appearance before local press feedback.
pub fn button_style(
    action: Action,
    phase: Phase,
    difficulty: Difficulty,
    primary: bool,
    compact: bool,
) -> Style {
    let enabled = button_enabled(action, phase);
    let selected = matches!(action, Action::Difficulty(choice) if choice == difficulty);
    let directional = matches!(action, Action::Turn(_));
    let mut style = surface(
        LayoutStyle {
            width: if directional {
                px(38.0)
            } else if compact {
                Length::Auto
            } else {
                Length::Percent(1.0)
            },
            height: px(if directional {
                36.0
            } else if compact {
                32.0
            } else if primary {
                46.0
            } else {
                38.0
            }),
            flex_grow: if compact && !directional { 1.0 } else { 0.0 },
            flex_shrink: 0.0,
            border_width: 1.0,
            padding: Edges::all(px(0.0)),
            justify_content: ContentAlignment::Center,
            align_items: Alignment::Center,
            ..row()
        },
        if primary {
            GREEN
        } else if selected {
            [226, 237, 210]
        } else {
            WHITE
        },
        if compact { 9.0 } else { 12.0 },
    );
    style.hit_test = enabled;
    style.opacity = if enabled { 1.0 } else { 0.48 };
    style.border_color = color(if primary {
        [160, 207, 101]
    } else if selected {
        [139, 169, 112]
    } else {
        BORDER
    });
    style
}

/// Inputs passed once when mounting the screen component.
#[derive(Props)]
pub struct ScreenProps {
    /// View inputs owned by the application.
    pub ui: Ui,
}

impl Ui {
    /// Builds the same capture, cancel and release-inside handler for both screens.
    pub fn button_pointer(
        self,
        action: Action,
        pressed: Signal<bool>,
    ) -> impl FnMut(&UiPointerEvent, &UiTree, &mut UiCommands) {
        let actions = self.actions;
        let errors = self.errors;
        move |event, tree, commands| {
            if event.target != event.current_target {
                return;
            }
            match event.kind {
                PointerEventKind::Down if event.button == Some(PointerButton::Primary) => {
                    commands.capture(event.current_target);
                    if let Err(error) = pressed.set(true) {
                        let _ = errors.send(error.to_string());
                    }
                }
                PointerEventKind::Up if event.button == Some(PointerButton::Primary) => {
                    let was_pressed = match pressed.get() {
                        Ok(value) => value,
                        Err(error) => {
                            let _ = errors.send(error.to_string());
                            false
                        }
                    };
                    let inside = tree
                        .bounds(event.current_target)
                        .is_ok_and(|bounds| in_rect(bounds, event.position));
                    if let Err(error) = pressed.set(false) {
                        let _ = errors.send(error.to_string());
                    }
                    if was_pressed && inside {
                        // Dropping the application receiver during shutdown ends action delivery.
                        let _ = actions.send(action);
                    }
                }
                PointerEventKind::Cancel => {
                    if let Err(error) = pressed.set(false) {
                        let _ = errors.send(error.to_string());
                    }
                }
                _ => {}
            }
        }
    }
}

fn in_rect(rect: Rect, point: Point) -> bool {
    let origin = rect.origin();
    let size = rect.size();
    point.x() >= origin.x()
        && point.y() >= origin.y()
        && point.x() < origin.x() + size.width()
        && point.y() < origin.y() + size.height()
}
