//! Temporary one-image window port for the platform context.
//!
//! M1 replaces this placeholder with the real platform trait. No API here is settled.

/// Borrowed, tightly packed, top-to-bottom straight sRGB RGBA8 pixels.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    /// Width in pixels; must be nonzero.
    pub width: u32,
    /// Height in pixels; must be nonzero.
    pub height: u32,
    /// Exactly `width * height * 4` bytes, with no row padding.
    pub pixels: &'a [u8],
}

/// Errors opening or presenting a single image.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Dimensions or byte length do not describe a supported RGBA8 frame.
    #[error("invalid RGBA8 frame dimensions or byte length")]
    InvalidFrame,
    /// AppKit presentation must run on the process main thread.
    #[error("window presentation requires the main thread")]
    MainThreadRequired,
    /// The native image could not be allocated.
    #[error("native bitmap allocation failed")]
    BitmapAllocation,
    /// This adapter is unavailable on the current operating system.
    #[error("this window adapter requires macOS")]
    UnsupportedPlatform,
}

/// Placeholder for M0's single-image viewer; M1 replaces it with the real
/// platform trait. Nothing here is settled API.
pub trait WindowPresenter {
    /// Opens one window at the frame's dimensions, blits the image, and blocks
    /// until the user closes it. Implementations validate the frame before use.
    /// No input, scheduling, or display-scale policy is exposed.
    fn present(&self, frame: Frame<'_>) -> Result<(), Error>;
}
