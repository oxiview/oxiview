use crate::{
    Alignment, ContentAlignment, Display, Edges, FlexDirection, FlexWrap, GridAutoFlow,
    GridPlacement, LayoutStyle, Length, Overflow, Position, Style, StyleError, TrackSize,
};
use oxiview_base::{Color, CornerRadii};

/// Built-in state flags for a retained control.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct InteractionState {
    /// The node participates in the effective hover route.
    pub hovered: bool,
    /// An activation gesture is armed on the node.
    pub pressed: bool,
    /// The node has active keyboard-visible focus.
    pub focused: bool,
    /// The node or an ancestor disables interaction.
    pub disabled: bool,
}

/// A declaration that applies during one built-in interaction state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateVariant {
    /// The control is hovered.
    Hovered,
    /// The control has visible keyboard focus.
    Focused,
    /// The control is pressed.
    Pressed,
    /// The control is disabled.
    Disabled,
}

impl StateVariant {
    /// State variants in increasing priority within one declaration layer.
    pub const PRIORITY: [Self; 4] = [Self::Hovered, Self::Focused, Self::Pressed, Self::Disabled];

    /// Whether this variant applies to the combined interaction state.
    pub const fn applies(self, state: InteractionState) -> bool {
        match self {
            Self::Hovered => state.hovered,
            Self::Focused => state.focused,
            Self::Pressed => state.pressed,
            Self::Disabled => state.disabled,
        }
    }
}

/// A node-local paint indicator independent of its layout border.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusRing {
    /// Solid stroke color.
    pub color: Color,
    /// Inset stroke width in logical units. Zero suppresses the stroke.
    pub width: f64,
}

macro_rules! layout_patch {
    ($( $(#[$docs:meta])* $field:ident: $kind:ty ),+ $(,)?) => {
        /// Sparse layout declarations. `None` leaves a property unspecified.
        #[derive(Clone, Debug, Default, PartialEq)]
        pub struct LayoutPatch {
            $( $(#[$docs])* pub $field: Option<$kind>, )+
        }

        impl LayoutPatch {
            fn overlay(&mut self, later: Self) {
                $(if later.$field.is_some() { self.$field = later.$field; })+
            }

            fn apply_to(&self, style: &mut LayoutStyle) {
                $(if let Some(value) = &self.$field { style.$field = value.clone(); })+
            }
        }

        impl From<LayoutStyle> for LayoutPatch {
            fn from(style: LayoutStyle) -> Self {
                Self { $( $field: Some(style.$field), )+ }
            }
        }
    };
}

layout_patch! {
    /// Layout family.
    display: Display,
    /// Positioning mode.
    position: Position,
    /// Preferred border-box width.
    width: Length,
    /// Preferred border-box height.
    height: Length,
    /// Minimum border-box width.
    min_width: Length,
    /// Minimum border-box height.
    min_height: Length,
    /// Maximum border-box width.
    max_width: Length,
    /// Maximum border-box height.
    max_height: Length,
    /// Optional width-to-height ratio. `Some(None)` clears an earlier ratio.
    aspect_ratio: Option<f64>,
    /// Offsets for positioned nodes.
    inset: Edges<Length>,
    /// Outside spacing.
    margin: Edges<Length>,
    /// Nonnegative interior spacing.
    padding: Edges<Length>,
    /// Uniform nonnegative logical border width.
    border_width: f64,
    /// Flex main-axis direction.
    flex_direction: FlexDirection,
    /// Flex wrapping policy.
    flex_wrap: FlexWrap,
    /// Nonnegative flex growth factor.
    flex_grow: f64,
    /// Nonnegative flex shrink factor.
    flex_shrink: f64,
    /// Initial flex main size.
    flex_basis: Length,
    /// Nonnegative row gap.
    row_gap: Length,
    /// Nonnegative column gap.
    column_gap: Length,
    /// Default cross-axis or grid block alignment.
    align_items: Alignment,
    /// Per-item cross-axis or grid block alignment.
    align_self: Option<Alignment>,
    /// Default grid inline alignment.
    justify_items: Alignment,
    /// Per-item grid inline alignment.
    justify_self: Option<Alignment>,
    /// Main-axis or grid inline track distribution.
    justify_content: ContentAlignment,
    /// Flex line or grid block track distribution.
    align_content: ContentAlignment,
    /// Explicit grid columns.
    grid_template_columns: Vec<TrackSize>,
    /// Explicit grid rows.
    grid_template_rows: Vec<TrackSize>,
    /// Implicit grid column sizing pattern.
    grid_auto_columns: Vec<TrackSize>,
    /// Implicit grid row sizing pattern.
    grid_auto_rows: Vec<TrackSize>,
    /// Sequential grid auto-placement direction.
    grid_auto_flow: GridAutoFlow,
    /// Horizontal starting grid line or span.
    grid_column_start: GridPlacement,
    /// Horizontal ending grid line or span.
    grid_column_end: GridPlacement,
    /// Vertical starting grid line or span.
    grid_row_start: GridPlacement,
    /// Vertical ending grid line or span.
    grid_row_end: GridPlacement,
}

/// Sparse, typed local declarations. Explicit defaults are ordinary values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StylePatch {
    /// Layout declarations.
    pub layout: LayoutPatch,
    /// Solid background color.
    pub background: Option<Color>,
    /// Solid border color.
    pub border_color: Option<Color>,
    /// Circular corner radii.
    pub border_radii: Option<CornerRadii>,
    /// Group opacity between zero and one.
    pub opacity: Option<f64>,
    /// Stacking level. `Some(None)` restores automatic stacking.
    pub z_index: Option<Option<i32>>,
    /// Overflow clipping behavior.
    pub overflow: Option<Overflow>,
    /// Whether this node participates in geometric hit selection.
    pub hit_test: Option<bool>,
    /// Explicit opt-in to hover updates under a stationary pointer.
    pub stationary_hover_updates: Option<bool>,
    /// Optional independent focus ring paint.
    pub focus_ring: Option<FocusRing>,
    /// Logical paragraph font size for a text node.
    pub text_size: Option<f64>,
    /// Paragraph foreground color for a text node.
    pub text_color: Option<Color>,
}

impl StylePatch {
    /// Layers later declarations over these declarations by property.
    pub fn overlay(&mut self, later: Self) {
        self.layout.overlay(later.layout);
        self.background = later.background.or(self.background);
        self.border_color = later.border_color.or(self.border_color);
        self.border_radii = later.border_radii.or(self.border_radii);
        self.opacity = later.opacity.or(self.opacity);
        self.z_index = later.z_index.or(self.z_index);
        self.overflow = later.overflow.or(self.overflow);
        self.hit_test = later.hit_test.or(self.hit_test);
        self.stationary_hover_updates = later
            .stationary_hover_updates
            .or(self.stationary_hover_updates);
        self.focus_ring = later.focus_ring.or(self.focus_ring);
        self.text_size = later.text_size.or(self.text_size);
        self.text_color = later.text_color.or(self.text_color);
    }

    /// Applies declarations represented by [`Style`] to an existing style.
    pub fn apply_to(&self, style: &mut Style) {
        self.layout.apply_to(&mut style.layout);
        style.background = self.background.unwrap_or(style.background);
        style.border_color = self.border_color.unwrap_or(style.border_color);
        style.border_radii = self.border_radii.unwrap_or(style.border_radii);
        style.opacity = self.opacity.unwrap_or(style.opacity);
        style.z_index = self.z_index.unwrap_or(style.z_index);
        style.overflow = self.overflow.unwrap_or(style.overflow);
        style.hit_test = self.hit_test.unwrap_or(style.hit_test);
        style.stationary_hover_updates = self
            .stationary_hover_updates
            .unwrap_or(style.stationary_hover_updates);
    }

    /// Checks every specified property against its typed domain.
    pub fn validate(&self) -> Result<(), StyleError> {
        let mut style = Style::default();
        self.apply_to(&mut style);
        style.validate()?;
        if let Some(ring) = self.focus_ring {
            if !valid_number(ring.width) {
                return Err(StyleError {
                    property: "focus_ring.width",
                });
            }
            if !valid_color(ring.color) {
                return Err(StyleError {
                    property: "focus_ring.color",
                });
            }
        }
        if self.text_size.is_some_and(|value| !valid_font_size(value)) {
            return Err(StyleError {
                property: "text_size",
            });
        }
        if self.text_color.is_some_and(|value| !valid_color(value)) {
            return Err(StyleError {
                property: "text_color",
            });
        }
        Ok(())
    }
}

pub(super) fn valid_number(value: f64) -> bool {
    value.is_finite() && (0.0..=f64::from(f32::MAX)).contains(&value)
}

pub(super) fn valid_font_size(value: f64) -> bool {
    value as f32 > 0.0 && value <= 1024.0
}

fn valid_color(value: Color) -> bool {
    let [r, g, b, a] = value.to_srgb();
    [r, g, b, a].into_iter().all(f32::is_finite) && (0.0..=1.0).contains(&a)
}
