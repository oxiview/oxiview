//! M0 compatibility API, removed in M1 S5.

use crate::Error;

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

/// Placeholder for M0's single-image viewer; M1 replaces it with the real
/// platform trait. Removed in M1 S5; nothing here is settled API.
pub trait WindowPresenter {
    /// Opens one window at the frame's dimensions, blits the image, and blocks
    /// until the user closes it. Implementations validate the frame before use.
    /// Call on the main thread.
    /// No input, scheduling, or display-scale policy is exposed.
    fn present(&self, frame: Frame<'_>) -> Result<(), Error>;
}
