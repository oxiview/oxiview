//! List rows, table cells, and column headings mounted by the virtual collections.

use super::{
    Ui,
    style::{
        ACCENT, INK, LINE, MUTED, SOFT, WHITE, caption, color, fill, label, pad, px, severity_ink,
        typeface,
    },
};
use crate::{
    app::Intent,
    model::{Column, LogBook, Severity},
};
use oxiview_base::{BezierPath, Color, CornerRadii, PathCommand, Point};
use oxiview_core::{
    Action, NodeSemantics, RichText, TextSpan, TextSpanStyle, TextStyle, View, node,
};
use oxiview_scene::{Brush, FillRule, Primitive};
use oxiview_style::{
    Alignment, Edges, FlexDirection, LayoutStyle, Length, Overflow, Position, StateVariant, Style,
    StylePatch,
};
use oxiview_widgets::collection::{ListRow, TableItem};

const LIST_MESSAGE_X: f64 = 207.;
const SEAM: u32 = 0xe4ece9;
const STRIPE: u32 = 0xf7faf9;
const HOVER: u32 = 0xb5d3ca;
const SELECTED: u32 = 0xd9eee8;
const ACTIVE: u32 = 0xc4e4dc;

pub(crate) fn severity_text(level: Severity) -> Result<RichText, oxiview_text::Error> {
    let label = level.label();
    RichText::new(
        label,
        vec![TextSpan {
            range: 0..label.len(),
            style: TextSpanStyle {
                color: Some(color(severity_ink(level))),
                ..Default::default()
            },
        }],
    )
}

fn row_face(index: usize, selected: bool, is_active: bool) -> u32 {
    if is_active {
        ACTIVE
    } else if selected {
        SELECTED
    } else if index.is_multiple_of(2) {
        WHITE
    } else {
        STRIPE
    }
}

pub(crate) fn row_background(index: usize, selected: bool, is_active: bool) -> Color {
    color(row_face(index, selected, is_active))
}

pub(crate) fn list_line_typeface() -> TextStyle {
    typeface(13., 430, INK)
}

pub(crate) fn list_message_width(row_width: f64) -> f64 {
    (row_width - LIST_MESSAGE_X - 14.).max(1.)
}

fn row_overflow() -> Style {
    Style {
        border_color: Color::from_rgba8(0, 0, 0, 0),
        layout: LayoutStyle {
            min_width: px(0.),
            min_height: px(0.),
            border_width: 1.,
            ..Default::default()
        },
        overflow: Overflow::Clip,
        ..Default::default()
    }
}

pub(crate) fn list_row(
    ui: &Ui,
    book: &LogBook,
    row: ListRow,
    selected: bool,
    expanded: bool,
) -> View {
    let Some(record) = book.record_at(row.index) else {
        return node();
    };
    let background = row_face(row.index, selected, false);
    let mut style = row_overflow();
    style.layout.align_items = Alignment::Start;
    style.layout.padding = Edges {
        left: px(11.),
        right: px(13.),
        top: px(7.),
        bottom: px(5.),
    };
    style.background = color(background);
    node()
        .style(style)
        .style_variant(
            StateVariant::Hovered,
            StylePatch {
                border_color: Some(color(HOVER)),
                ..Default::default()
            },
        )
        .semantics(NodeSemantics {
            role: "listitem".into(),
            label: format!("Record {}", record.line),
            value: record.severity.label().into(),
            ..Default::default()
        })
        .child(disclosure(expanded))
        .child(caption(ui, format!("#{}", record.line), px(53.), MUTED))
        .child(caption(
            ui,
            book.cell(row.index, Column::Time).into_owned(),
            px(78.),
            MUTED,
        ))
        .child(caption(
            ui,
            record.severity.label(),
            px(47.),
            severity_ink(record.severity),
        ))
        .child(if expanded {
            node()
                .style(Style {
                    layout: LayoutStyle {
                        align_self: Some(Alignment::Stretch),
                        flex_direction: FlexDirection::Column,
                        ..fill()
                    },
                    overflow: Overflow::Clip,
                    ..Default::default()
                })
                .child(label(
                    ui,
                    record.message.as_str(),
                    list_line_typeface(),
                    LayoutStyle {
                        width: Length::Percent(1.),
                        flex_shrink: 0.,
                        ..Default::default()
                    },
                ))
        } else {
            label(
                ui,
                compact(&record.message),
                list_line_typeface(),
                LayoutStyle {
                    height: px(18.),
                    ..fill()
                },
            )
        })
}

pub(crate) fn compact(value: &str) -> String {
    const MAX_CHARS: usize = 384;
    let mut chars = value.chars();
    let mut out = String::with_capacity(value.len().min(MAX_CHARS));
    for char in chars.by_ref().take(MAX_CHARS) {
        out.push(match char {
            '\n' => '↵',
            '\r' | '\t' => ' ',
            char => char,
        });
    }
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

fn triangle(points: [(f64, f64); 3], ink: u32) -> Vec<Primitive> {
    let [first, second, third] = points.map(|(x, y)| Point::new(x, y));
    BezierPath::new(vec![
        PathCommand::MoveTo(first),
        PathCommand::LineTo(second),
        PathCommand::LineTo(third),
        PathCommand::Close,
    ])
    .map(|path| Primitive::FillPath(path, FillRule::NonZero, Brush::Solid(color(ink))))
    .into_iter()
    .collect()
}

fn disclosure(expanded: bool) -> View {
    let points = if expanded {
        [(1., 6.), (8., 6.), (4.5, 10.)]
    } else {
        [(2., 4.), (6., 7.5), (2., 11.)]
    };
    node()
        .style(Style {
            layout: LayoutStyle {
                width: px(17.),
                height: px(16.),
                ..Default::default()
            },
            hit_test: false,
            ..Default::default()
        })
        .content(triangle(points, ACCENT))
}

pub(crate) fn heading_label(book: &LogBook, column: Column) -> String {
    match book.sort() {
        Some((active, ascending)) if active == column => format!(
            "{} ({})",
            column.label(),
            if ascending { "ascending" } else { "descending" }
        ),
        _ => column.label().into(),
    }
}

pub(crate) fn heading_indicator(book: &LogBook, column: Column) -> Vec<Primitive> {
    match book.sort() {
        Some((active, true)) if active == column => {
            triangle([(0., 8.), (7., 8.), (3.5, 4.)], ACCENT)
        }
        Some((active, false)) if active == column => {
            triangle([(0., 5.), (7., 5.), (3.5, 9.)], ACCENT)
        }
        _ => Vec::new(),
    }
}

pub(crate) fn table_item(ui: &Ui, book: &LogBook, item: TableItem, column: Column) -> View {
    let mut style = row_overflow();
    style.layout.align_items = Alignment::Center;
    style.layout.padding = pad(9., 0.);
    let is_row_header = column == Column::Number;
    let (value, font, role) = match item.row {
        Some((_, index)) => {
            style.background = row_background(index, false, false);
            let text = compact(book.cell(index, column).as_ref());
            let ink = match column {
                Column::Level => book
                    .record_at(index)
                    .map_or(MUTED, |record| severity_ink(record.severity)),
                Column::Message => INK,
                _ => MUTED,
            };
            let weight = if column == Column::Level { 650 } else { 450 };
            (
                text,
                typeface(12., weight, ink),
                if is_row_header { "rowheader" } else { "cell" },
            )
        }
        None => {
            style.background = color(SOFT);
            (
                column.label().into(),
                typeface(11., 700, INK),
                "columnheader",
            )
        }
    };
    let line = if item.row.is_some() { SEAM } else { LINE };
    let cell = node()
        .style(style)
        .style_variant(
            StateVariant::Hovered,
            StylePatch {
                border_color: Some(color(HOVER)),
                ..Default::default()
            },
        )
        .semantics(NodeSemantics {
            role: role.into(),
            label: if item.row.is_some() {
                value.clone()
            } else {
                heading_label(book, column)
            },
            ..Default::default()
        })
        .child(label(
            ui,
            value,
            font,
            LayoutStyle {
                height: px(17.),
                ..fill()
            },
        ))
        .child(node().style(Style {
            background: color(line),
            border_radii: CornerRadii::uniform(0.),
            layout: LayoutStyle {
                position: Position::Absolute,
                width: px(1.),
                inset: Edges {
                    right: px(0.),
                    top: px(0.),
                    bottom: px(0.),
                    left: Length::Auto,
                },
                ..Default::default()
            },
            hit_test: false,
            ..Default::default()
        }));
    if item.row.is_some() {
        cell
    } else {
        let queue = ui.intents.clone();
        cell.child(
            node()
                .style(Style {
                    layout: LayoutStyle {
                        width: px(9.),
                        height: px(13.),
                        flex_shrink: 0.,
                        ..Default::default()
                    },
                    hit_test: false,
                    ..Default::default()
                })
                .content(heading_indicator(book, column)),
        )
        .on_action(
            Action::new("log.sort-column", "Sort by this column"),
            move |_, _, _| {
                queue.push(Intent::Sort(column));
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::Intents,
        model::{Batch, Record},
        view::LIST_HEIGHT,
    };
    use oxiview_base::Rect;
    use oxiview_core::{TextSystem, UiTree};
    use std::path::Path;

    #[test]
    fn list_row_announces_the_source_line_after_a_source_switch()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut book = LogBook::with_sample_count(2);
        book.apply(
            Path::new("new-source.log"),
            Batch {
                reset: true,
                replace_partial: false,
                records: vec![Record::parse("First line of the new source".into())],
            },
            true,
        )?;
        let record = book.record_at(0).ok_or("missing record")?;
        assert_eq!(record.line, 1);
        assert_ne!(record.id, record.line as u64);

        let mut text = TextSystem::without_system_fonts();
        text.register_font(
            include_bytes!("../../../../crates/oxiview-text/tests/fonts/NotoSans.ttf").to_vec(),
        )?;
        let ui = Ui::new(text, Intents::default());
        let mut tree = UiTree::new()?;
        let row = ListRow {
            key: record.id,
            index: 0,
            frame: Rect::new(0., 0., 640., LIST_HEIGHT),
        };
        let node = tree.mount(tree.root(), list_row(&ui, &book, row, false, false))?;
        assert_eq!(
            tree.semantics(node)?.ok_or("missing row semantics")?.label,
            "Record 1"
        );
        Ok(())
    }
}
