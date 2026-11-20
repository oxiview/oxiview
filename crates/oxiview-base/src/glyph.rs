use crate::{Color, Point};
use std::{fmt, sync::Arc};

/// Immutable OpenType font bytes shared by shaped glyphs and retained scenes.
#[derive(Clone)]
pub struct GlyphFont(Arc<dyn AsRef<[u8]> + Send + Sync>);

impl GlyphFont {
    /// Takes ownership of the shared font file. Font parsing is adapter-specific.
    pub fn new(bytes: Arc<[u8]>) -> Self {
        Self(Arc::new(bytes))
    }

    /// Shares an existing immutable font allocation, including a memory-mapped file.
    pub fn from_shared(bytes: Arc<dyn AsRef<[u8]> + Send + Sync>) -> Self {
        Self(bytes)
    }

    /// Returns the complete font file, which can contain multiple faces.
    pub fn bytes(&self) -> &[u8] {
        self.0.as_ref().as_ref()
    }
}

impl AsRef<[u8]> for GlyphFont {
    fn as_ref(&self) -> &[u8] {
        self.bytes()
    }
}

impl fmt::Debug for GlyphFont {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GlyphFont")
            .field("byte_len", &self.bytes().len())
            .finish()
    }
}

impl PartialEq for GlyphFont {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.bytes() == other.bytes()
    }
}

impl Eq for GlyphFont {}

/// Presentation selected for a shaped glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphKind {
    /// Unhinted monochrome outline.
    Outline,
    /// OpenType COLR paint graph using palette zero.
    Color,
}

/// Positioned glyph in logical coordinates, independent of the renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    /// Immutable backing font file.
    pub font: GlyphFont,
    /// OpenType collection index, or zero for a standalone face.
    pub face_index: u32,
    /// Glyph identifier within the face.
    pub glyph_id: u16,
    /// Logical pixels per em.
    pub size: f32,
    /// Local y-down baseline position, including adjustments from shaping.
    pub origin: Point,
    /// Outline color or COLR foreground. For COLR, alpha applies to the composed glyph.
    pub color: Color,
    /// Normalized variation coordinates in font axis order, stored as signed F2Dot14.
    pub variations: Arc<[i16]>,
    /// Representation to draw.
    pub kind: GlyphKind,
}
