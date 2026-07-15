//! Shaping-backed editable text geometry over Skia Paragraph.

use jian_core::geometry::{rect, Point, Rect};
use jian_core::layout::measure::FontStyleKind;
use jian_core::render::{
    byte_to_utf16_offset, normalize_utf16_offset, utf16_len, utf16_to_byte_offset, FieldKey,
    Granularity, TextGeometry, TextRect, WritingDirection,
};
use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::textlayout::{
    FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, RectHeightStyle, RectWidthStyle,
    TextBox, TextDirection, TextStyle,
};
use skia_safe::{FontMgr, FontStyle};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

const NATURAL_LAYOUT_BUDGET: f32 = 1.0e6;
const CARET_WIDTH: f32 = 1.0;

/// Paragraph inputs for one editable field.
///
/// The host updates this snapshot after text or layout changes. Coordinates are
/// surface-logical; `text_origin` is the same origin used to paint the Paragraph.
#[derive(Clone, Debug)]
pub struct SkiaTextField {
    pub text: String,
    pub bounds: Rect,
    pub text_origin: Point,
    pub max_width: f32,
    pub font_family: String,
    pub font_size: f32,
    pub font_weight: u16,
    pub font_style: FontStyleKind,
    pub letter_spacing: f32,
    pub line_height: f32,
    /// `bounds` is the surface-space AABB when any ancestor is rotated.
    pub rotated_ancestor: bool,
}

impl SkiaTextField {
    pub fn new(text: impl Into<String>, bounds: Rect) -> Self {
        Self {
            text: text.into(),
            bounds,
            text_origin: bounds.origin,
            max_width: bounds.size.width,
            font_family: String::new(),
            font_size: 14.0,
            font_weight: 400,
            font_style: FontStyleKind::Normal,
            letter_spacing: 0.0,
            line_height: 0.0,
            rotated_ancestor: false,
        }
    }
}

struct FieldLayout {
    field: SkiaTextField,
    paragraph: Paragraph,
}

/// Mutable field registry queried through `jian_core::Runtime`.
pub struct SkiaTextGeometry {
    font_collection: RefCell<Rc<FontCollection>>,
    font_resolver: crate::font_resolve::FontResolver,
    built_generation: Cell<u64>,
    fields: RefCell<HashMap<FieldKey, FieldLayout>>,
}

impl SkiaTextGeometry {
    pub fn new() -> Self {
        crate::font_lock::with_font_lock(|| Self::with_font_manager(FontMgr::default()))
    }

    pub fn with_font_manager(font_mgr: FontMgr) -> Self {
        crate::font_lock::with_font_lock(|| {
            let built_generation = crate::bundled_fonts::generation();
            let font_resolver = crate::font_resolve::FontResolver::new(font_mgr);
            let collection = crate::measure::build_collection(&font_resolver);
            Self {
                font_collection: RefCell::new(Rc::new(collection)),
                font_resolver,
                built_generation: Cell::new(built_generation),
                fields: RefCell::new(HashMap::new()),
            }
        })
    }

    /// Insert or replace a field snapshot after layout or text changes.
    pub fn set_field(&self, key: FieldKey, field: SkiaTextField) {
        crate::font_lock::with_font_lock(|| {
            self.refresh_if_stale();
            let paragraph = build_paragraph(&field, &self.font_collection.borrow());
            self.fields
                .borrow_mut()
                .insert(key, FieldLayout { field, paragraph });
        });
    }

    pub fn remove_field(&self, key: &FieldKey) -> bool {
        self.fields.borrow_mut().remove(key).is_some()
    }

    pub fn clear(&self) {
        self.fields.borrow_mut().clear();
    }

    fn refresh_if_stale(&self) {
        let current = crate::bundled_fonts::generation();
        if current == self.built_generation.get() {
            return;
        }
        let collection = Rc::new(crate::measure::build_collection(&self.font_resolver));
        for layout in self.fields.borrow_mut().values_mut() {
            layout.paragraph = build_paragraph(&layout.field, &collection);
        }
        *self.font_collection.borrow_mut() = collection;
        self.built_generation.set(current);
    }

    fn with_field<T>(&self, key: &FieldKey, query: impl FnOnce(&FieldLayout) -> T) -> Option<T> {
        crate::font_lock::with_font_lock(|| {
            self.refresh_if_stale();
            let fields = self.fields.borrow();
            fields.get(key).map(query)
        })
    }
}

impl Default for SkiaTextGeometry {
    fn default() -> Self {
        Self::new()
    }
}

impl TextGeometry for SkiaTextGeometry {
    fn caret_rect(&self, field: &FieldKey, offset_utf16: u32) -> Option<Rect> {
        self.with_field(field, |layout| caret_rect(layout, offset_utf16))
    }

    fn rects_for_range(&self, field: &FieldKey, start: u32, end: u32) -> Vec<TextRect> {
        self.with_field(field, |layout| range_rects(layout, start, end))
            .unwrap_or_default()
    }

    fn position_at_point(&self, field: &FieldKey, x: f32, y: f32) -> Option<u32> {
        self.with_field(field, |layout| position_at_point(layout, x, y))
    }

    fn range_at_point(
        &self,
        field: &FieldKey,
        x: f32,
        y: f32,
        granularity: Granularity,
    ) -> Option<(u32, u32)> {
        self.with_field(field, |layout| {
            let offset = position_at_point(layout, x, y);
            boundary_at_offset(&layout.field.text, offset, granularity)
        })
    }
}

fn build_paragraph(field: &SkiaTextField, collection: &FontCollection) -> Paragraph {
    let mut paragraph_style = ParagraphStyle::new();
    let mut text_style = TextStyle::new();
    let font_size = if field.font_size.is_finite() && field.font_size > 0.0 {
        field.font_size
    } else {
        14.0
    };
    text_style.set_font_size(font_size);
    if !field.font_family.is_empty() {
        text_style.set_font_families(&[field.font_family.as_str()]);
    }
    let slant = match field.font_style {
        FontStyleKind::Normal => Slant::Upright,
        FontStyleKind::Italic => Slant::Italic,
    };
    text_style.set_font_style(FontStyle::new(
        Weight::from(field.font_weight as i32),
        Width::NORMAL,
        slant,
    ));
    if field.letter_spacing.is_finite() && field.letter_spacing != 0.0 {
        text_style.set_letter_spacing(field.letter_spacing);
    }
    if field.line_height.is_finite() && field.line_height > 0.0 {
        text_style.set_height(field.line_height);
        text_style.set_height_override(true);
        text_style.set_half_leading(true);
    }
    paragraph_style.set_text_style(&text_style);

    let mut builder = ParagraphBuilder::new(&paragraph_style, collection.clone());
    builder.add_text(&field.text);
    let mut paragraph = builder.build();
    let width = if field.max_width.is_finite() && field.max_width > 0.0 {
        field.max_width
    } else {
        NATURAL_LAYOUT_BUDGET
    };
    paragraph.layout(width);
    paragraph
}

fn caret_rect(layout: &FieldLayout, requested: u32) -> Rect {
    if layout.field.rotated_ancestor {
        return layout.field.bounds;
    }
    let text = &layout.field.text;
    let offset = normalize_utf16_offset(text, requested);
    let length = utf16_len(text);

    if offset < length {
        let boxes = layout.paragraph.get_rects_for_range(
            offset as usize..offset.saturating_add(1) as usize,
            RectHeightStyle::Max,
            RectWidthStyle::Tight,
        );
        if let Some(text_box) = boxes.first() {
            return caret_from_box(&layout.field, text_box, true);
        }
    }

    let prefix_end = offset.max(u32::from(text.is_empty()));
    let boxes = layout.paragraph.get_rects_for_range(
        0..prefix_end as usize,
        RectHeightStyle::Max,
        RectWidthStyle::Tight,
    );
    boxes
        .last()
        .map(|text_box| caret_from_box(&layout.field, text_box, false))
        .unwrap_or_else(|| {
            rect(
                layout.field.text_origin.x,
                layout.field.text_origin.y,
                CARET_WIDTH,
                layout.field.font_size.max(1.0),
            )
        })
}

fn caret_from_box(field: &SkiaTextField, text_box: &TextBox, leading: bool) -> Rect {
    let is_ltr = text_box.direct == TextDirection::LTR;
    let x = if leading == is_ltr {
        text_box.rect.left()
    } else {
        text_box.rect.right()
    };
    rect(
        field.text_origin.x + x,
        field.text_origin.y + text_box.rect.top(),
        CARET_WIDTH,
        text_box.rect.height().max(1.0),
    )
}

fn range_rects(layout: &FieldLayout, requested_start: u32, requested_end: u32) -> Vec<TextRect> {
    let text = &layout.field.text;
    let mut start = normalize_utf16_offset(text, requested_start);
    let mut end = normalize_utf16_offset(text, requested_end);
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    if start == end {
        return Vec::new();
    }
    let boxes = layout.paragraph.get_rects_for_range(
        start as usize..end as usize,
        RectHeightStyle::Max,
        RectWidthStyle::Tight,
    );
    if layout.field.rotated_ancestor {
        let direction = boxes
            .first()
            .map(text_box_direction)
            .unwrap_or(WritingDirection::LeftToRight);
        return vec![TextRect {
            rect: layout.field.bounds,
            writing_direction: direction,
        }];
    }
    boxes
        .into_iter()
        .map(|text_box| TextRect {
            rect: rect(
                layout.field.text_origin.x + text_box.rect.left(),
                layout.field.text_origin.y + text_box.rect.top(),
                text_box.rect.width(),
                text_box.rect.height(),
            ),
            writing_direction: text_box_direction(&text_box),
        })
        .collect()
}

fn text_box_direction(text_box: &TextBox) -> WritingDirection {
    if text_box.direct == TextDirection::RTL {
        WritingDirection::RightToLeft
    } else {
        WritingDirection::LeftToRight
    }
}

fn position_at_point(layout: &FieldLayout, x: f32, y: f32) -> u32 {
    let (local_x, local_y) = if layout.field.rotated_ancestor {
        let bounds = layout.field.bounds;
        let x_ratio = ((x - bounds.min_x()) / bounds.size.width.max(1.0)).clamp(0.0, 1.0);
        let y_ratio = ((y - bounds.min_y()) / bounds.size.height.max(1.0)).clamp(0.0, 1.0);
        (
            x_ratio * layout.field.max_width.max(1.0),
            y_ratio * layout.paragraph.height().max(1.0),
        )
    } else {
        (
            x - layout.field.text_origin.x,
            y - layout.field.text_origin.y,
        )
    };
    let position = layout
        .paragraph
        .get_glyph_position_at_coordinate((local_x, local_y))
        .position;
    normalize_utf16_offset(&layout.field.text, position.max(0) as u32)
}

fn boundary_at_offset(text: &str, offset: u32, granularity: Granularity) -> (u32, u32) {
    if text.is_empty() {
        return (0, 0);
    }
    let byte = utf16_to_byte_offset(text, offset);
    let segment = match granularity {
        Granularity::Character => text.grapheme_indices(true).find_map(|(start, value)| {
            segment_contains(start, value.len(), byte, text.len()).then_some((start, value.len()))
        }),
        Granularity::Word => text.split_word_bound_indices().find_map(|(start, value)| {
            segment_contains(start, value.len(), byte, text.len()).then_some((start, value.len()))
        }),
    }
    .or_else(|| match granularity {
        Granularity::Character => text
            .grapheme_indices(true)
            .next_back()
            .map(|(start, value)| (start, value.len())),
        Granularity::Word => text
            .split_word_bound_indices()
            .next_back()
            .map(|(start, value)| (start, value.len())),
    })
    .unwrap_or((0, 0));
    (
        byte_to_utf16_offset(text, segment.0),
        byte_to_utf16_offset(text, segment.0 + segment.1),
    )
}

fn segment_contains(start: usize, length: usize, position: usize, text_len: usize) -> bool {
    let end = start + length;
    (start <= position && position < end) || (position == text_len && end == text_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uax29_boundaries_cover_cjk_latin_and_emoji_zwj() {
        assert_eq!(
            boundary_at_offset("设计 OpenAI", 0, Granularity::Word),
            (0, 1)
        );
        assert_eq!(
            boundary_at_offset("设计 OpenAI", 4, Granularity::Word),
            (3, 9)
        );
        assert_eq!(
            boundary_at_offset("A👩‍💻B", 2, Granularity::Character),
            (1, 6)
        );
    }
}
