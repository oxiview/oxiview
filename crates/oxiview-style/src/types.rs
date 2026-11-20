/// A typed length in logical coordinates; percentages are fractions of the reference size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Length {
    /// Content or available-space dependent sizing.
    #[default]
    Auto,
    /// A logical length.
    Points(f64),
    /// A fractional percentage, where 1.0 means 100 percent.
    Percent(f64),
}

/// Four independently specified box edges.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edges<T> {
    /// Left edge.
    pub left: T,
    /// Right edge.
    pub right: T,
    /// Top edge.
    pub top: T,
    /// Bottom edge.
    pub bottom: T,
}
impl<T: Copy> Edges<T> {
    /// Uses one value on all four edges.
    pub const fn all(value: T) -> Self {
        Self {
            left: value,
            right: value,
            top: value,
            bottom: value,
        }
    }
    /// Returns values in left, right, top, bottom order.
    pub const fn values(self) -> [T; 4] {
        [self.left, self.right, self.top, self.bottom]
    }
}

/// Two independently specified axes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Dimensions<T> {
    /// Horizontal component.
    pub width: T,
    /// Vertical component.
    pub height: T,
}

/// Node layout participation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Display {
    /// Flexbox layout.
    #[default]
    Flex,
    /// Grid layout.
    Grid,
    /// Normal block flow.
    Block,
    /// Suppress this subtree.
    None,
}

/// Positioning relative to structural ancestors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Position {
    /// Normal flow with ignored insets.
    #[default]
    Static,
    /// Normal flow with a relative offset.
    Relative,
    /// Out of flow within the nearest positioned ancestor.
    Absolute,
}

/// Main-axis direction for flex layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlexDirection {
    /// Horizontal forward flow.
    #[default]
    Row,
    /// Vertical forward flow.
    Column,
    /// Horizontal reverse flow.
    RowReverse,
    /// Vertical reverse flow.
    ColumnReverse,
}

/// Flex line wrapping policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlexWrap {
    /// Keep a single line.
    #[default]
    NoWrap,
    /// Wrap into forward lines.
    Wrap,
    /// Wrap into reverse lines.
    WrapReverse,
}

/// Alignment of an item within its available area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alignment {
    /// Stretch the auto-sized item.
    #[default]
    Stretch,
    /// Align at the start edge.
    Start,
    /// Align at the end edge.
    End,
    /// Center the item.
    Center,
    /// Align first baselines in a flex row or grid row. Uses start on the inline axis.
    Baseline,
}

/// Distribution of a group of tracks or items.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContentAlignment {
    /// Use the layout family's normal distribution (flex start or track stretch).
    #[default]
    Normal,
    /// Pack at the logical start.
    Start,
    /// Pack at the end.
    End,
    /// Center the group.
    Center,
    /// Stretch available tracks.
    Stretch,
    /// Distribute space between items.
    SpaceBetween,
    /// Distribute space around items.
    SpaceAround,
    /// Use equal edge and internal spaces.
    SpaceEvenly,
}

/// Sequential grid auto-placement direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridAutoFlow {
    /// Fill rows first.
    #[default]
    Row,
    /// Fill columns first.
    Column,
}

/// A breadth used in a grid minmax track.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum TrackBreadth {
    /// Intrinsic automatic sizing.
    #[default]
    Auto,
    /// A logical length.
    Points(f64),
    /// A fractional percentage.
    Percent(f64),
    /// A fraction of free space, allowed only as a maximum.
    Fr(f64),
}
/// An explicit or implicit grid track.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum TrackSize {
    /// Intrinsic automatic sizing.
    #[default]
    Auto,
    /// A logical length.
    Points(f64),
    /// A fractional percentage.
    Percent(f64),
    /// A fraction of free space.
    Fr(f64),
    /// Minimum and maximum breadth.
    MinMax(TrackBreadth, TrackBreadth),
}
/// A grid line or span, with zero line/span values invalid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridPlacement {
    /// Automatic placement.
    #[default]
    Auto,
    /// A signed one-based grid line.
    Line(i16),
    /// A positive number of tracks.
    Span(u16),
}

/// Owned box, positioning, Flex, Grid, and Block inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutStyle {
    /// Layout family.
    pub display: Display,
    /// Positioning mode.
    pub position: Position,
    /// Preferred border-box width.
    pub width: Length,
    /// Preferred border-box height.
    pub height: Length,
    /// Minimum border-box width.
    pub min_width: Length,
    /// Minimum border-box height.
    pub min_height: Length,
    /// Maximum border-box width.
    pub max_width: Length,
    /// Maximum border-box height.
    pub max_height: Length,
    /// Optional positive width-to-height ratio.
    pub aspect_ratio: Option<f64>,
    /// Offsets for positioned nodes.
    pub inset: Edges<Length>,
    /// Outside spacing, including negative and auto values.
    pub margin: Edges<Length>,
    /// Nonnegative interior spacing; auto is invalid.
    pub padding: Edges<Length>,
    /// Uniform nonnegative logical border width.
    pub border_width: f64,
    /// Flex main-axis direction.
    pub flex_direction: FlexDirection,
    /// Flex wrapping policy.
    pub flex_wrap: FlexWrap,
    /// Nonnegative flex growth factor.
    pub flex_grow: f64,
    /// Nonnegative flex shrink factor.
    pub flex_shrink: f64,
    /// Initial flex main size.
    pub flex_basis: Length,
    /// Nonnegative row gap; auto is invalid.
    pub row_gap: Length,
    /// Nonnegative column gap; auto is invalid.
    pub column_gap: Length,
    /// Default cross-axis or grid block alignment.
    pub align_items: Alignment,
    /// Per-item cross-axis or grid block alignment.
    pub align_self: Option<Alignment>,
    /// Default grid inline alignment.
    pub justify_items: Alignment,
    /// Per-item grid inline alignment.
    pub justify_self: Option<Alignment>,
    /// Main-axis or grid inline track distribution.
    pub justify_content: ContentAlignment,
    /// Flex line or grid block track distribution.
    pub align_content: ContentAlignment,
    /// Explicit grid columns.
    pub grid_template_columns: Vec<TrackSize>,
    /// Explicit grid rows.
    pub grid_template_rows: Vec<TrackSize>,
    /// Implicit grid column sizing pattern.
    pub grid_auto_columns: Vec<TrackSize>,
    /// Implicit grid row sizing pattern.
    pub grid_auto_rows: Vec<TrackSize>,
    /// Sequential grid auto-placement order.
    pub grid_auto_flow: GridAutoFlow,
    /// Grid column start placement.
    pub grid_column_start: GridPlacement,
    /// Grid column end placement.
    pub grid_column_end: GridPlacement,
    /// Grid row start placement.
    pub grid_row_start: GridPlacement,
    /// Grid row end placement.
    pub grid_row_end: GridPlacement,
}
impl Default for LayoutStyle {
    fn default() -> Self {
        Self {
            display: Default::default(),
            position: Default::default(),
            width: Default::default(),
            height: Default::default(),
            min_width: Default::default(),
            min_height: Default::default(),
            max_width: Default::default(),
            max_height: Default::default(),
            aspect_ratio: Default::default(),
            inset: Edges::all(Length::Auto),
            margin: Edges::all(Length::Points(0.0)),
            padding: Edges::all(Length::Points(0.0)),
            border_width: 0.0,
            flex_direction: Default::default(),
            flex_wrap: Default::default(),
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Default::default(),
            row_gap: Length::Points(0.0),
            column_gap: Length::Points(0.0),
            align_items: Default::default(),
            align_self: Default::default(),
            justify_items: Default::default(),
            justify_self: Default::default(),
            justify_content: Default::default(),
            align_content: ContentAlignment::Normal,
            grid_template_columns: Default::default(),
            grid_template_rows: Default::default(),
            grid_auto_columns: vec![TrackSize::Auto],
            grid_auto_rows: vec![TrackSize::Auto],
            grid_auto_flow: Default::default(),
            grid_column_start: Default::default(),
            grid_column_end: Default::default(),
            grid_row_start: Default::default(),
            grid_row_end: Default::default(),
        }
    }
}
