use std::sync::Arc;

/// Shared integer image dimensions in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelSize {
    /// Width in physical pixels; each adapter validates its own limits.
    pub width: u32,
    /// Height in physical pixels; zero dimensions can represent suspension.
    pub height: u32,
}

/// Owned row-major, straight (non-premultiplied) sRGB RGBA8 pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    size: PixelSize,
    pixels: Arc<[u8]>,
}
impl RgbaImage {
    /// Validates byte length and normalizes transparent pixels to zero RGB.
    pub fn new(size: PixelSize, mut pixels: Vec<u8>) -> Result<Self, crate::Error> {
        let len = u64::from(size.width)
            .checked_mul(u64::from(size.height))
            .and_then(|n| n.checked_mul(4));
        if len != Some(pixels.len() as u64) {
            return Err(crate::Error::InvalidImage);
        }
        for pixel in pixels.chunks_exact_mut(4) {
            if let [r, g, b, 0] = pixel {
                *r = 0;
                *g = 0;
                *b = 0;
            }
        }
        Ok(Self {
            size,
            pixels: pixels.into(),
        })
    }

    /// Output width in pixels.
    pub fn width(&self) -> u32 {
        self.size.width
    }
    /// Output height in pixels.
    pub fn height(&self) -> u32 {
        self.size.height
    }
    /// Straight RGBA8 bytes, with transparent pixels normalized to zero RGB.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}
