use crate::{GridPlacement, LayoutStyle, Length, TrackBreadth, TrackSize};

/// A property value outside the supported finite domain.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid value for {property}")]
pub struct StyleError {
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
    pub fn validate(&self) -> Result<(), StyleError> {
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
                return Err(StyleError { property });
            }
        }
        Ok(())
    }
}

impl crate::Style {
    /// Validates layout and painting properties without changing this style.
    pub fn validate(&self) -> Result<(), StyleError> {
        self.layout.validate()?;
        if !self.opacity.is_finite() || !(0.0..=1.0).contains(&self.opacity) {
            return Err(StyleError {
                property: "opacity",
            });
        }
        if !self
            .border_radii
            .values()
            .into_iter()
            .all(|v| valid_number(v, false))
        {
            return Err(StyleError {
                property: "border_radii",
            });
        }
        for color in [self.background, self.border_color] {
            let [r, g, b, a] = color.to_srgb();
            if ![r, g, b, a].into_iter().all(f32::is_finite) || !(0.0..=1.0).contains(&a) {
                return Err(StyleError { property: "color" });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    #[test]
    fn rejects_nonfinite_and_negative_interior_values() {
        let mut style = Style::default();
        assert!(style.validate().is_ok());
        style.layout.padding.left = Length::Points(-1.0);
        assert!(style.validate().is_err());
        style.layout.padding.left = Length::Points(0.0);
        style.layout.margin.left = Length::Points(-10.0);
        assert!(style.validate().is_ok());
        style.opacity = f64::NAN;
        assert!(style.validate().is_err());
    }
    #[test]
    fn rejects_invalid_grid_ranges_and_zero_spans() {
        let mut style = LayoutStyle {
            grid_template_columns: vec![TrackSize::MinMax(
                TrackBreadth::Fr(1.0),
                TrackBreadth::Auto,
            )],
            ..LayoutStyle::default()
        };
        assert!(style.validate().is_err());
        style.grid_template_columns = vec![TrackSize::MinMax(
            TrackBreadth::Points(10.0),
            TrackBreadth::Fr(1.0),
        )];
        assert!(style.validate().is_ok());
        style.grid_column_start = GridPlacement::Span(0);
        assert!(style.validate().is_err());
    }
}
