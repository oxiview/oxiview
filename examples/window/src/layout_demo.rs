//! Interactive retained-tree preview. Bitmap captions are example diagnostics,
//! not a framework text API or a replacement for the M4 text milestone.

use oxiview_base::{Color, CornerRadii, Point, Rect, Size};
use oxiview_core::{
    Application, FrameTiming, LatencyRecord, NodeId, PaintedScene, PointerEventKind, Scheduler,
    UiCommands, UiError, UiPointerEvent, UiTree,
};
use oxiview_platform::{PhysicalSize, PointerButton, WindowAttributes, WindowEvent, WindowWaker};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_scene::{Brush, Clip, NodeKind, Primitive};
use oxiview_style::{
    Alignment, Display, Edges, FlexDirection, FlexWrap, LayoutStyle, Length, Overflow, Position,
    Style, TrackSize,
};
use std::{
    collections::HashMap,
    error::Error,
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

const WIDTH: u32 = 1120;
const HEIGHT: u32 = 800;
const INK: [u8; 3] = [32, 45, 55];
const MUTED: [u8; 3] = [105, 113, 119];
const PAPER: [u8; 3] = [246, 245, 240];
const WHITE: [u8; 3] = [255, 255, 252];
const ORANGE: [u8; 3] = [238, 150, 91];

fn color([r, g, b]: [u8; 3]) -> Color {
    Color::from_rgba8(r, g, b, 255)
}
fn px(value: f64) -> Length {
    Length::Points(value)
}
fn palette(index: usize) -> [u8; 3] {
    match index % 6 {
        0 => [174, 209, 208],
        1 => [237, 191, 160],
        2 => [204, 197, 229],
        3 => [213, 220, 174],
        4 => [173, 199, 225],
        _ => [232, 181, 185],
    }
}
fn column() -> LayoutStyle {
    LayoutStyle {
        flex_direction: FlexDirection::Column,
        min_width: px(0.0),
        min_height: px(0.0),
        ..Default::default()
    }
}
fn row(height: f64) -> LayoutStyle {
    LayoutStyle {
        height: px(height),
        flex_shrink: 0.0,
        column_gap: px(10.0),
        align_items: Alignment::Center,
        min_width: px(0.0),
        ..Default::default()
    }
}
fn surface(layout: LayoutStyle, background: [u8; 3]) -> Style {
    Style {
        layout,
        background: color(background),
        hit_test: false,
        ..Default::default()
    }
}
fn panel(height: Length, grow: f64) -> Style {
    let mut style = surface(
        LayoutStyle {
            height,
            flex_grow: grow,
            padding: Edges::all(px(18.0)),
            border_width: 1.0,
            row_gap: px(12.0),
            ..column()
        },
        WHITE,
    );
    style.border_color = color([220, 222, 216]);
    style.border_radii = CornerRadii::uniform(16.0);
    style
}
fn tile_style(index: usize) -> Style {
    let mut style = surface(
        LayoutStyle {
            width: px(106.0),
            height: px(70.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
        palette(index),
    );
    style.hit_test = true;
    style.border_radii = CornerRadii::uniform(12.0);
    style
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Action {
    Add,
    Remove,
    Wrap,
    Size,
    Layer,
    Reset,
}
#[derive(Clone, Copy)]
enum Role {
    Button(Action),
    Tile(usize),
    Grid(usize),
    Drag,
}
struct Label {
    node: NodeId,
    text: String,
    x: f64,
    y: f64,
    pixel: f64,
    ink: [u8; 3],
}
impl Label {
    fn new(node: NodeId, text: impl Into<String>, pixel: f64, ink: [u8; 3]) -> Self {
        Self {
            node,
            text: text.into(),
            x: 0.0,
            y: 0.0,
            pixel,
            ink,
        }
    }
    fn inset(mut self, x: f64, y: f64) -> Self {
        self.x = x;
        self.y = y;
        self
    }
}
struct Drag {
    grab: Point,
}
struct DemoState {
    flow: NodeId,
    stage: NodeId,
    draggable: NodeId,
    footer: NodeId,
    tiles: Vec<NodeId>,
    grid_tiles: Vec<NodeId>,
    controls: HashMap<Action, NodeId>,
    roles: HashMap<NodeId, Role>,
    labels: Vec<Label>,
    drag: Option<Drag>,
    pressed: Option<NodeId>,
    wrapped: bool,
    wide: bool,
    front: bool,
    next_tile: usize,
    last_action: &'static str,
    errors: Vec<String>,
    label_dirty: bool,
}

fn edit(tree: &UiTree, commands: &mut UiCommands, id: NodeId, change: impl FnOnce(&mut Style)) {
    if let Ok(style) = tree.style(id) {
        let mut style = style.clone();
        change(&mut style);
        commands.set_style(id, style);
    }
}
fn contains(rect: Rect, point: Point) -> bool {
    let origin = rect.origin();
    let size = rect.size();
    point.x() >= origin.x()
        && point.y() >= origin.y()
        && point.x() < origin.x() + size.width()
        && point.y() < origin.y() + size.height()
}
impl DemoState {
    fn background(role: Role, hovered: bool, pressed: bool) -> Color {
        let base = match role {
            Role::Button(Action::Add) => {
                if pressed {
                    [135, 73, 50]
                } else if hovered {
                    [52, 76, 87]
                } else {
                    INK
                }
            }
            Role::Button(_) => {
                if pressed {
                    [237, 204, 178]
                } else if hovered {
                    [226, 231, 224]
                } else {
                    WHITE
                }
            }
            Role::Tile(index) | Role::Grid(index) => {
                let [r, g, b] = palette(index);
                if pressed {
                    [
                        r.saturating_sub(28),
                        g.saturating_sub(28),
                        b.saturating_sub(28),
                    ]
                } else if hovered {
                    [
                        r.saturating_add(16),
                        g.saturating_add(16),
                        b.saturating_add(16),
                    ]
                } else {
                    [r, g, b]
                }
            }
            Role::Drag => {
                if pressed {
                    [225, 122, 62]
                } else if hovered {
                    [249, 170, 111]
                } else {
                    ORANGE
                }
            }
        };
        color(base)
    }
    fn handle(&mut self, event: &UiPointerEvent, tree: &UiTree, commands: &mut UiCommands) {
        if event.target != event.current_target {
            return;
        }
        let Some(role) = self.roles.get(&event.current_target).copied() else {
            return;
        };
        let id = event.current_target;
        match event.kind {
            PointerEventKind::Enter | PointerEventKind::Leave => {
                let hovered = matches!(event.kind, PointerEventKind::Enter);
                let pressed = self.pressed == Some(id);
                edit(tree, commands, id, |style| {
                    style.background = Self::background(role, hovered, pressed)
                });
            }
            PointerEventKind::Down if event.button == Some(PointerButton::Primary) => {
                self.pressed = Some(id);
                commands.capture(id);
                if matches!(role, Role::Drag) {
                    self.drag = Some(Drag {
                        grab: event.local_position,
                    });
                    self.last_action = "DRAGGING / CAPTURE ACTIVE";
                    self.label_dirty = true;
                }
                edit(tree, commands, id, |style| {
                    style.background = Self::background(role, true, true)
                });
            }
            PointerEventKind::Move if matches!(role, Role::Drag) => {
                if let Some(drag) = &self.drag
                    && let Ok(bounds) = tree.bounds(self.stage)
                {
                    let x = event.position.x() - bounds.origin().x() - drag.grab.x();
                    let y = event.position.y() - bounds.origin().y() - drag.grab.y();
                    edit(tree, commands, id, |style| {
                        style.layout.inset.left = px(x);
                        style.layout.inset.top = px(y);
                    });
                }
            }
            PointerEventKind::Up if event.button == Some(PointerButton::Primary) => {
                let inside = tree
                    .bounds(id)
                    .is_ok_and(|bounds| contains(bounds, event.position));
                let updated_tile = self.pressed == Some(id)
                    && inside
                    && matches!(role, Role::Tile(_) | Role::Grid(_));
                if self.pressed == Some(id) && inside {
                    match role {
                        Role::Button(action) => self.activate(action, tree, commands),
                        Role::Tile(_) => {
                            edit(tree, commands, id, |style| {
                                style.layout.width = if style.layout.width == px(106.0) {
                                    px(160.0)
                                } else {
                                    px(106.0)
                                };
                                style.background = Self::background(role, inside, false);
                            });
                            self.last_action = "TILE WIDTH CHANGED";
                            self.label_dirty = true;
                        }
                        Role::Grid(_) => {
                            edit(tree, commands, id, |style| {
                                style.opacity = if style.opacity < 1.0 { 1.0 } else { 0.55 };
                                style.background = Self::background(role, inside, false);
                            });
                            self.last_action = "GRID GROUP OPACITY CHANGED";
                            self.label_dirty = true;
                        }
                        Role::Drag => {}
                    }
                }
                if matches!(role, Role::Drag) {
                    self.drag = None;
                    self.last_action = "DRAG ENDED / CAPTURE RELEASED";
                    self.label_dirty = true;
                }
                self.pressed = None;
                if !updated_tile {
                    edit(tree, commands, id, |style| {
                        style.background = Self::background(role, inside, false)
                    });
                }
            }
            PointerEventKind::Cancel => {
                self.pressed = None;
                self.drag = None;
                self.last_action = "INTERACTION CANCELED";
                self.label_dirty = true;
                edit(tree, commands, id, |style| {
                    style.background = Self::background(role, false, false)
                });
            }
            _ => {}
        }
    }
    fn activate(&mut self, action: Action, tree: &UiTree, commands: &mut UiCommands) {
        match action {
            Action::Add => {
                if self.tiles.len() >= 32 {
                    self.last_action = "PREVIEW LIMIT / 32 TILES";
                } else {
                    let number = self.next_tile;
                    match commands.append(self.flow, tile_style(number)) {
                        Ok(id) => {
                            self.tiles.push(id);
                            self.roles.insert(id, Role::Tile(number));
                            self.labels.push(
                                Label::new(id, format!("{:02}", number + 1), 2.0, INK)
                                    .inset(14.0, 14.0),
                            );
                            self.next_tile += 1;
                            self.last_action = "NODE CREATED AND MOUNTED";
                        }
                        Err(error) => self.errors.push(error.to_string()),
                    }
                }
            }
            Action::Remove => {
                if let Some(id) = self.tiles.pop() {
                    commands.remove(id);
                    self.roles.remove(&id);
                    self.labels.retain(|label| label.node != id);
                    self.last_action = "NODE REMOVED / OLD ID INVALID";
                } else {
                    self.last_action = "FLOW IS EMPTY / ADD A TILE";
                }
            }
            Action::Wrap => {
                self.wrapped = !self.wrapped;
                let wrap = if self.wrapped {
                    FlexWrap::Wrap
                } else {
                    FlexWrap::NoWrap
                };
                edit(tree, commands, self.flow, |style| {
                    style.layout.flex_wrap = wrap
                });
                self.last_action = if self.wrapped {
                    "FLEX WRAP ENABLED"
                } else {
                    "FLEX WRAP DISABLED / CLIPPED"
                };
            }
            Action::Size => {
                self.wide = !self.wide;
                let width = if self.wide { 148.0 } else { 106.0 };
                for &id in &self.tiles {
                    edit(tree, commands, id, |style| style.layout.width = px(width));
                }
                self.last_action = "TILE DIMENSIONS UPDATED / REFLOW";
            }
            Action::Layer => {
                self.front = !self.front;
                let z = if self.front { 6 } else { 2 };
                edit(tree, commands, self.draggable, |style| {
                    style.z_index = Some(z)
                });
                self.last_action = if self.front {
                    "ORANGE LAYER MOVED TO FRONT"
                } else {
                    "ORANGE LAYER MOVED BEHIND BLUE"
                };
            }
            Action::Reset => {
                self.wrapped = true;
                self.wide = false;
                self.front = false;
                edit(tree, commands, self.flow, |style| {
                    style.layout.flex_wrap = FlexWrap::Wrap
                });
                for &id in &self.tiles {
                    edit(tree, commands, id, |style| style.layout.width = px(106.0));
                }
                edit(tree, commands, self.draggable, |style| {
                    style.layout.inset.left = px(34.0);
                    style.layout.inset.top = px(42.0);
                    style.z_index = Some(2);
                });
                self.last_action = "POSITIONS AND LAYOUT RESET";
            }
        }
        self.label_dirty = true;
    }
}

struct LayoutDemo {
    tree: UiTree,
    state: DemoState,
    viewport: Size,
    presents: Arc<AtomicU64>,
    trace_input: bool,
}
impl LayoutDemo {
    fn new(presents: Arc<AtomicU64>, trace_input: bool) -> Result<Self, UiError> {
        let mut tree = UiTree::new()?;
        let root = tree.root();
        tree.set_style(
            root,
            surface(
                LayoutStyle {
                    width: Length::Percent(1.0),
                    height: Length::Percent(1.0),
                    padding: Edges::all(px(24.0)),
                    row_gap: px(16.0),
                    ..column()
                },
                PAPER,
            ),
        )?;
        let mut labels = Vec::new();
        let mut roles = HashMap::new();
        let mut controls = HashMap::new();
        let header = tree.append(root, surface(row(62.0), PAPER))?;
        let title_group = tree.append(
            header,
            surface(
                LayoutStyle {
                    flex_grow: 1.0,
                    row_gap: px(8.0),
                    ..column()
                },
                PAPER,
            ),
        )?;
        let title = tree.append(title_group, surface(row(25.0), PAPER))?;
        labels.push(Label::new(title, "OXIVIEW / LAYOUT LAB", 3.0, INK));
        let subtitle = tree.append(title_group, surface(row(14.0), PAPER))?;
        labels.push(Label::new(
            subtitle,
            "ONE TREE. NATIVE PIXELS. LIVE INTERACTION.",
            1.35,
            MUTED,
        ));
        let mut badge_style = surface(
            LayoutStyle {
                width: px(176.0),
                height: px(34.0),
                flex_shrink: 0.0,
                ..Default::default()
            },
            INK,
        );
        badge_style.border_radii = CornerRadii::uniform(17.0);
        let badge = tree.append(header, badge_style)?;
        labels.push(Label::new(badge, "M2 / PREVIEW", 1.65, WHITE).inset(26.0, 11.0));

        let toolbar = tree.append(root, surface(row(44.0), PAPER))?;
        for (action, caption, width) in [
            (Action::Add, "+ ADD", 104.0),
            (Action::Remove, "- REMOVE", 126.0),
            (Action::Wrap, "WRAP", 104.0),
            (Action::Size, "RESIZE", 110.0),
            (Action::Layer, "LAYER", 104.0),
            (Action::Reset, "RESET", 104.0),
        ] {
            let mut style = surface(
                LayoutStyle {
                    width: px(width),
                    height: px(40.0),
                    flex_shrink: 0.0,
                    border_width: 1.0,
                    ..Default::default()
                },
                if action == Action::Add { INK } else { WHITE },
            );
            style.border_color = color([216, 220, 214]);
            style.border_radii = CornerRadii::uniform(10.0);
            style.hit_test = true;
            let id = tree.append(toolbar, style)?;
            controls.insert(action, id);
            roles.insert(id, Role::Button(action));
            labels.push(
                Label::new(
                    id,
                    caption,
                    1.65,
                    if action == Action::Add { WHITE } else { INK },
                )
                .inset(16.0, 14.0),
            );
        }
        let body = tree.append(
            root,
            surface(
                LayoutStyle {
                    flex_grow: 1.0,
                    min_height: px(320.0),
                    min_width: px(0.0),
                    column_gap: px(18.0),
                    ..Default::default()
                },
                PAPER,
            ),
        )?;
        let left = tree.append(
            body,
            surface(
                LayoutStyle {
                    flex_grow: 1.0,
                    flex_basis: px(0.0),
                    row_gap: px(16.0),
                    ..column()
                },
                PAPER,
            ),
        )?;
        let flow_panel = tree.append(left, panel(Length::Auto, 1.0))?;
        let flow_heading = tree.append(flow_panel, surface(row(30.0), WHITE))?;
        labels.push(Label::new(flow_heading, "01 / FLEX FLOW", 2.0, INK));
        let mut flow_style = surface(
            LayoutStyle {
                flex_grow: 1.0,
                min_height: px(160.0),
                min_width: px(0.0),
                flex_wrap: FlexWrap::Wrap,
                row_gap: px(12.0),
                column_gap: px(12.0),
                align_content: oxiview_style::ContentAlignment::Start,
                ..Default::default()
            },
            WHITE,
        );
        flow_style.overflow = Overflow::Clip;
        let flow = tree.append(flow_panel, flow_style)?;
        let mut tiles = Vec::new();
        for index in 0..8 {
            let id = tree.append(flow, tile_style(index))?;
            roles.insert(id, Role::Tile(index));
            labels.push(Label::new(id, format!("{:02}", index + 1), 2.0, INK).inset(14.0, 14.0));
            tiles.push(id);
        }
        let flow_note = tree.append(flow_panel, surface(row(13.0), WHITE))?;
        labels.push(Label::new(
            flow_note,
            "HOVER / CLICK TILES / ADD / REMOVE / REFLOW",
            1.2,
            MUTED,
        ));

        let grid_panel = tree.append(left, panel(px(220.0), 0.0))?;
        let grid_heading = tree.append(grid_panel, surface(row(28.0), WHITE))?;
        labels.push(Label::new(
            grid_heading,
            "02 / GRID / 1FR 1FR 1FR",
            1.8,
            INK,
        ));
        let grid = tree.append(
            grid_panel,
            surface(
                LayoutStyle {
                    display: Display::Grid,
                    flex_grow: 1.0,
                    min_height: px(100.0),
                    grid_template_columns: vec![TrackSize::Fr(1.0); 3],
                    grid_template_rows: vec![TrackSize::Fr(1.0); 2],
                    row_gap: px(10.0),
                    column_gap: px(10.0),
                    ..Default::default()
                },
                WHITE,
            ),
        )?;
        let mut grid_tiles = Vec::new();
        for index in 0..6 {
            let mut style = surface(LayoutStyle::default(), palette(index + 2));
            style.hit_test = true;
            style.border_radii = CornerRadii::uniform(8.0);
            let id = tree.append(grid, style)?;
            roles.insert(id, Role::Grid(index + 2));
            labels.push(Label::new(id, format!("FR / {}", index + 1), 1.6, INK).inset(13.0, 13.0));
            grid_tiles.push(id);
        }

        let right = tree.append(
            body,
            surface(
                LayoutStyle {
                    width: px(318.0),
                    flex_shrink: 0.0,
                    row_gap: px(16.0),
                    ..column()
                },
                PAPER,
            ),
        )?;
        let layer_panel = tree.append(right, panel(px(330.0), 0.0))?;
        let layer_heading = tree.append(layer_panel, surface(row(28.0), WHITE))?;
        labels.push(Label::new(layer_heading, "03 / LAYERS + CLIP", 1.8, INK));
        let mut stage_style = surface(
            LayoutStyle {
                position: Position::Relative,
                height: px(214.0),
                flex_shrink: 0.0,
                ..Default::default()
            },
            [43, 59, 67],
        );
        stage_style.z_index = Some(0);
        stage_style.overflow = Overflow::Clip;
        stage_style.border_radii = CornerRadii::uniform(13.0);
        let stage = tree.append(layer_panel, stage_style)?;
        for (x, y, width, height, radius, tone, z) in [
            (84.0, 24.0, 142.0, 110.0, 14.0, [126, 175, 174], 1),
            (126.0, 108.0, 142.0, 130.0, 20.0, [158, 174, 216], 3),
        ] {
            let mut style = surface(
                LayoutStyle {
                    position: Position::Absolute,
                    width: px(width),
                    height: px(height),
                    inset: Edges {
                        left: px(x),
                        top: px(y),
                        ..Edges::all(Length::Auto)
                    },
                    ..Default::default()
                },
                tone,
            );
            style.border_radii = CornerRadii::uniform(radius);
            style.z_index = Some(z);
            tree.append(stage, style)?;
        }
        let mut drag_style = surface(
            LayoutStyle {
                position: Position::Absolute,
                width: px(132.0),
                height: px(96.0),
                inset: Edges {
                    left: px(34.0),
                    top: px(42.0),
                    ..Edges::all(Length::Auto)
                },
                ..Default::default()
            },
            ORANGE,
        );
        drag_style.border_radii = CornerRadii::uniform(15.0);
        drag_style.z_index = Some(2);
        drag_style.hit_test = true;
        let draggable = tree.append(stage, drag_style)?;
        roles.insert(draggable, Role::Drag);
        let layer_note = tree.append(layer_panel, surface(row(16.0), WHITE))?;
        labels.push(Label::new(
            layer_note,
            "DRAG ORANGE / TOGGLE LAYER",
            1.2,
            MUTED,
        ));

        let block_panel = tree.append(right, panel(Length::Auto, 1.0))?;
        let block_heading = tree.append(block_panel, surface(row(25.0), WHITE))?;
        labels.push(Label::new(block_heading, "04 / BLOCK FLOW", 1.8, INK));
        let block = tree.append(
            block_panel,
            surface(
                LayoutStyle {
                    display: Display::Block,
                    flex_grow: 1.0,
                    min_height: px(100.0),
                    ..Default::default()
                },
                WHITE,
            ),
        )?;
        for (index, ratio) in [0.95, 0.67, 0.82].into_iter().enumerate() {
            let mut style = surface(
                LayoutStyle {
                    height: px(24.0),
                    width: Length::Percent(ratio),
                    margin: Edges {
                        top: px(7.0),
                        bottom: px(7.0),
                        ..Edges::all(px(0.0))
                    },
                    ..Default::default()
                },
                palette(index + 3),
            );
            style.border_radii = CornerRadii::uniform(6.0);
            tree.append(block, style)?;
        }
        let footer = tree.append(root, surface(row(18.0), PAPER))?;
        let viewport = Size::new(f64::from(WIDTH), f64::from(HEIGHT));
        tree.layout(viewport)?;
        Ok(Self {
            tree,
            viewport,
            presents,
            trace_input,
            state: DemoState {
                flow,
                stage,
                draggable,
                footer,
                tiles,
                grid_tiles,
                controls,
                roles,
                labels,
                drag: None,
                pressed: None,
                wrapped: true,
                wide: false,
                front: false,
                next_tile: 8,
                last_action: "READY / RESIZE THE WINDOW OR TRY THE CONTROLS",
                errors: Vec::new(),
                label_dirty: false,
            },
        })
    }
    fn input(&mut self, event: &WindowEvent) -> Result<bool, oxiview_core::Error> {
        if let WindowEvent::Resized {
            physical_size,
            scale_factor,
            ..
        } = event
        {
            self.viewport = Size::new(
                f64::from(physical_size.width) / scale_factor,
                f64::from(physical_size.height) / scale_factor,
            );
        }
        let state = &mut self.state;
        let mut drag_events = Vec::new();
        let result = self
            .tree
            .dispatch(event, self.viewport, |event, tree, commands| {
                if self.trace_input
                    && event.target == state.draggable
                    && event.current_target == state.draggable
                {
                    drag_events.push(event.kind);
                }
                state.handle(event, tree, commands)
            })
            .map_err(application_error)?;
        if self.trace_input
            && matches!(
                event,
                WindowEvent::PointerMoved { .. }
                    | WindowEvent::PointerButton { .. }
                    | WindowEvent::PointerLeft { .. }
                    | WindowEvent::PointerInterrupted { .. }
                    | WindowEvent::ActivationChanged { .. }
                    | WindowEvent::VisibilityChanged { .. }
                    | WindowEvent::Resized { .. }
                    | WindowEvent::Destroyed
            )
        {
            eprintln!(
                "layout-input {event:?} viewport={:?} drag_events={drag_events:?} captured={} hovered={}",
                self.viewport,
                self.tree.capture_holder() == Some(state.draggable),
                self.tree.hover_target() == Some(state.draggable),
            );
        }
        for report in result.reports {
            for outcome in report.outcomes {
                if let Err(error) = outcome.result {
                    self.state.errors.push(format!(
                        "batch {} command {}: {error}",
                        report.batch, outcome.sequence
                    ));
                }
            }
        }
        for error in self.state.errors.drain(..) {
            eprintln!("layout-preview: {error}");
        }
        let label_changed = std::mem::take(&mut self.state.label_dirty);
        Ok(result.changed || label_changed)
    }
    fn compose(
        &mut self,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, oxiview_core::Error> {
        self.viewport = Size::new(
            f64::from(size.width) / scale,
            f64::from(size.height) / scale,
        );
        let mut painted = self
            .tree
            .paint(self.viewport, scale)
            .map_err(application_error)?;
        for label in &self.state.labels {
            if let Ok(bounds) = self.tree.bounds(label.node) {
                let clip = if self.state.tiles.contains(&label.node) {
                    self.tree.bounds(self.state.flow).ok()
                } else {
                    None
                };
                caption(&mut painted, bounds, clip, label).map_err(application_error)?;
            }
        }
        let footer_text = format!(
            "{:02} FLOW NODES  /  {}",
            self.state.tiles.len(),
            self.state.last_action
        );
        let footer = Label::new(self.state.footer, footer_text, 1.2, MUTED);
        if let Ok(bounds) = self.tree.bounds(self.state.footer) {
            caption(&mut painted, bounds, None, &footer).map_err(application_error)?;
        }
        Ok(painted)
    }
}
fn application_error(error: impl std::fmt::Display) -> oxiview_core::Error {
    oxiview_core::Error::Application(error.to_string())
}
impl Application for LayoutDemo {
    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        _: &WindowWaker,
    ) -> Result<(), oxiview_core::Error> {
        if self.input(event)? {
            scheduler.invalidate();
        }
        Ok(())
    }
    fn paint(
        &mut self,
        _: FrameTiming,
        size: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, oxiview_core::Error> {
        self.compose(size, scale)
    }
    fn presented(&mut self, _: Option<LatencyRecord>) {
        self.presents.fetch_add(1, Ordering::SeqCst);
    }
}

fn caption(
    painted: &mut PaintedScene,
    bounds: Rect,
    enclosing_clip: Option<Rect>,
    label: &Label,
) -> Result<(), oxiview_scene::Error> {
    let index = painted.scene.children(Some(painted.root))?.len();
    let clip_bounds = if let Some(parent) = enclosing_clip {
        let left = bounds.origin().x().max(parent.origin().x());
        let top = bounds.origin().y().max(parent.origin().y());
        let right = (bounds.origin().x() + bounds.size().width())
            .min(parent.origin().x() + parent.size().width());
        let bottom = (bounds.origin().y() + bounds.size().height())
            .min(parent.origin().y() + parent.size().height());
        if right <= left || bottom <= top {
            return Ok(());
        }
        Rect::new(left, top, right, bottom)
    } else {
        bounds
    };
    let clip = painted.scene.insert(
        Some(painted.root),
        index,
        NodeKind::Clip(Clip::Rect(clip_bounds)),
    )?;
    let origin = bounds.origin();
    let mut draw_index = 0;
    for (letter, character) in label.text.chars().enumerate() {
        let x = origin.x() + label.x + letter as f64 * 6.0 * label.pixel;
        if x >= origin.x() + bounds.size().width() {
            break;
        }
        for (row, bits) in glyph(character).into_iter().enumerate() {
            let mut column = 0;
            while column < 5 {
                if bits & (1 << (4 - column)) == 0 {
                    column += 1;
                    continue;
                }
                let start = column;
                while column < 5 && bits & (1 << (4 - column)) != 0 {
                    column += 1;
                }
                let y = origin.y() + label.y + row as f64 * label.pixel;
                painted.scene.insert(
                    Some(clip),
                    draw_index,
                    NodeKind::Draw(Primitive::Fill(
                        Rect::new(
                            x + f64::from(start) * label.pixel,
                            y,
                            x + f64::from(column) * label.pixel,
                            y + label.pixel,
                        ),
                        CornerRadii::uniform(0.0),
                        Brush::Solid(color(label.ink)),
                    )),
                )?;
                draw_index += 1;
            }
        }
    }
    Ok(())
}
fn glyph(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 25, 21, 19, 19, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        '/' => [1, 1, 2, 4, 8, 16, 16],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        ':' => [0, 12, 12, 0, 12, 12, 0],
        _ => [0; 7],
    }
}

fn write_image(
    demo: &mut LayoutDemo,
    width: u32,
    height: u32,
    scale: f64,
    path: &Path,
) -> Result<(), Box<dyn Error>> {
    let painted = demo.compose(PhysicalSize { width, height }, scale)?;
    let image = CpuRenderer.render(
        &painted.scene,
        painted.root,
        oxiview_scene::PixelSize { width, height },
    )?;
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(image.pixels())?;
    writer.finish()?;
    Ok(())
}
fn center(rect: Rect) -> Point {
    Point::new(
        rect.origin().x() + rect.size().width() / 2.0,
        rect.origin().y() + rect.size().height() / 2.0,
    )
}
fn click(demo: &mut LayoutDemo, action: Action, clock: &mut u64) -> Result<(), Box<dyn Error>> {
    demo.tree.layout(demo.viewport)?;
    let id = *demo
        .state
        .controls
        .get(&action)
        .ok_or("missing preview control")?;
    let position = center(demo.tree.bounds(id)?);
    *clock += 1;
    demo.input(&WindowEvent::PointerMoved {
        position,
        timestamp: Duration::from_millis(*clock),
    })?;
    for pressed in [true, false] {
        *clock += 1;
        demo.input(&WindowEvent::PointerButton {
            button: PointerButton::Primary,
            pressed,
            position,
            timestamp: Duration::from_millis(*clock),
        })?;
    }
    Ok(())
}
fn exercise(demo: &mut LayoutDemo) -> Result<(), Box<dyn Error>> {
    let mut clock = 10;
    let count = demo.state.tiles.len();
    click(demo, Action::Add, &mut clock)?;
    let added = *demo.state.tiles.last().ok_or("add produced no node")?;
    if demo.state.tiles.len() != count + 1 || !demo.tree.contains(added) {
        return Err("add did not mount a live node".into());
    }
    click(demo, Action::Remove, &mut clock)?;
    if demo.state.tiles.len() != count || demo.tree.contains(added) {
        return Err("remove did not invalidate node identity".into());
    }
    click(demo, Action::Wrap, &mut clock)?;
    if demo.tree.style(demo.state.flow)?.layout.flex_wrap != FlexWrap::NoWrap {
        return Err("wrap toggle did not apply".into());
    }
    click(demo, Action::Wrap, &mut clock)?;
    click(demo, Action::Size, &mut clock)?;
    demo.tree.layout(demo.viewport)?;
    if let Some(&first) = demo.state.tiles.first()
        && demo.tree.bounds(first)?.size().width() < 140.0
    {
        return Err("tile resize did not reach layout".into());
    }
    let first = *demo
        .state
        .grid_tiles
        .first()
        .ok_or("grid fixture missing")?;
    let fourth = *demo.state.grid_tiles.get(3).ok_or("grid fixture missing")?;
    let a = demo.tree.bounds(first)?;
    let b = demo.tree.bounds(fourth)?;
    if b.origin().y() <= a.origin().y() || (a.origin().x() - b.origin().x()).abs() > 0.1 {
        return Err("grid did not produce two aligned rows".into());
    }
    let drag = demo.state.draggable;
    let before = demo.tree.bounds(drag)?;
    let start = Point::new(before.origin().x() + 18.0, before.origin().y() + 18.0);
    demo.input(&WindowEvent::PointerButton {
        button: PointerButton::Primary,
        pressed: true,
        position: start,
        timestamp: Duration::from_millis(100),
    })?;
    if demo.tree.capture_holder() != Some(drag) {
        return Err("drag failed to acquire capture".into());
    }
    let end = Point::new(start.x() + 62.0, start.y() + 34.0);
    demo.input(&WindowEvent::PointerMoved {
        position: end,
        timestamp: Duration::from_millis(101),
    })?;
    demo.input(&WindowEvent::PointerButton {
        button: PointerButton::Primary,
        pressed: false,
        position: end,
        timestamp: Duration::from_millis(102),
    })?;
    demo.tree.layout(demo.viewport)?;
    if demo.tree.capture_holder().is_some()
        || demo.tree.bounds(drag)?.origin().x() <= before.origin().x()
    {
        return Err("drag did not move and release capture".into());
    }
    click(demo, Action::Layer, &mut clock)?;
    if demo.tree.style(drag)?.z_index != Some(6) {
        return Err("layer toggle did not change z-index".into());
    }
    let position = Point::new(12.0, 12.0);
    demo.input(&WindowEvent::PointerMoved {
        position,
        timestamp: Duration::from_millis(200),
    })?;
    // Complete the requested preparation before checking a genuinely idle tree.
    let _ = demo.tree.paint(demo.viewport, 1.0)?;
    for tick in 201..211 {
        if demo.input(&WindowEvent::PointerMoved {
            position,
            timestamp: Duration::from_millis(tick),
        })? {
            return Err("unchanged pointer input requested redraw".into());
        }
    }
    Ok(())
}
pub(super) fn run(
    headless: bool,
    trace_input: bool,
    presents: Arc<AtomicU64>,
) -> Result<bool, Box<dyn Error>> {
    if headless {
        let mut demo = LayoutDemo::new(presents.clone(), trace_input)?;
        let directory =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/xtask/m2-preview");
        std::fs::create_dir_all(&directory)?;
        write_image(
            &mut demo,
            WIDTH,
            HEIGHT,
            1.0,
            &directory.join("initial.png"),
        )?;
        exercise(&mut demo)?;
        write_image(
            &mut demo,
            WIDTH,
            HEIGHT,
            1.0,
            &directory.join("interacted.png"),
        )?;
        write_image(&mut demo, 880, 760, 1.0, &directory.join("resized.png"))?;
        presents.fetch_add(3, Ordering::SeqCst);
        println!(
            "M2 preview checks passed: add/remove, stale identity, wrap, resize, grid, capture drag, z-index, unchanged-input redraw suppression."
        );
        println!("Preview images: {}", directory.display());
        return Ok(true);
    }
    #[cfg(target_os = "macos")]
    {
        println!(
            "M2 preview: use ADD/REMOVE/WRAP/RESIZE/LAYER/RESET; hover or click tiles; drag the orange tile across the clip or outside the window."
        );
        oxiview_app::run_macos(
            oxiview_platform_macos::MacOsPlatform::default(),
            WindowAttributes {
                title: "oxiview — M2 Layout Lab".into(),
                logical_size: Size::new(f64::from(WIDTH), f64::from(HEIGHT)),
                resize_timeout: Duration::from_millis(250),
            },
            move || LayoutDemo::new(presents, trace_input).map_err(application_error),
        )?;
        Ok(true)
    }
    #[cfg(not(target_os = "macos"))]
    Err(
        "native M2 preview currently requires macOS; use --layout-demo --headless on other hosts"
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_actions_use_live_nodes_and_capture() -> Result<(), Box<dyn Error>> {
        let mut demo = LayoutDemo::new(Arc::new(AtomicU64::new(0)), false)?;
        exercise(&mut demo)
    }
}
