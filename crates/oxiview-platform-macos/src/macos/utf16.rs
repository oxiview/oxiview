//! Scalar-safe conversion between native UTF-16 ranges and displayed UTF-8.

use std::ops::Range;

pub(super) fn bytes(
    text: &str,
    location: usize,
    length: usize,
    clip_end: bool,
) -> Option<Range<usize>> {
    let end = location.checked_add(length)?;
    let mut start = None;
    let mut units = 0;
    for (offset, character) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        if units == location {
            start = Some(offset);
        }
        if units == end {
            return Some(start?..offset);
        }
        if units > end {
            return None;
        }
        units += character.len_utf16();
    }
    clip_end.then_some(start?..text.len())
}

pub(super) fn units(text: &str, bytes: Range<usize>) -> Option<(usize, usize)> {
    let start = text.get(..bytes.start)?.encode_utf16().count();
    let length = text.get(bytes)?.encode_utf16().count();
    Some((start, length))
}
