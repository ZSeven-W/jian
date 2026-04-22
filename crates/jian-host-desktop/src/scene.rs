//! Scene walker — `RuntimeDocument` + `LayoutEngine` → `Vec<DrawOp>`.
//!
//! MVP walker: visit every node, read its resolved layout rect, pull the
//! following fields via a schema-agnostic JSON round-trip:
//!
//! - `fill[]` — first solid color → fill paint.
//! - `stroke.{thickness,fill[]}` — first solid color + uniform thickness.
//! - `cornerRadius` (uniform f64 **or** `[tl,tr,br,bl]`) → `RoundedRect`.
//! - `content` on text nodes → `DrawOp::Text` with colour-from-fill.
//!
//! Gradient fills (`linear_gradient` / `radial_gradient`), image fills,
//! and background blur / shadow effects still arrive via a later commit
//! once the jian-skia `draw_canvas` path learns shaders / samplers.

use jian_core::geometry::{point, rect, Point};
use jian_core::render::{BorderRadii, DrawOp, Paint, PathCommand, StrokeOp, TextRun};
use jian_core::scene::Color;
use jian_ops_schema::node::PenNode;
use serde_json::Value;

/// Build a flat draw-op list for the given document + layout. Callers
/// pump each op through `RenderBackend::draw` between
/// `begin_frame` / `end_frame`.
pub fn collect_draws(
    doc: &jian_core::document::RuntimeDocument,
    layout: &jian_core::layout::LayoutEngine,
) -> Vec<DrawOp> {
    // Deterministic order: walk the tree depth-first starting at the
    // roots so parents paint before children and sibling z-order is
    // preserved.
    let mut out = Vec::with_capacity(doc.tree.nodes.len());
    for &root in &doc.tree.roots {
        walk(doc, layout, root, &mut out);
    }
    out
}

fn walk(
    doc: &jian_core::document::RuntimeDocument,
    layout: &jian_core::layout::LayoutEngine,
    key: jian_core::document::NodeKey,
    out: &mut Vec<DrawOp>,
) {
    let Some(node) = doc.tree.nodes.get(key) else {
        return;
    };
    let r = layout.node_rect(key);
    let json = serde_json::to_value(&node.schema).ok();

    if let (Some(r), Some(json)) = (r, &json) {
        emit_for_node(r, json, out);
    }

    for &child in &node.children {
        walk(doc, layout, child, out);
    }
}

fn emit_for_node(r: jian_core::geometry::Rect, json: &Value, out: &mut Vec<DrawOp>) {
    let rect_logical = rect(r.min_x(), r.min_y(), r.size.width, r.size.height);

    // --- Text first: draw_rect isn't the right primitive for text.
    if let Some(text_op) = try_text(json, r) {
        out.push(text_op);
        return;
    }

    // --- Fill (solid only for MVP) + optional stroke.
    let fill = first_solid_color(json.get("fill"));
    let stroke = stroke_op(json);
    let opacity = 1.0_f32;

    if fill.is_none() && stroke.is_none() {
        return;
    }

    let paint = Paint {
        fill,
        stroke,
        opacity,
    };
    let radii = corner_radii(json);
    if radii.map(|rr| rr != BorderRadii::zero()).unwrap_or(false) {
        out.push(DrawOp::RoundedRect {
            rect: rect_logical,
            radii: radii.unwrap(),
            paint,
        });
    } else {
        out.push(DrawOp::Rect {
            rect: rect_logical,
            paint,
        });
    }
}

fn first_solid_color(v: Option<&Value>) -> Option<Color> {
    let arr = v?.as_array()?;
    for fill in arr {
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

fn stroke_op(json: &Value) -> Option<StrokeOp> {
    let stroke = json.get("stroke")?.as_object()?;
    let thickness = stroke.get("thickness").and_then(|t| {
        if let Some(n) = t.as_f64() {
            Some(n as f32)
        } else if let Some(obj) = t.as_object() {
            obj.get("uniform").and_then(|u| u.as_f64()).map(|n| n as f32)
        } else {
            None
        }
    })?;
    if thickness <= 0.0 {
        return None;
    }
    let color = first_solid_color(stroke.get("fill")).unwrap_or(Color::rgba(0, 0, 0, 255));
    Some(StrokeOp {
        color,
        width: thickness,
    })
}

fn corner_radii(json: &Value) -> Option<BorderRadii> {
    let cr = json.get("cornerRadius")?;
    if let Some(n) = cr.as_f64() {
        return Some(BorderRadii::uniform(n as f32));
    }
    if let Some(arr) = cr.as_array() {
        if arr.len() == 4 {
            let get = |i: usize| arr[i].as_f64().unwrap_or(0.0) as f32;
            return Some(BorderRadii {
                tl: get(0),
                tr: get(1),
                br: get(2),
                bl: get(3),
            });
        }
    }
    None
}

fn try_text(json: &Value, r: jian_core::geometry::Rect) -> Option<DrawOp> {
    // A text node has `"type": "text"` and a `content` field that is
    // either a string or an array of styled segments (MVP: concatenate
    // `.text` for styled arrays).
    if json.get("type").and_then(|t| t.as_str()) != Some("text") {
        return None;
    }
    let content = match json.get("content")? {
        Value::String(s) => s.clone(),
        Value::Array(segs) => {
            let mut buf = String::new();
            for seg in segs {
                if let Some(t) = seg.as_object().and_then(|o| o.get("text")).and_then(|t| t.as_str())
                {
                    buf.push_str(t);
                }
            }
            if buf.is_empty() {
                return None;
            }
            buf
        }
        _ => return None,
    };
    let font_size = json
        .get("fontSize")
        .and_then(|v| v.as_f64())
        .unwrap_or(16.0) as f32;
    let font_family = json
        .get("fontFamily")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let color = first_solid_color(json.get("fill")).unwrap_or(Color::rgb(0, 0, 0));
    Some(DrawOp::Text(TextRun {
        content,
        font_family,
        font_size,
        color,
        origin: point(r.min_x(), r.min_y()),
    }))
}

// Keep unused imports harmless.
#[allow(dead_code)]
fn _unused(_: PathCommand, _: Point) {}
#[allow(dead_code)]
fn _keep_penode<'a>(_: &'a PenNode) {}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_core::Runtime;

    fn doc_with(src: &str) -> Runtime {
        let mut rt = Runtime::new();
        rt.load_str(src).unwrap();
        rt.build_layout((800.0, 600.0)).unwrap();
        rt
    }

    #[test]
    fn emits_rect_with_solid_fill() {
        let rt = doc_with(
            r##"{ "formatVersion":"1.0", "version":"1.0.0", "id":"x",
                 "app": { "name":"x", "version":"1", "id":"x" },
                 "children": [
                   { "type":"rectangle", "id":"a", "width":100, "height":50,
                     "fill":[{ "type":"solid", "color":"#ff0000" }] }
                 ]}"##,
        );
        let ops = collect_draws(rt.document.as_ref().unwrap(), &rt.layout);
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], DrawOp::Rect { .. }));
    }

    #[test]
    fn emits_rounded_rect_when_corner_radius_set() {
        let rt = doc_with(
            r##"{ "formatVersion":"1.0", "version":"1.0.0", "id":"x",
                 "app": { "name":"x", "version":"1", "id":"x" },
                 "children": [
                   { "type":"rectangle", "id":"a", "width":100, "height":50,
                     "cornerRadius": 8,
                     "fill":[{ "type":"solid", "color":"#1e88e5" }] }
                 ]}"##,
        );
        let ops = collect_draws(rt.document.as_ref().unwrap(), &rt.layout);
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            DrawOp::RoundedRect { radii, .. } => {
                assert_eq!(radii.tl, 8.0);
                assert_eq!(radii.br, 8.0);
            }
            other => panic!("expected RoundedRect, got {:?}", other),
        }
    }

    #[test]
    fn emits_stroke_from_pen_stroke() {
        let rt = doc_with(
            r##"{ "formatVersion":"1.0", "version":"1.0.0", "id":"x",
                 "app": { "name":"x", "version":"1", "id":"x" },
                 "children": [
                   { "type":"rectangle", "id":"a", "width":100, "height":50,
                     "fill":[{ "type":"solid", "color":"#ffffff" }],
                     "stroke": { "thickness": 2.0,
                                 "fill": [{ "type":"solid", "color":"#000000" }] } }
                 ]}"##,
        );
        let ops = collect_draws(rt.document.as_ref().unwrap(), &rt.layout);
        match &ops[0] {
            DrawOp::Rect { paint, .. } | DrawOp::RoundedRect { paint, .. } => {
                let s = paint.stroke.as_ref().expect("stroke");
                assert_eq!(s.width, 2.0);
            }
            other => panic!("unexpected op {:?}", other),
        }
    }

    #[test]
    fn emits_text_op_for_text_nodes() {
        let rt = doc_with(
            r##"{ "formatVersion":"1.0", "version":"1.0.0", "id":"x",
                 "app": { "name":"x", "version":"1", "id":"x" },
                 "children": [
                   { "type":"text", "id":"t", "content":"hello",
                     "fontSize": 24,
                     "fill":[{ "type":"solid", "color":"#333333" }] }
                 ]}"##,
        );
        let ops = collect_draws(rt.document.as_ref().unwrap(), &rt.layout);
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            DrawOp::Text(run) => {
                assert_eq!(run.content, "hello");
                assert!((run.font_size - 24.0).abs() < f32::EPSILON);
            }
            other => panic!("expected Text, got {:?}", other),
        }
    }

    #[test]
    fn walks_children_recursively() {
        let rt = doc_with(
            r##"{ "formatVersion":"1.0", "version":"1.0.0", "id":"x",
                 "app": { "name":"x", "version":"1", "id":"x" },
                 "children": [
                   { "type":"frame", "id":"root", "width":200, "height":100,
                     "fill":[{ "type":"solid", "color":"#eeeeee" }],
                     "children": [
                       { "type":"rectangle", "id":"a", "width":50, "height":50,
                         "fill":[{ "type":"solid", "color":"#ff0000" }] }
                     ]}
                 ]}"##,
        );
        let ops = collect_draws(rt.document.as_ref().unwrap(), &rt.layout);
        // Parent fill + child fill → 2 ops.
        assert_eq!(ops.len(), 2);
    }
}
