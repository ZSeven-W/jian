use super::{node_to_style, node_to_style_responsive};
use crate::document::NodeTree;
use crate::geometry::Rect;
use crate::layout::fill_text::legacy_style;
use crate::layout::{measure::EstimateBackend, LayoutEngine};
use jian_ops_schema::node::PenNode;
use serde_json::{json, Value};
use std::rc::Rc;
use taffy::prelude::*;

fn row_layout(
    width: f32,
    growth: Option<&str>,
    min_width: Option<f32>,
    responsive: bool,
    justify: Option<&str>,
) -> (Rect, Rect) {
    let root: PenNode = serde_json::from_value(json!({
        "type": "frame", "id": "row", "layout": "horizontal",
        "width": width, "height": "fit_content",
        "justifyContent": justify, "alignItems": "center",
        "children": [
            {"type": "ellipse", "id": "icon", "width": 36, "height": 36},
            {
                "type": "text", "id": "text", "width": "fill_container",
                "height": "fit_content", "minWidth": min_width,
                "content": "MMMMMMMMMMMMMMMMMMMMMMMM", "fontSize": 16,
                "lineHeight": 1.0, "textGrowth": growth
            }
        ]
    }))
    .unwrap();
    compute_row(root, width, responsive)
}

fn compute_row(root: PenNode, width: f32, responsive: bool) -> (Rect, Rect) {
    let mut doc = NodeTree::new();
    doc.insert_subtree(root, None);
    let mut engine = LayoutEngine::with_backend(Rc::new(EstimateBackend));
    let root_id = if responsive {
        engine.build_responsive(&doc, true).unwrap()[0]
    } else {
        engine.build(&doc).unwrap()[0]
    };
    if responsive {
        engine.compute_responsive(root_id, (width, 300.0)).unwrap();
    } else {
        engine.compute(root_id, (width, 300.0)).unwrap();
    }
    (
        engine.node_rect(doc.get("icon").unwrap()).unwrap(),
        engine.node_rect(doc.get("text").unwrap()).unwrap(),
    )
}

#[test]
fn fill_text_uses_remaining_row_width_and_wraps() {
    for justify in ["center", "end", "space_between", "space_around"] {
        for width in [116.0, 196.0] {
            let (icon, text) = row_layout(width, Some("fixed-width"), None, false, Some(justify));
            assert_eq!(icon.origin.x, 0.0);
            assert_eq!(icon.size.width, 36.0);
            assert_eq!(text.origin.x, 36.0);
            assert_eq!(text.size.width, width - 36.0);
            assert!(text.size.height > 16.0, "fixed-width text must wrap");
        }
    }
}

#[test]
fn start_rows_keep_legacy_fill_text_overflow() {
    for justify in [None, Some("start")] {
        let (icon, text) = row_layout(116.0, Some("fixed-width"), None, false, justify);
        assert_eq!(icon.size.width, 36.0);
        assert_eq!(text.origin.x, 36.0);
        assert_eq!(text.size.width, 116.0);
    }
}

#[test]
fn responsive_start_rows_still_share_fill_text_width() {
    let (_, text) = row_layout(116.0, Some("fixed-width"), None, true, Some("start"));
    assert_eq!(text.size.width, 80.0);
}

#[test]
fn nested_fill_row_shares_its_resolved_content_width() {
    for (padding, gap) in [(0, 0), (6, 8)] {
        let root: PenNode = serde_json::from_value(json!({
            "type": "frame", "id": "root", "width": 116, "layout": "vertical",
            "children": [{
                "type": "frame", "id": "row", "width": "fill_container",
                "layout": "horizontal", "justifyContent": "center",
                "padding": [0, padding], "gap": gap,
                "children": [
                    {"type": "ellipse", "id": "icon", "width": 36, "height": 36},
                    {
                        "type": "text", "id": "text", "width": "fill_container",
                        "textGrowth": "fixed-width", "fontSize": 16, "lineHeight": 1.0,
                        "content": "MMMMMMMMMMMMMMMMMMMMMMMM"
                    }
                ]
            }]
        }))
        .unwrap();
        let (icon, text) = compute_row(root, 116.0, false);
        assert_eq!(icon.origin.x, padding as f32);
        assert_eq!(icon.size.width, 36.0);
        assert_eq!(text.origin.x, (padding + 36 + gap) as f32);
        assert_eq!(text.size.width, (80 - 2 * padding - gap) as f32);
        assert!(text.size.height > 16.0);
    }
}

fn legacy_fill_style(parent_fields: Value, child_fields: Value) -> Style {
    let mut parent = json!({
        "type": "frame", "id": "row", "width": 116,
        "layout": "horizontal", "justifyContent": "center"
    });
    parent
        .as_object_mut()
        .unwrap()
        .extend(parent_fields.as_object().unwrap().clone());
    let mut child = json!({
        "type": "text", "id": "text", "width": "fill_container",
        "textGrowth": "fixed-width", "content": "label"
    });
    child
        .as_object_mut()
        .unwrap()
        .extend(child_fields.as_object().unwrap().clone());
    let parent: PenNode = serde_json::from_value(parent).unwrap();
    let child: PenNode = serde_json::from_value(child).unwrap();
    legacy_style(&child, Some(&parent))
}

#[test]
fn legacy_fill_minimum_requires_a_bounded_non_start_horizontal_parent() {
    for parent_fields in [
        json!({}),
        json!({"layout": null}),
        json!({"width": "fill_container"}),
    ] {
        let style = legacy_fill_style(parent_fields, json!({}));
        assert_eq!(style.min_size.width, length(0.0_f32));
        assert_eq!(style.min_size.height, Dimension::Auto);
    }
    for parent_fields in [
        json!({"width": null}),
        json!({"width": "fit_content"}),
        json!({"width": "$app.width"}),
        json!({"width": 0}),
        json!({"width": -1}),
        json!({"layout": "vertical"}),
        json!({"layout": "none"}),
        json!({"justifyContent": null}),
        json!({"justifyContent": "start"}),
    ] {
        assert_eq!(
            legacy_fill_style(parent_fields, json!({})).min_size.width,
            Dimension::Auto
        );
    }
}

#[test]
fn legacy_fill_minimum_preserves_authored_geometry() {
    for child_fields in [
        json!({"minWidth": 100}),
        json!({"x": 0}),
        json!({"y": 0}),
        json!({"constraints": {"h": "left", "v": "top"}}),
    ] {
        assert_eq!(
            legacy_fill_style(json!({}), child_fields).min_size.width,
            Dimension::Auto
        );
    }
    let root: PenNode = serde_json::from_value(json!({
        "type": "text", "id": "text", "width": "fill_container",
        "textGrowth": "fixed-width", "content": "label"
    }))
    .unwrap();
    assert_eq!(legacy_style(&root, None).min_size.width, Dimension::Auto);
}

#[test]
fn non_wrapping_fill_text_keeps_legacy_minimum_and_overflow() {
    for growth in [None, Some("auto"), Some("fixed-width-height")] {
        let node: PenNode = serde_json::from_value(json!({
            "type": "text", "id": "text", "width": "fill_container",
            "content": "label", "textGrowth": growth
        }))
        .unwrap();
        assert_eq!(node_to_style(&node).min_size.width, Dimension::Auto);
        let (icon, text) = row_layout(116.0, growth, None, false, Some("center"));
        assert_eq!(icon.size.width, 36.0);
        assert_eq!(text.size.width, 116.0);
        assert_eq!(text.size.height, 16.0, "{growth:?} text must not wrap");
    }
}

#[test]
fn non_fill_text_keeps_legacy_automatic_minimum_and_ignores_limits() {
    for width in [Value::Null, json!("fit_content"), json!(80)] {
        let node: PenNode = serde_json::from_value(json!({
            "type": "text", "id": "text", "width": width,
            "height": "fill_container", "minWidth": 100, "maxHeight": 40,
            "content": "label", "textGrowth": "fixed-width"
        }))
        .unwrap();
        let style = node_to_style(&node);
        assert_eq!(style.min_size.width, Dimension::Auto);
        assert_eq!(style.min_size.height, Dimension::Auto);
        assert_eq!(style.max_size.height, Dimension::Auto);
    }
}

#[test]
fn other_fill_leaves_keep_automatic_minimum() {
    let node: PenNode = serde_json::from_value(json!({
        "type": "ellipse", "id": "icon", "width": "fill_container", "height": 36
    }))
    .unwrap();
    assert_eq!(node_to_style(&node).min_size.width, Dimension::Auto);
}

#[test]
fn responsive_explicit_minimum_overrides_fill_text_implicit_minimum() {
    let node: PenNode = serde_json::from_value(json!({
        "type": "text", "id": "text", "width": "fill_container",
        "content": "label", "minWidth": 100, "textGrowth": "fixed-width"
    }))
    .unwrap();
    let mut warnings = Vec::new();
    let style = node_to_style_responsive(&node, &mut warnings);
    assert_eq!(style.min_size.width, length(100.0_f32));
    assert!(warnings.is_empty());

    let (_, legacy_text) = row_layout(
        116.0,
        Some("fixed-width"),
        Some(100.0),
        false,
        Some("center"),
    );
    assert_eq!(legacy_text.size.width, 116.0);
    let (icon, responsive_text) = row_layout(
        116.0,
        Some("fixed-width"),
        Some(100.0),
        true,
        Some("center"),
    );
    assert_eq!(icon.size.width, 36.0);
    assert_eq!(responsive_text.size.width, 100.0);
}
