#![cfg(feature = "textlayout")]

use jian_core::geometry::point;
use jian_core::render::{FieldKey, Granularity, TextGeometryError, WritingDirection};
use jian_core::runtime::Runtime;
use jian_skia::{SkiaMeasure, SkiaTextField, SkiaTextGeometry};
use std::rc::Rc;

fn runtime_with_field(text: &str, width: f32) -> Runtime {
    let mut runtime = Runtime::new();
    runtime
        .load_str(&format!(
            r#"{{
              "version":"0.8.0",
              "children":[{{
                "type":"text_area",
                "id":"field",
                "value":{text},
                "width":{width},
                "height":240
              }}]
            }}"#,
            text = serde_json::to_string(text).unwrap(),
        ))
        .unwrap();
    runtime
        .build_layout_with(Rc::new(SkiaMeasure::new()), (400.0, 300.0))
        .unwrap();
    let key = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    runtime.focus_request(key).unwrap();

    let bounds = runtime.focused_node_rect().unwrap();
    let mut field = SkiaTextField::new(text, bounds);
    field.text_origin = point(bounds.min_x() + 6.0, bounds.min_y() + 6.0);
    field.max_width = (width - 12.0).max(1.0);
    field.font_size = 20.0;
    field.line_height = 1.3;

    let geometry = Rc::new(SkiaTextGeometry::new());
    geometry.set_field(FieldKey::new("field"), field);
    runtime.install_text_geometry(geometry);
    runtime
}

fn midpoint_x(a: f32, b: f32) -> f32 {
    a + (b - a) * 0.25
}

#[test]
fn runtime_reports_not_ready_before_first_layout() {
    let mut runtime = Runtime::new();
    runtime
        .load_str(
            r#"{
              "version":"0.8.0",
              "children":[{
                "type":"text_area","id":"field","value":"pending",
                "width":160,"height":80
              }]
            }"#,
        )
        .unwrap();
    let key = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    runtime.focus_request(key).unwrap();
    runtime.install_text_geometry(Rc::new(SkiaTextGeometry::new()));

    assert_eq!(runtime.text_caret_rect(0), Err(TextGeometryError::NotReady));
}

#[test]
fn multiline_caret_rect_uses_the_requested_line_through_runtime() {
    let runtime = runtime_with_field("first\nsecond\nthird", 240.0);

    let first = runtime.text_caret_rect(2).unwrap();
    let second = runtime.text_caret_rect(8).unwrap();
    let third = runtime.text_caret_rect(15).unwrap();

    assert!(first.min_y() < second.min_y(), "{first:?} vs {second:?}");
    assert!(second.min_y() < third.min_y(), "{second:?} vs {third:?}");
}

#[test]
fn wrapped_range_returns_one_ltr_rect_per_visual_line_through_runtime() {
    let text = "alpha beta gamma delta epsilon zeta";
    let runtime = runtime_with_field(text, 112.0);
    let end = text.encode_utf16().count() as u32;

    let rects = runtime.text_rects_for_range(0, end).unwrap();

    assert!(rects.len() >= 2, "selection did not wrap: {rects:?}");
    assert!(
        rects
            .windows(2)
            .all(|pair| pair[0].rect.min_y() < pair[1].rect.min_y()),
        "expected exactly one rectangle on each successive line: {rects:?}"
    );
    assert!(rects
        .iter()
        .all(|rect| rect.writing_direction == WritingDirection::LeftToRight));
}

#[test]
fn range_rects_preserve_paragraph_bidi_direction_through_runtime() {
    let text = "שלום עולם";
    let runtime = runtime_with_field(text, 240.0);
    let end = text.encode_utf16().count() as u32;

    let rects = runtime.text_rects_for_range(0, end).unwrap();

    assert!(!rects.is_empty());
    assert!(rects
        .iter()
        .all(|rect| rect.writing_direction == WritingDirection::RightToLeft));
}

#[test]
fn coordinate_hit_test_round_trips_a_shaped_caret_through_runtime() {
    let runtime = runtime_with_field("round trip", 240.0);
    let offset = 7;
    let caret = runtime.text_caret_rect(offset).unwrap();

    let hit = runtime
        .text_position_at_point(caret.min_x() + 0.1, caret.center().y)
        .unwrap();

    assert_eq!(hit, offset);
}

#[test]
fn word_hit_test_uses_uax29_for_cjk_and_latin_through_runtime() {
    let text = "设计 OpenAI 工具";
    let runtime = runtime_with_field(text, 300.0);

    let cjk_start = runtime.text_caret_rect(0).unwrap();
    let cjk_end = runtime.text_caret_rect(1).unwrap();
    let cjk = runtime
        .text_range_at_point(
            midpoint_x(cjk_start.min_x(), cjk_end.min_x()),
            cjk_start.center().y,
            Granularity::Word,
        )
        .unwrap();
    assert_eq!(cjk, (0, 1));

    let latin_start = runtime.text_caret_rect(3).unwrap();
    let latin_end = runtime.text_caret_rect(4).unwrap();
    let latin = runtime
        .text_range_at_point(
            midpoint_x(latin_start.min_x(), latin_end.min_x()),
            latin_start.center().y,
            Granularity::Word,
        )
        .unwrap();
    assert_eq!(latin, (3, 9));
}

#[test]
fn grapheme_hit_test_never_splits_an_emoji_zwj_sequence_through_runtime() {
    let runtime = runtime_with_field("A👩‍💻B", 300.0);
    let emoji_start = runtime.text_caret_rect(1).unwrap();
    let emoji_end = runtime.text_caret_rect(6).unwrap();

    let range = runtime
        .text_range_at_point(
            midpoint_x(emoji_start.min_x(), emoji_end.min_x()),
            emoji_start.center().y,
            Granularity::Character,
        )
        .unwrap();

    assert_eq!(range, (1, 6));
}
