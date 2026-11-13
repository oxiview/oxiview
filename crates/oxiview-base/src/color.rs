use peniko::color::{AlphaColor, LinearSrgb, Srgb};

/// Straight-alpha color carrying both sRGB and linear-sRGB representations.
///
/// Floating-point channels are not clamped, preserving extended gamut values.
/// Alpha is linear and unchanged by color-space conversion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    srgb: AlphaColor<Srgb>,
    linear: AlphaColor<LinearSrgb>,
}
impl Color {
    /// Creates a color from red, green, blue, and alpha sRGB bytes.
    pub fn from_rgba8(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self::from_srgb([red, green, blue, alpha].map(|channel| f32::from(channel) / 255.0))
    }
    /// Creates a color from straight sRGB RGBA channels.
    pub fn from_srgb(rgba: [f32; 4]) -> Self {
        let srgb = AlphaColor::new(rgba);
        Self {
            srgb,
            linear: srgb.convert(),
        }
    }
    /// Creates a color from straight linear-sRGB RGBA channels.
    pub fn from_linear(rgba: [f32; 4]) -> Self {
        let linear = AlphaColor::new(rgba);
        Self {
            srgb: linear.convert(),
            linear,
        }
    }
    /// Returns straight sRGB RGBA channels.
    pub fn to_srgb(self) -> [f32; 4] {
        self.srgb.components
    }
    /// Returns straight linear-sRGB RGBA channels.
    pub fn to_linear(self) -> [f32; 4] {
        self.linear.components
    }
}
