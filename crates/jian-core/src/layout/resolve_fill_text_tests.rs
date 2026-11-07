use super::{node_to_style, node_to_style_responsive};
use crate::document::NodeTree;
use crate::geometry::Rect;
use crate::layout::{measure::EstimateBackend, LayoutEngine};
use jian_ops_schema::node::PenNode;
use serde_json::{json, Value};
use std::rc::Rc;
use taffy::prelude::*;

fn row_layout(width: f32, growth: &str, min_width: Option<f32>, responsive: bool) -> (Rect, Rect) {
    let root: PenNode = serde_json::from_value(json!({
        "type": "frame", "id": "row", "layout": "horizontal",
        "width": width, "height": "fit_content",
        "justifyContent": "center", "alignItems": "center",
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
    for width in [116.0, 196.0] {
        let (icon, text) = row_layout(width, "fixed-width", None, false);
        assert_eq!(icon.origin.x, 0.0);
        assert_eq!(icon.size.width, 36.0);
        assert_eq!(text.origin.x, 36.0);
        assert_eq!(text.size.width, width - 36.0);
        assert!(text.size.height > 16.0, "fixed-width text must wrap");
    }
}

#[test]
fn fill_text_keeps_no_wrap_growth_modes() {
    for growth in ["auto", "fixed-width-height"] {
        let (icon, text) = row_layout(116.0, growth, None, false);
        assert_eq!(icon.size.width, 36.0);
        assert_eq!(text.size.width, 80.0);
        assert_eq!(text.size.height, 16.0, "{growth} text must not wrap");
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
        "content": "label", "minWidth": 100
    }))
    .unwrap();
    let mut warnings = Vec::new();
    let style = node_to_style_responsive(&node, &mut warnings);
    assert_eq!(style.min_size.width, length(100.0_f32));
    assert!(warnings.is_empty());

    let (_, legacy_text) = row_layout(116.0, "fixed-width", Some(100.0), false);
    assert_eq!(legacy_text.size.width, 80.0);
    let (icon, responsive_text) = row_layout(116.0, "fixed-width", Some(100.0), true);
    assert_eq!(icon.size.width, 36.0);
    assert_eq!(responsive_text.size.width, 100.0);
}
