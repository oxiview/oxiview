use crate::{Edges, FocusRing, InteractionState, Length, StateVariant, Style};
use oxiview_base::{Color, CornerRadii};
use std::collections::BTreeMap;

mod file;

/// The application's choice of palette for a window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AppearanceMode {
    /// Follow the system appearance. An unavailable system appearance uses light.
    #[default]
    System,
    /// Always use the light palette.
    Light,
    /// Always use the dark palette.
    Dark,
}

/// Effective window preferences used when evaluating theme rules.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ThemeEnvironment {
    /// Whether the active palette is dark.
    pub dark: bool,
    /// Whether the system requests increased contrast.
    pub increased_contrast: bool,
    /// System accent, if the backend supports it.
    pub accent_color: Option<Color>,
}

/// An invalid theme document. The key is the path of the offending declaration.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{key}: {message}")]
pub struct ThemeError {
    key: String,
    message: String,
}

impl ThemeError {
    /// Dotted key associated with the error, or `theme` for a document syntax error.
    pub fn key(&self) -> &str {
        &self.key
    }

    fn new(key: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            message: message.into(),
        }
    }
}

/// Resolved declarations offered by the matching theme rules.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ThemeStyle {
    /// Solid background color.
    pub background: Option<Color>,
    /// Solid border color.
    pub border_color: Option<Color>,
    /// Circular corner radii.
    pub border_radii: Option<CornerRadii>,
    /// Interior spacing.
    pub padding: Option<Edges<Length>>,
    /// Group opacity between zero and one.
    pub opacity: Option<f64>,
    /// Optional independent focus ring paint.
    pub focus_ring: Option<FocusRing>,
    /// Logical paragraph font size.
    pub text_size: Option<f64>,
    /// Paragraph foreground color.
    pub text_color: Option<Color>,
}

impl ThemeStyle {
    /// Applies the declarations represented by [`Style`] as defaults for a node.
    pub fn apply_to(self, style: &mut Style) {
        style.background = self.background.unwrap_or(style.background);
        style.border_color = self.border_color.unwrap_or(style.border_color);
        style.border_radii = self.border_radii.unwrap_or(style.border_radii);
        style.layout.padding = self.padding.unwrap_or(style.layout.padding);
        style.opacity = self.opacity.unwrap_or(style.opacity);
    }
}

/// A validated versioned theme containing typed tokens and widget-type rules.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Theme {
    dark: Tokens,
    high_contrast: Tokens,
    rules: BTreeMap<String, WidgetRules>,
}

impl Theme {
    /// Parses an entire version 1 theme and validates token references and rule values.
    ///
    /// ```
    /// use oxiview_style::{InteractionState, Theme, ThemeEnvironment};
    ///
    /// let theme = Theme::from_toml(r##"
    /// version = 1
    /// [tokens.colors]
    /// surface = "#cbd4f1"
    /// [rules.button]
    /// background = "@surface"
    /// "##)?;
    /// let style = theme.resolve("button", InteractionState::default(), ThemeEnvironment::default());
    /// assert!(style.background.is_some());
    /// # Ok::<(), oxiview_style::ThemeError>(())
    /// ```
    pub fn from_toml(source: &str) -> Result<Self, ThemeError> {
        file::parse(source)
    }

    /// Resolves the widget rule and its active state variants in ascending priority.
    pub fn resolve(
        &self,
        widget: &str,
        state: InteractionState,
        environment: ThemeEnvironment,
    ) -> ThemeStyle {
        let mut style = ThemeStyle::default();
        if let Some(rules) = self.rules.get(widget) {
            self.apply(&rules.base, environment, &mut style);
            for variant in StateVariant::PRIORITY {
                if variant.applies(state) {
                    self.apply(&rules.variants[variant as usize], environment, &mut style);
                }
            }
        }
        style
    }

    fn apply(&self, rule: &Rule, environment: ThemeEnvironment, style: &mut ThemeStyle) {
        if let Some(color) = &rule.background {
            style.background = Some(self.color(color, environment));
        }
        if let Some(color) = &rule.border_color {
            style.border_color = Some(self.color(color, environment));
        }
        if let Some(radius) = &rule.border_radii {
            let radius = self.token(radius, environment, |tokens| &tokens.radii);
            style.border_radii = Some(CornerRadii::uniform(radius));
        }
        if let Some(padding) = &rule.padding {
            let padding = self.token(padding, environment, |tokens| &tokens.spacing);
            style.padding = Some(Edges::all(Length::Points(padding)));
        }
        style.opacity = rule.opacity.or(style.opacity);
        if let Some(ring) = &rule.focus_ring {
            style.focus_ring = Some(FocusRing {
                color: self.color(&ring.color, environment),
                width: self.token(&ring.width, environment, |tokens| &tokens.spacing),
            });
        }
        if let Some(size) = &rule.text_size {
            style.text_size = Some(self.token(size, environment, |tokens| &tokens.font_sizes));
        }
        if let Some(color) = &rule.text_color {
            style.text_color = Some(self.color(color, environment));
        }
    }

    fn token<T: Copy>(
        &self,
        value: &Value<T>,
        environment: ThemeEnvironment,
        map: impl Fn(&Tokens) -> &BTreeMap<String, T>,
    ) -> T {
        match value {
            Value::Literal(value) => *value,
            Value::Token { name, base } => {
                if environment.increased_contrast
                    && let Some(value) = map(&self.high_contrast).get(name)
                {
                    return *value;
                }
                if environment.dark
                    && let Some(value) = map(&self.dark).get(name)
                {
                    return *value;
                }
                *base
            }
        }
    }

    fn color(&self, value: &Value<Color>, environment: ThemeEnvironment) -> Color {
        if let Value::Token { name, .. } = value
            && name == "system_accent"
            && let Some(accent) = environment.accent_color
            && !(environment.increased_contrast && self.high_contrast.colors.contains_key(name))
        {
            return accent;
        }
        self.token(value, environment, |tokens| &tokens.colors)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Tokens {
    colors: BTreeMap<String, Color>,
    spacing: BTreeMap<String, f64>,
    radii: BTreeMap<String, f64>,
    font_sizes: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, PartialEq)]
enum Value<T> {
    Literal(T),
    Token { name: String, base: T },
}

#[derive(Clone, Debug, PartialEq)]
struct RingRule {
    color: Value<Color>,
    width: Value<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Rule {
    background: Option<Value<Color>>,
    border_color: Option<Value<Color>>,
    border_radii: Option<Value<f64>>,
    padding: Option<Value<f64>>,
    opacity: Option<f64>,
    focus_ring: Option<RingRule>,
    text_size: Option<Value<f64>>,
    text_color: Option<Value<Color>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct WidgetRules {
    base: Rule,
    variants: [Rule; 4],
}
