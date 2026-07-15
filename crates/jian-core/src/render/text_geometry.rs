//! Host-provided shaped text geometry for editable fields.
//!
//! The runtime boundary uses UTF-16 code-unit offsets because that is the
//! native indexing model of `UITextInput` and Skia Paragraph hit testing.

use crate::geometry::Rect;

/// Stable schema id of an editable field.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FieldKey(String);

impl FieldKey {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for FieldKey {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for FieldKey {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// Text direction of the shaped run represented by a selection rectangle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WritingDirection {
    LeftToRight,
    RightToLeft,
}

/// A visual selection rectangle and its paragraph bidi direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextRect {
    pub rect: Rect,
    pub writing_direction: WritingDirection,
}

/// Boundary used by `range_at_point`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Granularity {
    /// A UAX #29 extended grapheme cluster.
    Character,
    /// A locale-independent UAX #29 word-boundary segment.
    Word,
}

/// Runtime-level availability failures consumed by the future FFI status map.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TextGeometryError {
    #[error("no editable field is focused")]
    NoFocus,
    #[error("text geometry is not ready")]
    NotReady,
}

/// Shaped geometry service installed by a renderer host.
pub trait TextGeometry {
    fn caret_rect(&self, field: &FieldKey, offset_utf16: u32) -> Option<Rect>;
    fn rects_for_range(&self, field: &FieldKey, start: u32, end: u32) -> Vec<TextRect>;
    fn position_at_point(&self, field: &FieldKey, x: f32, y: f32) -> Option<u32>;
    fn range_at_point(
        &self,
        field: &FieldKey,
        x: f32,
        y: f32,
        granularity: Granularity,
    ) -> Option<(u32, u32)>;
}

/// Clamp a UTF-16 offset and snap a surrogate-interior offset to the pair start.
pub fn normalize_utf16_offset(text: &str, offset: u32) -> u32 {
    let target = offset.min(utf16_len(text));
    let mut current = 0_u32;
    for ch in text.chars() {
        let next = current.saturating_add(ch.len_utf16() as u32);
        if target < next {
            return current;
        }
        if target == next {
            return next;
        }
        current = next;
    }
    current
}

/// Convert a normalized UTF-16 offset to a UTF-8 byte boundary.
pub fn utf16_to_byte_offset(text: &str, offset: u32) -> usize {
    let target = normalize_utf16_offset(text, offset);
    let mut utf16 = 0_u32;
    for (byte, ch) in text.char_indices() {
        if utf16 == target {
            return byte;
        }
        utf16 = utf16.saturating_add(ch.len_utf16() as u32);
    }
    text.len()
}

/// Convert a UTF-8 byte position to UTF-16, snapping byte interiors down.
pub fn byte_to_utf16_offset(text: &str, byte: usize) -> u32 {
    let target = byte.min(text.len());
    let mut utf16 = 0_u32;
    for (index, ch) in text.char_indices() {
        if index >= target {
            break;
        }
        if index + ch.len_utf8() > target {
            break;
        }
        utf16 = utf16.saturating_add(ch.len_utf16() as u32);
    }
    utf16
}

pub fn utf16_len(text: &str) -> u32 {
    text.chars()
        .map(|ch| ch.len_utf16() as u32)
        .fold(0_u32, u32::saturating_add)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_conversion_snaps_inside_surrogate_pairs() {
        let text = "A😀中";
        assert_eq!(utf16_len(text), 4);
        assert_eq!(normalize_utf16_offset(text, 2), 1);
        assert_eq!(utf16_to_byte_offset(text, 2), 1);
        assert_eq!(byte_to_utf16_offset(text, 3), 1);
        assert_eq!(byte_to_utf16_offset(text, text.len()), 4);
    }
}
