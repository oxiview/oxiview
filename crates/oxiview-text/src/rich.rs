use crate::Error;
use oxiview_base::Color;
use std::{ops::Range, sync::Arc};

/// Overrides applied to a span. Unspecified properties inherit the paragraph's
/// [`TextStyle`](crate::TextStyle) and the default spacing and decoration.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextSpanStyle {
    /// Preferred font family or comma-separated fallback list.
    pub family: Option<String>,
    /// Logical font size.
    pub font_size: Option<f64>,
    /// OpenType weight from 1 through 1000.
    pub weight: Option<u16>,
    /// Monochrome foreground color. Color glyphs retain their own RGB.
    pub color: Option<Color>,
    /// Request an italic face or synthetic oblique when the font has no italic face.
    pub italic: bool,
    /// Draw an underline in the resolved foreground color.
    pub underline: bool,
    /// Draw a strikethrough in the resolved foreground color.
    pub strikethrough: bool,
    /// Extra logical spacing between letters. May be negative. Defaults to zero.
    pub letter_spacing: Option<f64>,
    /// Extra logical spacing at word separators. May be negative. Defaults to zero.
    pub word_spacing: Option<f64>,
    /// Line-height multiplier relative to the span's font size. Must be positive
    /// and at most 16. Inherits the paragraph's multiplier when unspecified.
    pub line_height: Option<f64>,
}

impl TextSpanStyle {
    fn validate(&self) -> Result<(), Error> {
        if self.font_size.is_some_and(|size| !valid_font_size(size))
            || self.weight.is_some_and(|weight| !valid_weight(weight))
            || self.color.is_some_and(|color| !valid_color(color))
            || self
                .letter_spacing
                .is_some_and(|spacing| !(spacing as f32).is_finite())
            || self
                .word_spacing
                .is_some_and(|spacing| !(spacing as f32).is_finite())
            || self
                .line_height
                .is_some_and(|height| !valid_line_height(height))
        {
            return Err(Error::InvalidInput);
        }
        Ok(())
    }
}

/// A style override for a nonempty range of UTF-8 byte offsets in a paragraph.
#[derive(Clone, Debug, PartialEq)]
pub struct TextSpan {
    /// The start is inclusive and the end is exclusive.
    pub range: Range<usize>,
    /// Properties applied across this range.
    pub style: TextSpanStyle,
}

/// Immutable paragraph content with validated, non-overlapping style spans.
/// Clones share the string and spans, making values cheap to pass through signals.
#[derive(Clone, Debug, Default)]
pub struct RichText {
    text: Arc<str>,
    spans: Arc<[TextSpan]>,
}

impl RichText {
    /// Validates byte boundaries and style overrides. Spans may be supplied in
    /// any order, up to 65,535 spans. Unstyled gaps inherit the paragraph style.
    /// A glyph shared by several code points (for example a ligature) can
    /// receive only one foreground and decoration style, from the start of that
    /// shaped cluster.
    pub fn new(text: impl Into<Arc<str>>, mut spans: Vec<TextSpan>) -> Result<Self, Error> {
        // Parley's ranged builder allocates one u16 style index per non-root run.
        if spans.len() > usize::from(u16::MAX) {
            return Err(Error::TooManySpans);
        }
        let text = text.into();
        spans.sort_by_key(|span| span.range.start);
        let mut previous_end = 0;
        for span in &spans {
            let Range { start, end } = span.range;
            if start < previous_end
                || start >= end
                || !text.is_char_boundary(start)
                || !text.is_char_boundary(end)
            {
                return Err(Error::InvalidSpan { start, end });
            }
            span.style.validate()?;
            previous_end = end;
        }
        Ok(Self {
            text,
            spans: spans.into(),
        })
    }

    /// Creates content with no span overrides.
    pub fn plain(text: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    /// Returns the paragraph's UTF-8 source.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Returns the validated spans, ordered by source offset.
    pub fn spans(&self) -> &[TextSpan] {
        &self.spans
    }
}

impl PartialEq for RichText {
    fn eq(&self, other: &Self) -> bool {
        (Arc::ptr_eq(&self.spans, &other.spans) || self.spans == other.spans)
            && (Arc::ptr_eq(&self.text, &other.text) || self.text == other.text)
    }
}

pub(crate) fn valid_font_size(size: f64) -> bool {
    // Parley measures with f32. Reject positive values that round down to zero.
    size as f32 > 0.0 && size <= 1024.0
}

pub(crate) fn valid_weight(weight: u16) -> bool {
    (1..=1000).contains(&weight)
}

pub(crate) fn valid_line_height(height: f64) -> bool {
    height as f32 > 0.0 && height <= 16.0
}

pub(crate) fn valid_color(color: Color) -> bool {
    color.to_srgb().into_iter().all(f32::is_finite)
}
