//! Persistent log viewer shell and its retained control handles.

mod records;
mod style;

pub(crate) use records::{
    compact, heading_indicator, heading_label, list_line_typeface, list_message_width, list_row,
    row_background, severity_text, table_item,
};

use style::{
    ACCENT, INK, LINE, MUTED, SOFT, WHITE, caption, color, fill, label, pad, px, severity_ink,
    typeface,
};

use crate::{
    app::{Intent, Intents, Mode},
    model::Severity,
};
use oxiview_core::{
    Error, FocusPolicy, NodeId, NodeSemantics, PointerEventKind, TextStyle, TextSystem, View, node,
};
use oxiview_platform::PointerButton;
use oxiview_style::{
    Alignment, Display, Edges, FlexDirection, FlexWrap, LayoutPatch, LayoutStyle, Length, Overflow,
    Position, StateVariant, Style, StylePatch,
};
use oxiview_widgets::controls::{
    Button, Scrollbar, ScrollbarAxis, Segmented, TextField, Toggle, ToggleKind,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub(crate) const LIST_HEIGHT: f64 = 34.;
pub(crate) const TABLE_HEIGHT: f64 = 29.;
pub(crate) const HEADER_HEIGHT: f64 = 34.;
pub(crate) const DISCLOSURE_EDGE: f64 = 29.;
const DETAIL_TEXT_PADDING: f64 = 16.;

const SUBTLE: u32 = 0x647975;
const PAPER: u32 = 0xf7faf9;

#[derive(Clone, Default)]
pub(crate) struct Slot(Rc<Cell<Option<NodeId>>>);

impl Slot {
    fn mount(&self, view: View) -> View {
        let slot = self.0.clone();
        view.on_mount(move |id| slot.set(Some(id)))
    }

    pub(crate) fn get(&self) -> Result<NodeId, Error> {
        self.0
            .get()
            .ok_or_else(|| Error::Application("Expected a mounted log viewer node".into()))
    }
}

#[derive(Clone)]
pub(crate) struct Ui {
    pub(crate) text: Rc<RefCell<TextSystem>>,
    pub(crate) open: Button,
    pub(crate) copy: Button,
    pub(crate) severity: [Toggle; 5],
    pub(crate) follow: Toggle,
    pub(crate) resume: Button,
    pub(crate) mode: Segmented,
    pub(crate) search: TextField,
    pub(crate) collection: Slot,
    pub(crate) header: Slot,
    pub(crate) legend: Slot,
    pub(crate) detail: Slot,
    pub(crate) detail_body: Slot,
    pub(crate) detail_label: Slot,
    pub(crate) empty: Slot,
    pub(crate) title: Slot,
    pub(crate) status: Slot,
    pub(crate) count: Slot,
    pub(crate) arrivals: Slot,
    pub(crate) horizontal: Scrollbar,
    pub(crate) horizontal_bar: Slot,
    pub(crate) vertical: Scrollbar,
    intents: Intents,
}

pub(crate) struct Inputs {
    mode: View,
    header: View,
    vertical: View,
    horizontal: View,
}

impl Ui {
    pub(crate) fn new(text: TextSystem, intents: Intents) -> Self {
        Self {
            text: Rc::new(RefCell::new(text)),
            open: Button::default(),
            copy: Button::default(),
            severity: Default::default(),
            follow: Toggle::default(),
            resume: Button::default(),
            mode: Segmented::default(),
            search: TextField::new(""),
            collection: Slot::default(),
            header: Slot::default(),
            legend: Slot::default(),
            detail: Slot::default(),
            detail_body: Slot::default(),
            detail_label: Slot::default(),
            empty: Slot::default(),
            title: Slot::default(),
            status: Slot::default(),
            count: Slot::default(),
            arrivals: Slot::default(),
            horizontal: Scrollbar::default(),
            horizontal_bar: Slot::default(),
            vertical: Scrollbar::default(),
            intents,
        }
    }

    pub(crate) fn inputs(&self) -> Result<Inputs, Error> {
        let queue = self.intents.clone();
        let mode = self.mode.view(
            self.text.clone(),
            "Record presentation",
            ["List", "Table"],
            typeface(12., 620, INK),
            move |index| {
                queue.push(Intent::Mode(if index == 0 {
                    Mode::List
                } else {
                    Mode::Table
                }))
            },
        )?;
        let queue = self.intents.clone();
        let vertical = self.vertical.view(
            ScrollbarAxis::Vertical,
            "Vertical position",
            move |offset| queue.push(Intent::Scrollbar(ScrollbarAxis::Vertical, offset)),
        )?;
        let queue = self.intents.clone();
        let horizontal = self.horizontal.view(
            ScrollbarAxis::Horizontal,
            "Horizontal position",
            move |offset| queue.push(Intent::Scrollbar(ScrollbarAxis::Horizontal, offset)),
        )?;
        let queue = self.intents.clone();
        let header = node().on_pointer(move |event, _, commands| {
            if pointer_interaction(event.kind, event.button, event.buttons) {
                match event.kind {
                    PointerEventKind::Down => commands.capture(event.current_target),
                    PointerEventKind::Up => commands.release_capture(event.current_target),
                    _ => {}
                }
                queue.push(Intent::Heading(*event));
            }
        });
        Ok(Inputs {
            mode,
            header,
            vertical,
            horizontal,
        })
    }
}

fn pointer_interaction(kind: PointerEventKind, button: Option<PointerButton>, buttons: u8) -> bool {
    match kind {
        PointerEventKind::Down | PointerEventKind::Up => button == Some(PointerButton::Primary),
        PointerEventKind::Move => buttons & 1 != 0,
        PointerEventKind::Cancel => true,
        _ => false,
    }
}

fn surface(layout: LayoutStyle, background: u32) -> Style {
    Style {
        layout,
        background: color(background),
        ..Default::default()
    }
}

fn rule() -> View {
    node().style(surface(
        LayoutStyle {
            height: px(1.),
            flex_shrink: 0.,
            ..Default::default()
        },
        LINE,
    ))
}

fn bar(height: f64, background: u32) -> Style {
    surface(
        LayoutStyle {
            height: px(height),
            flex_shrink: 0.,
            padding: pad(16., 0.),
            column_gap: px(8.),
            align_items: Alignment::Center,
            ..Default::default()
        },
        background,
    )
}

pub(crate) fn shell_style() -> Style {
    surface(
        LayoutStyle {
            width: Length::Percent(1.),
            height: Length::Percent(1.),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        },
        PAPER,
    )
}

fn toolbar(ui: &Ui) -> View {
    let queue = ui.intents.clone();
    let open = ui.open.view(
        ui.text.clone(),
        "Open…",
        typeface(12., 650, ACCENT),
        move || queue.push(Intent::Open),
    );
    let queue = ui.intents.clone();
    let copy = ui
        .copy
        .view(
            ui.text.clone(),
            "Copy",
            typeface(12., 630, INK),
            move || queue.push(Intent::Copy),
        )
        .disabled(true);
    node()
        .style(bar(56., WHITE))
        .child(label(
            ui,
            "Log Viewer",
            typeface(19., 720, INK),
            LayoutStyle {
                width: px(117.),
                flex_shrink: 0.,
                ..Default::default()
            },
        ))
        .child(open)
        .child(copy)
        .child(node().style(Style {
            layout: LayoutStyle {
                width: px(1.),
                height: px(20.),
                margin: pad(4., 0.),
                ..Default::default()
            },
            background: color(LINE),
            ..Default::default()
        }))
        .child(caption(ui, "Search", Length::Auto, MUTED))
        .child(
            ui.search
                .view(ui.text.clone(), "Search records", typeface(13., 430, INK)),
        )
}

fn filters(ui: &Ui, mode: View) -> View {
    let queue = ui.intents.clone();
    let follow = ui.follow.view(
        ui.text.clone(),
        "Follow",
        typeface(12., 650, INK),
        ToggleKind::Switch,
        false,
        move |checked| queue.push(Intent::Follow(checked)),
    );
    let mut filter_bar = bar(45., PAPER);
    filter_bar.layout.height = Length::Auto;
    filter_bar.layout.min_height = px(45.);
    filter_bar.layout.padding = pad(16., 4.);
    filter_bar.layout.flex_wrap = FlexWrap::Wrap;
    filter_bar.layout.row_gap = px(5.);
    node()
        .style(filter_bar)
        .child(caption(ui, "LEVEL", px(40.), MUTED))
        .children(
            Severity::ALL
                .into_iter()
                .enumerate()
                .map(|(index, severity)| {
                    let queue = ui.intents.clone();
                    ui.severity[index].view(
                        ui.text.clone(),
                        severity.label(),
                        typeface(12., 620, severity_ink(severity)),
                        ToggleKind::Checkbox,
                        true,
                        move |checked| queue.push(Intent::Severity(severity, checked)),
                    )
                }),
        )
        .child(node().style(Style {
            layout: fill(),
            hit_test: false,
            ..Default::default()
        }))
        .child(mode)
        .child(follow)
}

fn collection(ui: &Ui, header: View, vertical: View, horizontal: View) -> View {
    let queue = ui.intents.clone();
    let list = node()
        .style(Style {
            layout: LayoutStyle {
                position: Position::Relative,
                ..fill()
            },
            background: color(WHITE),
            overflow: Overflow::Clip,
            stationary_hover_updates: true,
            ..Default::default()
        })
        .style_variant(
            StateVariant::Focused,
            StylePatch {
                focus_ring: Some(oxiview_style::FocusRing {
                    color: color(ACCENT),
                    width: 1.5,
                }),
                ..Default::default()
            },
        )
        .focusable(FocusPolicy::Tab)
        .semantics(NodeSemantics {
            role: "list".into(),
            label: "Log records".into(),
            ..Default::default()
        })
        .on_pointer(move |event, _, commands| {
            if pointer_interaction(event.kind, event.button, event.buttons) {
                match event.kind {
                    PointerEventKind::Down => commands.capture(event.current_target),
                    PointerEventKind::Up => commands.release_capture(event.current_target),
                    _ => {}
                }
                queue.push(Intent::RecordPointer(*event));
            }
        });
    let header_box = Style {
        layout: LayoutStyle {
            position: Position::Relative,
            height: px(HEADER_HEIGHT),
            flex_shrink: 0.,
            ..Default::default()
        },
        background: color(SOFT),
        overflow: Overflow::Clip,
        ..Default::default()
    };
    let legend = node()
        .style(Style {
            layout: LayoutStyle {
                padding: pad(13., 0.),
                align_items: Alignment::Center,
                ..header_box.layout.clone()
            },
            ..header_box
        })
        .child(caption(ui, "RECORD", px(69.), MUTED))
        .child(caption(ui, "TIME", px(78.), MUTED))
        .child(caption(ui, "LEVEL", px(47.), MUTED))
        .child(caption(ui, "MESSAGE", Length::Auto, MUTED));
    let empty = ui.empty.mount(label(
        ui,
        "No records match the current filters.",
        typeface(14., 560, MUTED),
        LayoutStyle {
            position: Position::Absolute,
            inset: Edges {
                top: px(19.),
                left: px(16.),
                right: px(16.),
                bottom: Length::Auto,
            },
            ..Default::default()
        },
    ));
    node()
        .style(Style {
            layout: LayoutStyle {
                flex_direction: FlexDirection::Column,
                ..fill()
            },
            ..Default::default()
        })
        .child(rule())
        .child(
            node()
                .style(Style {
                    layout: fill(),
                    ..Default::default()
                })
                .child(
                    node()
                        .style(Style {
                            layout: LayoutStyle {
                                flex_direction: FlexDirection::Column,
                                ..fill()
                            },
                            ..Default::default()
                        })
                        .child(ui.legend.mount(legend))
                        .child(ui.header.mount(header.style(Style {
                            layout: LayoutStyle {
                                display: Display::None,
                                ..header_box.layout
                            },
                            ..header_box
                        })))
                        .child(
                            node()
                                .style(Style {
                                    layout: LayoutStyle {
                                        position: Position::Relative,
                                        ..fill()
                                    },
                                    ..Default::default()
                                })
                                .child(ui.collection.mount(list))
                                .child(empty),
                        ),
                )
                .child(vertical_rail(vertical)),
        )
        .child(
            ui.horizontal_bar.mount(
                node()
                    .style(Style {
                        layout: LayoutStyle {
                            display: Display::None,
                            height: px(12.),
                            flex_direction: FlexDirection::Column,
                            margin: Edges {
                                right: px(12.),
                                ..Edges::all(px(0.))
                            },
                            flex_shrink: 0.,
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .child(horizontal),
            ),
        )
}

fn vertical_rail(track: View) -> View {
    node()
        .style(Style {
            layout: LayoutStyle {
                width: px(12.),
                flex_direction: FlexDirection::Column,
                flex_shrink: 0.,
                ..Default::default()
            },
            ..Default::default()
        })
        .child(node().style(surface(
            LayoutStyle {
                height: px(HEADER_HEIGHT),
                flex_shrink: 0.,
                ..Default::default()
            },
            SOFT,
        )))
        .child(track.style_patch(StylePatch {
            layout: LayoutPatch {
                flex_grow: Some(1.),
                ..Default::default()
            },
            ..Default::default()
        }))
}

fn detail(ui: &Ui) -> View {
    let body = label(
        ui,
        "Select a record to inspect its complete contents.",
        detail_typeface(),
        LayoutStyle {
            width: Length::Percent(1.),
            padding: pad(DETAIL_TEXT_PADDING, 7.),
            flex_shrink: 0.,
            ..Default::default()
        },
    );
    node()
        .style(Style {
            layout: LayoutStyle {
                height: px(150.),
                flex_direction: FlexDirection::Column,
                flex_shrink: 0.,
                ..Default::default()
            },
            ..Default::default()
        })
        .child(rule())
        .child(
            node()
                .style(bar(29., SOFT))
                .child(caption(ui, "RECORD DETAIL", px(104.), MUTED))
                .child(ui.detail_label.mount(caption(ui, "", Length::Auto, ACCENT))),
        )
        .child(
            ui.detail.mount(
                node()
                    .style(Style {
                        layout: LayoutStyle {
                            flex_direction: FlexDirection::Column,
                            ..fill()
                        },
                        overflow: Overflow::Clip,
                        background: color(WHITE),
                        ..Default::default()
                    })
                    .semantics(NodeSemantics {
                        role: "document".into(),
                        label: "Selected log record".into(),
                        ..Default::default()
                    })
                    .child(ui.detail_body.mount(body)),
            ),
        )
}

pub(crate) fn detail_typeface() -> TextStyle {
    typeface(13., 430, INK)
}

pub(crate) fn detail_content_width(width: f64) -> f64 {
    (width - 2. * DETAIL_TEXT_PADDING).max(1.)
}

fn footer(ui: &Ui) -> View {
    let queue = ui.intents.clone();
    let resume = ui.resume.view(
        ui.text.clone(),
        "Resume follow",
        typeface(11., 650, ACCENT),
        move || queue.push(Intent::Follow(true)),
    );
    node()
        .style(Style {
            layout: LayoutStyle {
                flex_direction: FlexDirection::Column,
                ..Default::default()
            },
            ..Default::default()
        })
        .child(rule())
        .child(
            node()
                .style(bar(36., PAPER))
                .child(ui.title.mount(caption(ui, "", px(140.), MUTED)))
                .child(ui.count.mount(caption(ui, "", px(154.), INK)))
                .child(ui.status.mount(label(
                    ui,
                    "",
                    typeface(11., 490, SUBTLE),
                    LayoutStyle {
                        height: px(16.),
                        ..fill()
                    },
                )))
                .child(ui.arrivals.mount(caption(ui, "", Length::Auto, ACCENT)))
                .child(resume),
        )
}

oxiview_macros::oxi_module!("src/screen.oxi");
