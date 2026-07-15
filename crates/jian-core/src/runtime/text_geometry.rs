use super::Runtime;
use crate::document::NodeKey;
use crate::geometry::{rect, Affine2, Rect};
use crate::render::{
    normalize_utf16_offset, utf16_to_byte_offset, FieldKey, Granularity, TextGeometry,
    TextGeometryError, TextRect, WritingDirection,
};
use jian_ops_schema::node::PenNode;
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

const APPROX_FONT_SIZE: f32 = 14.0;
const APPROX_CHAR_WIDTH: f32 = APPROX_FONT_SIZE * 0.55;
const APPROX_LINE_HEIGHT: f32 = APPROX_FONT_SIZE * 1.3;
const APPROX_PADDING: f32 = 6.0;

struct FocusedField {
    key: FieldKey,
    bounds: Rect,
    text: String,
    multiline: bool,
    rotated_aabb: Option<Rect>,
}

impl Runtime {
    /// Install shaped geometry supplied by the active renderer host.
    pub fn install_text_geometry(&mut self, service: Rc<dyn TextGeometry>) {
        self.text_geometry = Some(service);
    }

    /// Restore the built-in, unshaped approximation.
    pub fn use_approximate_text_geometry(&mut self) {
        self.text_geometry = None;
    }

    /// Lifecycle hosts set this false on suspend. A successful Runtime layout
    /// sets it true again, so pre-layout and suspended queries return NotReady.
    pub fn set_text_geometry_ready(&mut self, ready: bool) {
        self.text_geometry_ready = ready;
    }

    pub fn text_caret_rect(&self, offset_utf16: u32) -> Result<Rect, TextGeometryError> {
        let field = self.focused_field()?;
        if let Some(aabb) = field.rotated_aabb {
            return Ok(aabb);
        }
        let offset = normalize_utf16_offset(&field.text, offset_utf16);
        match &self.text_geometry {
            Some(service) => service
                .caret_rect(&field.key, offset)
                .ok_or(TextGeometryError::NotReady),
            None => Ok(approximate_caret(&field, offset)),
        }
    }

    pub fn text_rects_for_range(
        &self,
        start_utf16: u32,
        end_utf16: u32,
    ) -> Result<Vec<TextRect>, TextGeometryError> {
        let field = self.focused_field()?;
        let (start, end) = normalized_range(&field.text, start_utf16, end_utf16);
        if start == end {
            return Ok(Vec::new());
        }
        if let Some(aabb) = field.rotated_aabb {
            let direction = self
                .text_geometry
                .as_ref()
                .and_then(|service| {
                    service
                        .rects_for_range(&field.key, start, end)
                        .first()
                        .map(|text_rect| text_rect.writing_direction)
                })
                .unwrap_or(WritingDirection::LeftToRight);
            return Ok(vec![TextRect {
                rect: aabb,
                writing_direction: direction,
            }]);
        }
        match &self.text_geometry {
            Some(service) => {
                if service.caret_rect(&field.key, 0).is_none() {
                    return Err(TextGeometryError::NotReady);
                }
                Ok(service.rects_for_range(&field.key, start, end))
            }
            None => Ok(approximate_range_rects(&field, start, end)),
        }
    }

    pub fn text_position_at_point(&self, x: f32, y: f32) -> Result<u32, TextGeometryError> {
        let field = self.focused_field()?;
        if field.rotated_aabb.is_some() {
            return Ok(approximate_position(&field, x, y));
        }
        match &self.text_geometry {
            Some(service) => service
                .position_at_point(&field.key, x, y)
                .map(|offset| normalize_utf16_offset(&field.text, offset))
                .ok_or(TextGeometryError::NotReady),
            None => Ok(approximate_position(&field, x, y)),
        }
    }

    pub fn text_range_at_point(
        &self,
        x: f32,
        y: f32,
        granularity: Granularity,
    ) -> Result<(u32, u32), TextGeometryError> {
        let field = self.focused_field()?;
        if field.rotated_aabb.is_some() {
            let position = approximate_position(&field, x, y);
            return Ok(approximate_boundary(&field.text, position, granularity));
        }
        match &self.text_geometry {
            Some(service) => service
                .range_at_point(&field.key, x, y, granularity)
                .map(|(start, end)| normalized_range(&field.text, start, end))
                .ok_or(TextGeometryError::NotReady),
            None => {
                let position = approximate_position(&field, x, y);
                Ok(approximate_boundary(&field.text, position, granularity))
            }
        }
    }

    fn focused_field(&self) -> Result<FocusedField, TextGeometryError> {
        let node = self.focus.current().ok_or(TextGeometryError::NoFocus)?;
        let document = self.document.as_ref().ok_or(TextGeometryError::NoFocus)?;
        let data = document
            .tree
            .nodes
            .get(node)
            .ok_or(TextGeometryError::NoFocus)?;
        let multiline = matches!(data.schema, PenNode::TextArea(_));
        if !matches!(
            data.schema,
            PenNode::TextInput(_) | PenNode::TextArea(_) | PenNode::NumberInput(_)
        ) {
            return Err(TextGeometryError::NoFocus);
        }
        if !self.text_geometry_ready {
            return Err(TextGeometryError::NotReady);
        }
        let bounds = self
            .node_scene_rect(node)
            .ok_or(TextGeometryError::NotReady)?;
        let id = crate::document::tree::node_schema_id(&data.schema);
        let text = self
            .widget_states
            .get(id)
            .and_then(|state| match state {
                crate::widget_state::WidgetState::TextInput(state) => Some(state.effective_text()),
                _ => None,
            })
            .unwrap_or_else(|| schema_text(&data.schema));
        let rotated_aabb = self.rotated_field_aabb(node, bounds);
        Ok(FocusedField {
            key: FieldKey::new(id),
            bounds,
            text,
            multiline,
            rotated_aabb,
        })
    }

    fn rotated_field_aabb(&self, field: NodeKey, bounds: Rect) -> Option<Rect> {
        let document = self.document.as_ref()?;
        let mut chain = Vec::new();
        let mut current = Some(field);
        while let Some(key) = current {
            chain.push(key);
            current = document.tree.nodes.get(key).and_then(|node| node.parent);
        }
        chain.reverse();

        let mut combined = Affine2::identity();
        let mut has_rotation = false;
        for key in chain {
            let node = document.tree.nodes.get(key)?;
            let json = serde_json::to_value(&node.schema).ok()?;
            let node_bounds = self.node_scene_rect(key)?;
            let degrees = json
                .get("rotation")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0) as f32;
            has_rotation |= degrees.abs() > f32::EPSILON;
            if let Some(transform) = node_transform(&json, node_bounds) {
                combined = combined.then(&transform);
            }
        }
        has_rotation.then(|| combined.outer_transformed_rect(&bounds))
    }
}

fn schema_text(node: &PenNode) -> String {
    match node {
        PenNode::TextInput(node) => node.value.clone().unwrap_or_default(),
        PenNode::TextArea(node) => node.value.clone().unwrap_or_default(),
        PenNode::NumberInput(node) => match node.value.as_ref() {
            Some(jian_ops_schema::node::base::NumberOrExpression::Number(value)) => {
                value.to_string()
            }
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn normalized_range(text: &str, start: u32, end: u32) -> (u32, u32) {
    let start = normalize_utf16_offset(text, start);
    let end = normalize_utf16_offset(text, end);
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

fn approximate_caret(field: &FocusedField, offset: u32) -> Rect {
    let (line, column) = approximate_line_column(field, offset);
    let x = field.bounds.min_x() + APPROX_PADDING + column as f32 * APPROX_CHAR_WIDTH;
    let y = if field.multiline {
        field.bounds.min_y() + APPROX_PADDING + line as f32 * APPROX_LINE_HEIGHT
    } else {
        field.bounds.min_y() + (field.bounds.size.height - APPROX_FONT_SIZE) * 0.5
    };
    rect(x, y, 1.0, APPROX_FONT_SIZE)
}

fn approximate_range_rects(field: &FocusedField, start: u32, end: u32) -> Vec<TextRect> {
    let start_caret = approximate_caret(field, start);
    let end_caret = approximate_caret(field, end);
    if !field.multiline || (start_caret.min_y() - end_caret.min_y()).abs() < f32::EPSILON {
        return vec![TextRect {
            rect: rect(
                start_caret.min_x(),
                start_caret.min_y(),
                (end_caret.min_x() - start_caret.min_x()).max(1.0),
                APPROX_FONT_SIZE,
            ),
            writing_direction: WritingDirection::LeftToRight,
        }];
    }
    vec![TextRect {
        rect: field.bounds,
        writing_direction: WritingDirection::LeftToRight,
    }]
}

fn approximate_position(field: &FocusedField, x: f32, y: f32) -> u32 {
    let target_line = if field.multiline {
        ((y - field.bounds.min_y() - APPROX_PADDING) / APPROX_LINE_HEIGHT)
            .round()
            .max(0.0) as usize
    } else {
        0
    };
    let target_column = ((x - field.bounds.min_x() - APPROX_PADDING) / APPROX_CHAR_WIDTH)
        .round()
        .max(0.0) as usize;
    approximate_boundaries(field)
        .into_iter()
        .min_by_key(|(_, line, column)| {
            line.abs_diff(target_line)
                .saturating_mul(100_000)
                .saturating_add(column.abs_diff(target_column))
        })
        .map(|(offset, _, _)| offset)
        .unwrap_or(0)
}

fn approximate_line_column(field: &FocusedField, offset: u32) -> (usize, usize) {
    approximate_boundaries(field)
        .into_iter()
        .find(|(candidate, _, _)| *candidate == offset)
        .map(|(_, line, column)| (line, column))
        .unwrap_or((0, 0))
}

fn approximate_boundaries(field: &FocusedField) -> Vec<(u32, usize, usize)> {
    let max_columns = if field.multiline {
        ((field.bounds.size.width - APPROX_PADDING * 2.0) / APPROX_CHAR_WIDTH)
            .floor()
            .max(1.0) as usize
    } else {
        usize::MAX
    };
    let mut result = vec![(0, 0, 0)];
    let mut utf16 = 0_u32;
    let mut line = 0_usize;
    let mut column = 0_usize;
    for ch in field.text.chars() {
        utf16 = utf16.saturating_add(ch.len_utf16() as u32);
        if ch == '\n' {
            line += 1;
            column = 0;
        } else {
            column += 1;
            if column >= max_columns {
                line += 1;
                column = 0;
            }
        }
        result.push((utf16, line, column));
    }
    result
}

fn approximate_boundary(text: &str, offset: u32, granularity: Granularity) -> (u32, u32) {
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
        crate::render::byte_to_utf16_offset(text, segment.0),
        crate::render::byte_to_utf16_offset(text, segment.0 + segment.1),
    )
}

fn segment_contains(start: usize, length: usize, position: usize, text_len: usize) -> bool {
    let end = start + length;
    (start <= position && position < end) || (position == text_len && end == text_len)
}

fn node_transform(json: &serde_json::Value, bounds: Rect) -> Option<Affine2> {
    let degrees = json
        .get("rotation")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0) as f32;
    let flip_x = json
        .get("flipX")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let flip_y = json
        .get("flipY")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if degrees.abs() <= f32::EPSILON && !flip_x && !flip_y {
        return None;
    }
    let center = bounds.center();
    let local = Affine2::scale(
        if flip_x { -1.0 } else { 1.0 },
        if flip_y { -1.0 } else { 1.0 },
    )
    .then(&Affine2::rotation(euclid::Angle::radians(
        degrees.to_radians(),
    )));
    Some(
        Affine2::translation(-center.x, -center.y)
            .then(&local)
            .then(&Affine2::translation(center.x, center.y)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_geometry_is_ready_after_layout_and_uses_multiline_offset() {
        let mut runtime = Runtime::new();
        runtime
            .load_str(
                r#"{
                  "version":"0.8.0",
                  "children":[{
                    "type":"text_area","id":"field","value":"one\ntwo",
                    "width":160,"height":80
                  }]
                }"#,
            )
            .unwrap();
        runtime.build_layout((200.0, 100.0)).unwrap();
        let key = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("field")
            .unwrap();
        runtime.focus_request(key).unwrap();

        let first = runtime.text_caret_rect(1).unwrap();
        let second = runtime.text_caret_rect(5).unwrap();
        assert!(first.min_y() < second.min_y());
    }

    #[test]
    fn geometry_without_an_editable_focus_reports_no_focus() {
        let runtime = Runtime::new();
        assert_eq!(
            runtime.text_position_at_point(0.0, 0.0),
            Err(TextGeometryError::NoFocus)
        );
    }

    #[test]
    fn null_geometry_character_range_keeps_emoji_zwj_together() {
        assert_eq!(
            approximate_boundary("A👩‍💻B", 2, Granularity::Character),
            (1, 6)
        );
    }

    #[test]
    fn lifecycle_host_can_make_geometry_not_ready_while_suspended() {
        let mut runtime = Runtime::new();
        runtime
            .load_str(
                r#"{
                  "version":"0.8.0",
                  "children":[{
                    "type":"text_input","id":"field","value":"text",
                    "width":100,"height":40
                  }]
                }"#,
            )
            .unwrap();
        runtime.build_layout((120.0, 60.0)).unwrap();
        let key = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("field")
            .unwrap();
        runtime.focus_request(key).unwrap();
        runtime.set_text_geometry_ready(false);

        assert_eq!(runtime.text_caret_rect(0), Err(TextGeometryError::NotReady));
    }

    #[test]
    fn rotated_ancestor_falls_back_to_field_aabb() {
        let mut runtime = Runtime::new();
        runtime
            .load_str(
                r#"{
                  "version":"0.8.0",
                  "children":[{
                    "type":"frame","id":"rotated","rotation":20,
                    "width":200,"height":120,"children":[{
                      "type":"text_area","id":"field","value":"text",
                      "width":100,"height":40
                    }]
                  }]
                }"#,
            )
            .unwrap();
        runtime.build_layout((240.0, 160.0)).unwrap();
        let key = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("field")
            .unwrap();
        runtime.focus_request(key).unwrap();

        let caret = runtime.text_caret_rect(2).unwrap();
        assert!(caret.size.width > 1.0);
        assert!(caret.size.height >= 40.0);
    }
}
