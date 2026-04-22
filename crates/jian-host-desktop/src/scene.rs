//! Scene walker — `RuntimeDocument` + `LayoutEngine` → `Vec<DrawOp>`.
//!
//! MVP walker: visit every node, read its resolved layout rect, pull
//! the first `fill[]` entry (solid color) via a schema-agnostic JSON
//! round-trip, and emit a `DrawOp::Rect` with that paint. Nodes
//! without a fill are skipped. No text, gradients, or path rendering
//! yet — those follow in a jian-ui layer.

use jian_core::document::RuntimeDocument;
use jian_core::geometry::rect;
use jian_core::layout::LayoutEngine;
use jian_core::render::{DrawOp, Paint};
use jian_core::scene::Color;
use jian_ops_schema::node::PenNode;

/// Build a flat draw-op list for the given document + layout. Callers
/// pump each op through `RenderBackend::draw` between
/// `begin_frame` / `end_frame`.
pub fn collect_draws(doc: &RuntimeDocument, layout: &LayoutEngine) -> Vec<DrawOp> {
    let mut out = Vec::with_capacity(doc.tree.nodes.len());
    for (key, node) in doc.tree.nodes.iter() {
        let Some(r) = layout.node_rect(key) else {
            continue;
        };
        let Some(color) = first_solid_fill(&node.schema) else {
            continue;
        };
        out.push(DrawOp::Rect {
            rect: rect(r.min_x(), r.min_y(), r.size.width, r.size.height),
            paint: Paint::solid(color),
        });
    }
    out
}

/// Extract the first solid-fill colour off a `PenNode` via JSON
/// round-trip. Works for every node variant that serializes a `fills`
/// field; returns `None` if the field is absent, empty, or not a solid
/// color.
fn first_solid_fill(n: &PenNode) -> Option<Color> {
    let v = serde_json::to_value(n).ok()?;
    let fills = v.as_object()?.get("fill")?.as_array()?;
    for fill in fills {
        let obj = fill.as_object()?;
        if obj.get("type").and_then(|t| t.as_str()) == Some("solid") {
            let hex = obj.get("color")?.as_str()?;
            if let Some(c) = Color::from_hex(hex) {
                return Some(c);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_core::Runtime;

    const DOC: &str = r##"{
      "formatVersion": "1.0", "version": "1.0.0", "id": "x",
      "app": { "name": "x", "version": "1", "id": "x" },
      "children": [
        { "type": "rectangle", "id": "a", "width": 100, "height": 50,
          "fill": [{ "type": "solid", "color": "#ff0000" }] },
        { "type": "rectangle", "id": "b", "width": 50, "height": 50 }
      ]
    }"##;

    #[test]
    fn collects_rect_with_solid_fill() {
        let mut rt = Runtime::new();
        rt.load_str(DOC).unwrap();
        rt.build_layout((800.0, 600.0)).unwrap();
        let ops = collect_draws(rt.document.as_ref().unwrap(), &rt.layout);
        // `a` has a fill, `b` doesn't — so only one DrawOp.
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            DrawOp::Rect { paint, .. } => {
                let c = paint.fill.expect("solid fill");
                assert_eq!(c.r(), 0xff);
                assert_eq!(c.g(), 0x00);
                assert_eq!(c.b(), 0x00);
            }
            _ => panic!("expected DrawOp::Rect"),
        }
    }

    #[test]
    fn unfilled_node_is_skipped() {
        let mut rt = Runtime::new();
        rt.load_str(
            r##"{ "formatVersion":"1.0", "version":"1.0.0", "id":"x",
                 "app": { "name":"x", "version":"1", "id":"x" },
                 "children": [{ "type":"rectangle", "id":"a", "width":10, "height":10 }] }"##,
        )
        .unwrap();
        rt.build_layout((100.0, 100.0)).unwrap();
        assert!(collect_draws(rt.document.as_ref().unwrap(), &rt.layout).is_empty());
    }
}
