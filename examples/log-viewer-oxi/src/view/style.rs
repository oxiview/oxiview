//! Palette, typography, and layout shared by the shell and record views.

use super::Ui;
use crate::model::Severity;
use oxiview_base::Color;
use oxiview_core::{TextStyle, View, text};
use oxiview_style::{Edges, LayoutStyle, Length, Overflow, Style};

pub(super) const INK: u32 = 0x253532;
pub(super) const MUTED: u32 = 0x526561;
pub(super) const ACCENT: u32 = 0x286b64;
pub(super) const WHITE: u32 = 0xffffff;
pub(super) const LINE: u32 = 0xd6e0dd;
pub(super) const SOFT: u32 = 0xf1f6f4;

pub(super) fn color(rgb: u32) -> Color {
    let [_, red, green, blue] = rgb.to_be_bytes();
    Color::from_rgba8(red, green, blue, 255)
}

pub(super) fn px(n: f64) -> Length {
    Length::Points(n)
}

pub(super) fn pad(x: f64, y: f64) -> Edges<Length> {
    Edges {
        left: px(x),
        right: px(x),
        top: px(y),
        bottom: px(y),
    }
}

pub(super) fn fill() -> LayoutStyle {
    LayoutStyle {
        flex_basis: px(0.),
        flex_grow: 1.,
        min_width: px(0.),
        min_height: px(0.),
        ..Default::default()
    }
}

pub(super) fn typeface(size: f64, weight: u16, ink: u32) -> TextStyle {
    TextStyle {
        font_size: size,
        weight,
        color: color(ink),
        family: Some("Noto Sans, system-ui".into()),
        ..Default::default()
    }
}

pub(super) fn label(
    ui: &Ui,
    value: impl Into<String>,
    font: TextStyle,
    layout: LayoutStyle,
) -> View {
    text(ui.text.clone(), value, font).style(Style {
        layout,
        hit_test: false,
        overflow: Overflow::Clip,
        ..Default::default()
    })
}

pub(super) fn caption(ui: &Ui, value: impl Into<String>, width: Length, ink: u32) -> View {
    label(
        ui,
        value,
        typeface(11., 630, ink),
        LayoutStyle {
            width,
            height: px(16.),
            ..Default::default()
        },
    )
}

pub(super) fn severity_ink(level: Severity) -> u32 {
    match level {
        Severity::Trace => 0x596b68,
        Severity::Debug => 0x506681,
        Severity::Info => 0x24665e,
        Severity::Warn => 0x8a5917,
        Severity::Error => 0xa1363c,
    }
}
