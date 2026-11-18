use crate::{GridPlacement, LayoutStyle, Length, TrackBreadth, TrackSize};

/// A property value outside the supported finite domain.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid value for {property}")]
pub struct InputError {
    /// The rejected property category.
    pub property: &'static str,
}
fn valid_number(value: f64, negative: bool) -> bool {
    value.is_finite() && value.abs() <= f64::from(f32::MAX) && (negative || value >= 0.0)
}
fn length(value: Length, negative: bool, auto: bool) -> bool {
    match value {
        Length::Auto => auto,
        Length::Points(v) | Length::Percent(v) => valid_number(v, negative),
    }
}
fn breadth(value: TrackBreadth, flexible: bool) -> bool {
    match value {
        TrackBreadth::Auto => true,
        TrackBreadth::Points(v) | TrackBreadth::Percent(v) => valid_number(v, false),
        TrackBreadth::Fr(v) => flexible && valid_number(v, false),
    }
}
fn track(value: TrackSize) -> bool {
    match value {
        TrackSize::Auto => true,
        TrackSize::Points(v) | TrackSize::Percent(v) | TrackSize::Fr(v) => valid_number(v, false),
        TrackSize::MinMax(min, max) => breadth(min, false) && breadth(max, true),
    }
}
impl LayoutStyle {
    /// Validates values without modifying or clamping them.
    pub fn validate(&self) -> Result<(), InputError> {
        let checks = [
            (
                "dimensions",
                [
                    self.width,
                    self.height,
                    self.min_width,
                    self.min_height,
                    self.max_width,
                    self.max_height,
                    self.flex_basis,
                ]
                .into_iter()
                .all(|v| length(v, false, true)),
            ),
            (
                "margin",
                self.margin
                    .values()
                    .into_iter()
                    .all(|v| length(v, true, true)),
            ),
            (
                "inset",
                self.inset
                    .values()
                    .into_iter()
                    .all(|v| length(v, true, true)),
            ),
            (
                "padding",
                self.padding
                    .values()
                    .into_iter()
                    .all(|v| length(v, false, false)),
            ),
            (
                "gap",
                [self.row_gap, self.column_gap]
                    .into_iter()
                    .all(|v| length(v, false, false)),
            ),
            ("border_width", valid_number(self.border_width, false)),
            (
                "flex_factor",
                [self.flex_grow, self.flex_shrink]
                    .into_iter()
                    .all(|v| valid_number(v, false)),
            ),
            (
                "aspect_ratio",
                self.aspect_ratio
                    .is_none_or(|v| valid_number(v, false) && (v as f32) > 0.0),
            ),
            (
                "grid_tracks",
                [
                    &self.grid_template_columns,
                    &self.grid_template_rows,
                    &self.grid_auto_columns,
                    &self.grid_auto_rows,
                ]
                .into_iter()
                .flatten()
                .copied()
                .all(track),
            ),
            (
                "grid_placement",
                [
                    self.grid_column_start,
                    self.grid_column_end,
                    self.grid_row_start,
                    self.grid_row_end,
                ]
                .into_iter()
                .all(|v| match v {
                    GridPlacement::Auto => true,
                    GridPlacement::Line(v) => v != 0,
                    GridPlacement::Span(v) => v != 0,
                }),
            ),
        ];
        for (property, valid) in checks {
            if !valid {
                return Err(InputError { property });
            }
        }
        Ok(())
    }
}
