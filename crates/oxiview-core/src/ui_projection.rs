//! Projection between owned styling and layout contexts.
use oxiview_layout as layout;
use oxiview_style as style;

pub(crate) fn project_style(style: &style::LayoutStyle) -> layout::LayoutInput {
    layout::LayoutInput {
        display: display(style.display),
        position: position(style.position),
        width: length(style.width),
        height: length(style.height),
        min_width: length(style.min_width),
        min_height: length(style.min_height),
        max_width: length(style.max_width),
        max_height: length(style.max_height),
        aspect_ratio: style.aspect_ratio,
        inset: edges(style.inset),
        margin: edges(style.margin),
        padding: edges(style.padding),
        border_width: style.border_width,
        flex_direction: flexdirection(style.flex_direction),
        flex_wrap: flexwrap(style.flex_wrap),
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        flex_basis: length(style.flex_basis),
        row_gap: length(style.row_gap),
        column_gap: length(style.column_gap),
        align_items: alignment(style.align_items),
        align_self: style.align_self.map(alignment),
        justify_items: alignment(style.justify_items),
        justify_self: style.justify_self.map(alignment),
        justify_content: contentalignment(style.justify_content),
        align_content: contentalignment(style.align_content),
        grid_template_columns: style
            .grid_template_columns
            .iter()
            .copied()
            .map(track)
            .collect(),
        grid_template_rows: style
            .grid_template_rows
            .iter()
            .copied()
            .map(track)
            .collect(),
        grid_auto_columns: style.grid_auto_columns.iter().copied().map(track).collect(),
        grid_auto_rows: style.grid_auto_rows.iter().copied().map(track).collect(),
        grid_auto_flow: gridautoflow(style.grid_auto_flow),
        grid_column_start: gridplacement(style.grid_column_start),
        grid_column_end: gridplacement(style.grid_column_end),
        grid_row_start: gridplacement(style.grid_row_start),
        grid_row_end: gridplacement(style.grid_row_end),
    }
}
fn display(v: style::Display) -> layout::Display {
    match v {
        style::Display::Flex => layout::Display::Flex,
        style::Display::Grid => layout::Display::Grid,
        style::Display::Block => layout::Display::Block,
        style::Display::None => layout::Display::None,
    }
}
fn position(v: style::Position) -> layout::Position {
    match v {
        style::Position::Static => layout::Position::Static,
        style::Position::Relative => layout::Position::Relative,
        style::Position::Absolute => layout::Position::Absolute,
    }
}
fn flexdirection(v: style::FlexDirection) -> layout::FlexDirection {
    match v {
        style::FlexDirection::Row => layout::FlexDirection::Row,
        style::FlexDirection::Column => layout::FlexDirection::Column,
        style::FlexDirection::RowReverse => layout::FlexDirection::RowReverse,
        style::FlexDirection::ColumnReverse => layout::FlexDirection::ColumnReverse,
    }
}
fn flexwrap(v: style::FlexWrap) -> layout::FlexWrap {
    match v {
        style::FlexWrap::NoWrap => layout::FlexWrap::NoWrap,
        style::FlexWrap::Wrap => layout::FlexWrap::Wrap,
        style::FlexWrap::WrapReverse => layout::FlexWrap::WrapReverse,
    }
}
fn alignment(v: style::Alignment) -> layout::Alignment {
    match v {
        style::Alignment::Stretch => layout::Alignment::Stretch,
        style::Alignment::Baseline => layout::Alignment::Baseline,
        style::Alignment::Start => layout::Alignment::Start,
        style::Alignment::End => layout::Alignment::End,
        style::Alignment::Center => layout::Alignment::Center,
    }
}
fn contentalignment(v: style::ContentAlignment) -> layout::ContentAlignment {
    match v {
        style::ContentAlignment::Normal => layout::ContentAlignment::Normal,
        style::ContentAlignment::Start => layout::ContentAlignment::Start,
        style::ContentAlignment::End => layout::ContentAlignment::End,
        style::ContentAlignment::Center => layout::ContentAlignment::Center,
        style::ContentAlignment::Stretch => layout::ContentAlignment::Stretch,
        style::ContentAlignment::SpaceBetween => layout::ContentAlignment::SpaceBetween,
        style::ContentAlignment::SpaceAround => layout::ContentAlignment::SpaceAround,
        style::ContentAlignment::SpaceEvenly => layout::ContentAlignment::SpaceEvenly,
    }
}
fn gridautoflow(v: style::GridAutoFlow) -> layout::GridAutoFlow {
    match v {
        style::GridAutoFlow::Row => layout::GridAutoFlow::Row,
        style::GridAutoFlow::Column => layout::GridAutoFlow::Column,
    }
}
fn length(v: style::Length) -> layout::Length {
    match v {
        style::Length::Auto => layout::Length::Auto,
        style::Length::Points(v) => layout::Length::Points(v),
        style::Length::Percent(v) => layout::Length::Percent(v),
    }
}
fn edges(v: style::Edges<style::Length>) -> layout::Edges<layout::Length> {
    layout::Edges {
        left: length(v.left),
        right: length(v.right),
        top: length(v.top),
        bottom: length(v.bottom),
    }
}
fn breadth(v: style::TrackBreadth) -> layout::TrackBreadth {
    match v {
        style::TrackBreadth::Auto => layout::TrackBreadth::Auto,
        style::TrackBreadth::Points(v) => layout::TrackBreadth::Points(v),
        style::TrackBreadth::Percent(v) => layout::TrackBreadth::Percent(v),
        style::TrackBreadth::Fr(v) => layout::TrackBreadth::Fr(v),
    }
}
fn track(v: style::TrackSize) -> layout::TrackSize {
    match v {
        style::TrackSize::Auto => layout::TrackSize::Auto,
        style::TrackSize::Points(v) => layout::TrackSize::Points(v),
        style::TrackSize::Percent(v) => layout::TrackSize::Percent(v),
        style::TrackSize::Fr(v) => layout::TrackSize::Fr(v),
        style::TrackSize::MinMax(min, max) => layout::TrackSize::MinMax(breadth(min), breadth(max)),
    }
}
fn gridplacement(v: style::GridPlacement) -> layout::GridPlacement {
    match v {
        style::GridPlacement::Auto => layout::GridPlacement::Auto,
        style::GridPlacement::Line(v) => layout::GridPlacement::Line(v),
        style::GridPlacement::Span(v) => layout::GridPlacement::Span(v),
    }
}
