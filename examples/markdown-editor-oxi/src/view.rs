//! Persistent application shell, document panes and inline command bars.

use crate::commands::{Command, Commands};
use oxiview_base::{Color, CornerRadii};
use oxiview_core::{
    Action, Editable, FocusPolicy, NodeId, NodeSemantics, TextStyle, TextSystem, View,
    editable_text, node, text,
};
use oxiview_style::{
    Alignment, ContentAlignment, Display, Edges, FlexDirection, FocusRing, LayoutStyle, Length,
    Overflow, StateVariant, Style, StylePatch,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone, Default)]
pub(crate) struct Slot(Rc<Cell<Option<NodeId>>>);
impl Slot {
    fn mount(&self, view: View) -> View {
        let slot = self.0.clone();
        view.on_mount(move |id| slot.set(Some(id)))
    }
    pub(crate) fn get(&self) -> Result<NodeId, oxiview_core::Error> {
        self.0.get().ok_or_else(|| {
            oxiview_core::Error::Application("Expected a mounted editor view".into())
        })
    }
}

#[derive(Clone, Default)]
pub(crate) struct Slots {
    pub(crate) title: Slot,
    pub(crate) status: Slot,
    pub(crate) position: Slot,
    pub(crate) preview: Slot,
    pub(crate) find: Slot,
    pub(crate) guard: Slot,
}

#[derive(Clone)]
pub(crate) struct Ui {
    pub(crate) text: Rc<RefCell<TextSystem>>,
    pub(crate) source: Editable,
    pub(crate) query: Editable,
    pub(crate) replacement: Editable,
    pub(crate) slots: Slots,
    pub(crate) commands: Commands,
}

const INK: u32 = 0x263831;
const MUTED: u32 = 0x52665b;
const PAPER: u32 = 0xf9faf5;
const WHITE: u32 = 0xffffff;
const LINE: u32 = 0xd8dfd5;
const ACCENT: u32 = 0x265742;
const FOCUS: u32 = 0x456db4;

#[cfg(not(test))]
pub(crate) const SANS_FAMILIES: &str = "Noto Sans, system-ui";
#[cfg(test)]
pub(crate) const SANS_FAMILIES: &str =
    "Noto Sans, Noto Sans Arabic, Oxiview Editing Fixture, system-ui";

fn color(rgb: u32) -> Color {
    let [_, r, g, b] = rgb.to_be_bytes();
    Color::from_rgba8(r, g, b, 255)
}
fn px(n: f64) -> Length {
    Length::Points(n)
}
fn padding(x: f64, y: f64) -> Edges<Length> {
    Edges {
        left: px(x),
        right: px(x),
        top: px(y),
        bottom: px(y),
    }
}
fn fill() -> LayoutStyle {
    LayoutStyle {
        flex_grow: 1.,
        flex_basis: px(0.),
        min_width: px(0.),
        min_height: px(0.),
        ..LayoutStyle::default()
    }
}
fn style(layout: LayoutStyle, bg: u32) -> Style {
    Style {
        layout,
        background: color(bg),
        border_color: color(LINE),
        ..Style::default()
    }
}
fn typeface(size: f64, weight: u16, ink: u32) -> TextStyle {
    TextStyle {
        font_size: size,
        weight,
        family: Some(SANS_FAMILIES.into()),
        color: color(ink),
        ..TextStyle::default()
    }
}
fn label(ui: &Ui, value: &str, size: f64, weight: u16, ink: u32) -> View {
    text(ui.text.clone(), value, typeface(size, weight, ink)).style(Style {
        hit_test: false,
        ..Style::default()
    })
}
fn focus_ring(ink: u32, width: f64) -> StylePatch {
    StylePatch {
        focus_ring: Some(FocusRing {
            color: color(ink),
            width,
        }),
        ..StylePatch::default()
    }
}
fn button(ui: &Ui, cmd: Command, primary: bool) -> View {
    let commands = ui.commands.clone();
    let face = if primary { ACCENT } else { WHITE };
    let mut surface = style(
        LayoutStyle {
            height: px(30.),
            padding: padding(10., 0.),
            border_width: 1.,
            flex_shrink: 0.,
            align_items: Alignment::Center,
            justify_content: ContentAlignment::Center,
            ..LayoutStyle::default()
        },
        face,
    );
    surface.border_radii = CornerRadii::uniform(6.);
    if primary {
        surface.border_color = color(ACCENT)
    }
    node()
        .style(surface)
        .style_variant(
            StateVariant::Focused,
            focus_ring(if primary { 0xb8d0fc } else { FOCUS }, 1.5),
        )
        .focusable(FocusPolicy::Tab)
        .semantics(NodeSemantics {
            label: cmd.label().into(),
            role: "button".into(),
            ..Default::default()
        })
        .on_action(Action::new(cmd.id(), cmd.label()), move |_, _, _| {
            commands.push(cmd)
        })
        .activate_with(cmd.id())
        .child(label(
            ui,
            cmd.label(),
            13.,
            600,
            if primary { WHITE } else { INK },
        ))
}

pub(super) fn shell_style() -> Style {
    style(
        LayoutStyle {
            width: Length::Percent(1.),
            height: Length::Percent(1.),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        },
        PAPER,
    )
}
pub(super) fn horizontal() -> Style {
    style(
        LayoutStyle {
            border_width: 1.,
            column_gap: px(1.),
            ..fill()
        },
        LINE,
    )
}
pub(super) fn toolbar(ui: &Ui) -> View {
    let title = ui
        .slots
        .title
        .mount(label(ui, "Untitled.md", 21., 720, INK).style(Style {
            layout: LayoutStyle {
                height: px(28.),
                ..LayoutStyle::default()
            },
            overflow: Overflow::Clip,
            hit_test: false,
            ..Style::default()
        }));
    let name = node()
        .style(Style {
            layout: LayoutStyle {
                flex_grow: 1.,
                min_width: px(0.),
                align_items: Alignment::Center,
                ..Default::default()
            },
            ..Default::default()
        })
        .child(title);
    node().style(bar(54., WHITE)).child(name).children(
        [
            Command::New,
            Command::Open,
            Command::Save,
            Command::SaveAs,
            Command::Bold,
            Command::Italic,
            Command::Find,
        ]
        .into_iter()
        .map(|cmd| button(ui, cmd, cmd == Command::Save)),
    )
}
fn bar(height: f64, bg: u32) -> Style {
    style(
        LayoutStyle {
            height: px(height),
            flex_shrink: 0.,
            padding: padding(16., 0.),
            column_gap: px(6.),
            align_items: Alignment::Center,
            ..Default::default()
        },
        bg,
    )
}
pub(crate) fn command_bar(visible: bool, guard: bool) -> Style {
    let mut result = bar(46., if guard { 0xffefd5 } else { 0xeaf0e8 });
    if !visible {
        result.layout.display = Display::None
    }
    result
}
fn field(ui: &Ui, control: Editable, wide: bool) -> View {
    let mut surface = style(
        if wide {
            fill()
        } else {
            LayoutStyle {
                width: px(158.),
                height: px(30.),
                padding: padding(5., 3.),
                border_width: 1.,
                ..Default::default()
            }
        },
        if wide { PAPER } else { WHITE },
    );
    surface.overflow = Overflow::Clip;
    if wide {
        surface.layout.padding = padding(16., 10.)
    } else {
        surface.border_radii = CornerRadii::uniform(4.)
    }
    editable_text(
        ui.text.clone(),
        control,
        TextStyle {
            family: Some(if wide {
                format!("Menlo, monospace, {SANS_FAMILIES}")
            } else {
                SANS_FAMILIES.into()
            }),
            ..typeface(if wide { 15. } else { 14. }, 400, INK)
        },
    )
    .style(surface)
    .style_variant(StateVariant::Focused, focus_ring(FOCUS, 1.))
}
pub(super) fn find_bar(ui: &Ui) -> View {
    ui.slots.find.mount(
        node()
            .style(command_bar(false, false))
            .child(label(ui, "Find", 13., 600, INK))
            .child(field(ui, ui.query.clone(), false))
            .child(label(ui, "Replace with", 13., 600, INK))
            .child(field(ui, ui.replacement.clone(), false))
            .children(
                [
                    Command::Previous,
                    Command::Next,
                    Command::Replace,
                    Command::ReplaceAll,
                    Command::Dismiss,
                ]
                .into_iter()
                .map(|c| button(ui, c, false)),
            ),
    )
}
pub(super) fn prompt(ui: &Ui) -> View {
    ui.slots.guard.mount(
        node()
            .style(command_bar(false, true))
            .child(label(ui, "Save changes to this document?", 14., 650, INK))
            .children(
                [Command::Save, Command::Discard, Command::KeepWriting]
                    .into_iter()
                    .map(|c| button(ui, c, c == Command::Save)),
            ),
    )
}
pub(super) fn pane(ui: &Ui, is_source: bool) -> View {
    let title = if is_source {
        "Markdown source"
    } else {
        "Live preview"
    };
    let header = node()
        .style(bar(36., 0xeef2eb))
        .child(label(ui, title, 13., 650, MUTED));
    let body = if is_source {
        field(ui, ui.source.clone(), true)
    } else {
        let mut surface = style(fill(), WHITE);
        surface.overflow = Overflow::Clip;
        ui.slots.preview.mount(node().style(surface))
    };
    let surface = style(
        LayoutStyle {
            flex_direction: FlexDirection::Column,
            ..fill()
        },
        if is_source { PAPER } else { WHITE },
    );
    node().style(surface).child(header).child(body)
}
pub(super) fn footer(ui: &Ui) -> View {
    let message = ui.slots.status.mount(label(ui, "Ready", 12., 500, MUTED));
    let position = ui.slots.position.mount(label(ui, "", 12., 500, MUTED));
    let mut surface = bar(30., 0xeef2eb);
    surface.layout.justify_content = ContentAlignment::SpaceBetween;
    node().style(surface).child(message).child(position)
}

oxiview_macros::oxi_module!("src/screen.oxi");
