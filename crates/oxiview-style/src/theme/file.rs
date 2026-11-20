use super::{RingRule, Rule, StateVariant, Theme, ThemeError, Tokens, Value, WidgetRules};
use crate::patch::{valid_font_size, valid_number};
use oxiview_base::Color;
use std::collections::BTreeMap;
use toml::{Table, Value as Toml};

pub(super) fn parse(source: &str) -> Result<Theme, ThemeError> {
    let mut document = source
        .parse::<Table>()
        .map_err(|error| ThemeError::new("theme", error.to_string()))?;
    let version = document.remove("version").and_then(|v| v.as_integer());
    if version != Some(1) {
        return Err(ThemeError::new(
            "version",
            "expected theme format version 1",
        ));
    }
    let base = tokens(
        take_table(&mut document, "tokens", "tokens")?,
        "tokens",
        None,
    )?;
    let dark = tokens(
        take_table(&mut document, "dark", "dark")?,
        "dark",
        Some(&base),
    )?;
    let high_contrast = tokens(
        take_table(&mut document, "high_contrast", "high_contrast")?,
        "high_contrast",
        Some(&base),
    )?;
    let rules = take_table(&mut document, "rules", "rules")?
        .into_iter()
        .map(|(widget, table)| {
            let key = format!("rules.{widget}");
            if !valid_name(&widget) {
                return Err(ThemeError::new(key, "invalid widget type"));
            }
            Ok((widget, widget_rules(into_table(table, &key)?, &key, &base)?))
        })
        .collect::<Result<_, _>>()?;
    no_unknown_keys(document, "")?;
    Ok(Theme {
        dark,
        high_contrast,
        rules,
    })
}

fn tokens(table: Table, prefix: &str, base: Option<&Tokens>) -> Result<Tokens, ThemeError> {
    let mut result = Tokens::default();
    for (group, values) in table {
        let path = format!("{prefix}.{group}");
        match group.as_str() {
            "colors" => {
                result.colors = token_set(values, &path, base.map(|tokens| &tokens.colors), color)?;
            }
            "spacing" => {
                result.spacing =
                    token_set(values, &path, base.map(|tokens| &tokens.spacing), length)?;
            }
            "radii" => {
                result.radii = token_set(values, &path, base.map(|tokens| &tokens.radii), length)?;
            }
            "font_sizes" => {
                result.font_sizes = token_set(
                    values,
                    &path,
                    base.map(|tokens| &tokens.font_sizes),
                    font_size,
                )?;
            }
            _ => return Err(ThemeError::new(path, "unknown token category")),
        }
    }
    Ok(result)
}

fn token_set<T>(
    table: Toml,
    prefix: &str,
    base: Option<&BTreeMap<String, T>>,
    parse: impl Fn(Toml, &str) -> Result<T, ThemeError>,
) -> Result<BTreeMap<String, T>, ThemeError> {
    into_table(table, prefix)?
        .into_iter()
        .map(|(name, value)| {
            let key = format!("{prefix}.{name}");
            if !valid_name(&name) {
                return Err(ThemeError::new(key, "invalid token name"));
            }
            if base.is_some_and(|tokens| !tokens.contains_key(&name)) {
                return Err(ThemeError::new(
                    key,
                    "palette or contrast override requires a base token of the same type",
                ));
            }
            Ok((name, parse(value, &key)?))
        })
        .collect()
}

fn widget_rules(
    mut table: Table,
    prefix: &str,
    tokens: &Tokens,
) -> Result<WidgetRules, ThemeError> {
    let mut rules = WidgetRules::default();
    for (name, state) in [
        ("hovered", StateVariant::Hovered),
        ("focused", StateVariant::Focused),
        ("pressed", StateVariant::Pressed),
        ("disabled", StateVariant::Disabled),
    ] {
        if let Some(variant) = table.remove(name) {
            let key = format!("{prefix}.{name}");
            rules.variants[state as usize] = rule(into_table(variant, &key)?, &key, tokens)?;
        }
    }
    rules.base = rule(table, prefix, tokens)?;
    Ok(rules)
}

fn rule(table: Table, prefix: &str, tokens: &Tokens) -> Result<Rule, ThemeError> {
    let mut rule = Rule::default();
    for (property, value) in table {
        let key = format!("{prefix}.{property}");
        match property.as_str() {
            "background" => {
                rule.background = Some(reference(value, &key, &tokens.colors, "color", color)?)
            }
            "border_color" => {
                rule.border_color = Some(reference(value, &key, &tokens.colors, "color", color)?)
            }
            "border_radii" => {
                rule.border_radii = Some(reference(value, &key, &tokens.radii, "radius", length)?)
            }
            "padding" => {
                rule.padding = Some(reference(value, &key, &tokens.spacing, "spacing", length)?)
            }
            "opacity" => {
                rule.opacity = Some(number(
                    value,
                    &key,
                    |opacity| (0.0..=1.0).contains(&opacity),
                    "expected an opacity between zero and one",
                )?)
            }
            "focus_ring" => rule.focus_ring = Some(focus_ring(value, &key, tokens)?),
            "text_size" => {
                rule.text_size = Some(reference(
                    value,
                    &key,
                    &tokens.font_sizes,
                    "font size",
                    font_size,
                )?)
            }
            "text_color" => {
                rule.text_color = Some(reference(value, &key, &tokens.colors, "color", color)?)
            }
            _ => return Err(ThemeError::new(key, "unknown theme rule property")),
        }
    }
    Ok(rule)
}

fn focus_ring(value: Toml, prefix: &str, tokens: &Tokens) -> Result<RingRule, ThemeError> {
    let mut table = into_table(value, prefix)?;
    let color_key = format!("{prefix}.color");
    let width_key = format!("{prefix}.width");
    let ring = RingRule {
        color: reference(
            required(&mut table, "color", &color_key)?,
            &color_key,
            &tokens.colors,
            "color",
            color,
        )?,
        width: reference(
            required(&mut table, "width", &width_key)?,
            &width_key,
            &tokens.spacing,
            "spacing",
            length,
        )?,
    };
    no_unknown_keys(table, prefix)?;
    Ok(ring)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn reference<T: Copy>(
    value: Toml,
    key: &str,
    tokens: &BTreeMap<String, T>,
    kind: &str,
    parse: impl FnOnce(Toml, &str) -> Result<T, ThemeError>,
) -> Result<Value<T>, ThemeError> {
    if let Some(name) = value.as_str().and_then(|text| text.strip_prefix('@')) {
        let base = tokens
            .get(name)
            .copied()
            .ok_or_else(|| ThemeError::new(key, format!("unknown {kind} token @{name}")))?;
        return Ok(Value::Token {
            name: name.to_owned(),
            base,
        });
    }
    parse(value, key).map(Value::Literal)
}

fn color(value: Toml, key: &str) -> Result<Color, ThemeError> {
    let invalid = || ThemeError::new(key, "expected a #RRGGBB or #RRGGBBAA color");
    let hex = value
        .as_str()
        .and_then(|text| text.strip_prefix('#'))
        .ok_or_else(invalid)?;
    if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    let encoded = u32::from_str_radix(hex, 16).map_err(|_| invalid())?;
    let [red, green, blue, alpha] = if hex.len() == 6 {
        (encoded << 8 | 255).to_be_bytes()
    } else {
        encoded.to_be_bytes()
    };
    Ok(Color::from_rgba8(red, green, blue, alpha))
}

fn number(
    value: Toml,
    key: &str,
    valid: fn(f64) -> bool,
    description: &str,
) -> Result<f64, ThemeError> {
    let number = match value {
        Toml::Integer(value) => Some(value as f64),
        Toml::Float(value) => Some(value),
        _ => None,
    };
    number
        .filter(|value| valid(*value))
        .ok_or_else(|| ThemeError::new(key, description))
}

fn length(value: Toml, key: &str) -> Result<f64, ThemeError> {
    number(
        value,
        key,
        valid_number,
        "expected a finite nonnegative logical length",
    )
}

fn font_size(value: Toml, key: &str) -> Result<f64, ThemeError> {
    number(
        value,
        key,
        valid_font_size,
        "expected a positive logical font size up to 1024",
    )
}

fn into_table(value: Toml, key: &str) -> Result<Table, ThemeError> {
    match value {
        Toml::Table(table) => Ok(table),
        _ => Err(ThemeError::new(key, "expected a table")),
    }
}

fn take_table(table: &mut Table, property: &str, key: &str) -> Result<Table, ThemeError> {
    table
        .remove(property)
        .map_or_else(|| Ok(Table::new()), |value| into_table(value, key))
}

fn required(table: &mut Table, property: &str, key: &str) -> Result<Toml, ThemeError> {
    table
        .remove(property)
        .ok_or_else(|| ThemeError::new(key, "required property is missing"))
}

fn no_unknown_keys(table: Table, prefix: &str) -> Result<(), ThemeError> {
    if let Some((key, _)) = table.into_iter().next() {
        let key = if prefix.is_empty() {
            key
        } else {
            format!("{prefix}.{key}")
        };
        return Err(ThemeError::new(key, "unknown theme key"));
    }
    Ok(())
}
