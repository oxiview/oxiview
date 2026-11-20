//! Typed UI properties for the styling port. Values remain framework-owned.

mod patch;
mod theme;
mod types;
use oxiview_base::{Color, CornerRadii};
pub use patch::{FocusRing, InteractionState, LayoutPatch, StateVariant, StylePatch};
pub use theme::{AppearanceMode, Theme, ThemeEnvironment, ThemeError, ThemeStyle};
pub use types::*;

/// Whether children may paint and hit outside the padding box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Overflow {
    /// Descendants may extend beyond this node.
    #[default]
    Visible,
    /// Clip descendants to the rounded padding box.
    Clip,
}

/// Typed style for one retained UI node.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    /// Layout-related properties.
    pub layout: LayoutStyle,
    /// Solid background, including alpha.
    pub background: Color,
    /// Solid border color.
    pub border_color: Color,
    /// Circular corner radii in clockwise order.
    pub border_radii: CornerRadii,
    /// Group opacity between zero and one.
    pub opacity: f64,
    /// Optional stacking level; none is auto.
    pub z_index: Option<i32>,
    /// Overflow clipping behavior.
    pub overflow: Overflow,
    /// Whether this node participates in geometric hit selection.
    pub hit_test: bool,
    /// Explicit opt-in to hover updates under a stationary pointer.
    pub stationary_hover_updates: bool,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            layout: LayoutStyle::default(),
            background: Color::from_rgba8(0, 0, 0, 0),
            border_color: Color::from_rgba8(0, 0, 0, 255),
            border_radii: CornerRadii::default(),
            opacity: 1.0,
            z_index: None,
            overflow: Overflow::Visible,
            hit_test: true,
            stationary_hover_updates: false,
        }
    }
}

mod validation;
pub use validation::StyleError;
