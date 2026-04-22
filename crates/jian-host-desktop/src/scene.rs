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
use jian_core::render::{
    BorderRadii, DrawOp, GradientStop, LinearGradient, Paint, PathCommand, ShadowSpec, StrokeOp,
    TextAlign, TextRun,
};
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

    // --- Icon font nodes emit a vector-glyph op.
    if json.get("type").and_then(|t| t.as_str()) == Some("icon_font") {
        let name = json
            .get("iconFontName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let family = json
            .get("iconFontFamily")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let color = first_solid_color(json.get("fill")).unwrap_or(Color::rgb(0, 0, 0));
        out.push(DrawOp::Icon {
            rect: rect_logical,
            name,
            family,
            color,
        });
        return;
    }

    // --- Text first: draw_rect isn't the right primitive for text.
    if let Some(text_op) = try_text(json, r) {
        out.push(text_op);
        return;
    }

    let radii = corner_radii(json).unwrap_or_else(BorderRadii::zero);
    let stroke = stroke_op(json);

    // --- Shadows (first effect entry that's a drop shadow) paint
    // *underneath* the fill, so emit the shadow op first.
    if let Some(shadow) = first_shadow(json) {
        out.push(DrawOp::ShadowedRect {
            rect: rect_logical,
            radii,
            shadow,
        });
    }

    // --- Fill can be solid or linear gradient. Inspect `fill[0]`.
    let fill_arr = json.get("fill").and_then(|v| v.as_array());
    let first_fill = fill_arr.and_then(|arr| arr.first());

    if let Some(grad) = first_fill.and_then(try_linear_gradient) {
        out.push(DrawOp::LinearGradientRect {
            rect: rect_logical,
            radii,
            gradient: grad,
            stroke,
        });
        return;
    }

    let fill = first_solid_color(json.get("fill"));
    if fill.is_none() && stroke.is_none() {
        return;
    }

    let paint = Paint {
        fill,
        stroke,
        opacity: 1.0,
    };
    if radii != BorderRadii::zero() {
        out.push(DrawOp::RoundedRect {
            rect: rect_logical,
            radii,
            paint,
        });
    } else {
        out.push(DrawOp::Rect {
            rect: rect_logical,
            paint,
        });
    }
}

fn try_linear_gradient(fill: &Value) -> Option<LinearGradient> {
    let obj = fill.as_object()?;
    if obj.get("type").and_then(|t| t.as_str()) != Some("linear_gradient") {
        return None;
    }
    let angle_deg = obj.get("angle").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    let stops_arr = obj.get("stops")?.as_array()?;
    let mut stops = Vec::with_capacity(stops_arr.len());
    for s in stops_arr {
        let so = s.as_object()?;
        let offset = so.get("offset").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let hex = so.get("color")?.as_str()?;
        let color = Color::from_hex(hex)?;
        stops.push(GradientStop { offset, color });
    }
    if stops.len() < 2 {
        return None;
    }
    Some(LinearGradient {
        angle_deg,
        stops,
        opacity: 1.0,
    })
}

fn first_shadow(json: &Value) -> Option<ShadowSpec> {
    let effects = json.get("effects")?.as_array()?;
    for e in effects {
        let obj = e.as_object()?;
        if obj.get("type").and_then(|t| t.as_str()) != Some("shadow") {
            continue;
        }
        let dx = obj.get("offsetX").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let dy = obj.get("offsetY").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let blur = obj.get("blur").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let spread = obj.get("spread").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let color = obj
            .get("color")
            .and_then(|v| v.as_str())
            .and_then(Color::from_hex)
            .unwrap_or(Color::rgba(0, 0, 0, 0x40));
        return Some(ShadowSpec {
            color,
            dx,
            dy,
            blur,
            spread,
        });
    }
    None
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
    let font_weight = json
        .get("fontWeight")
        .and_then(|v| v.as_u64())
        .map(|n| n as u16)
        .unwrap_or(400);
    let color = first_solid_color(json.get("fill")).unwrap_or(Color::rgb(0, 0, 0));
    let align = match json.get("textAlign").and_then(|v| v.as_str()) {
        Some("center") => TextAlign::Center,
        Some("right") | Some("end") => TextAlign::End,
        _ => TextAlign::Start,
    };
    let line_height = json
        .get("lineHeight")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .unwrap_or(0.0);
    Some(DrawOp::Text(TextRun {
        content,
        font_family,
        font_size,
        font_weight,
        color,
        origin: point(r.min_x(), r.min_y()),
        max_width: r.size.width,
        align,
        line_height,
    }))
}

// Keep unused imports harmless.
#[allow(dead_code)]
fn _unused(_: PathCommand, _: Point) {}
#[allow(dead_code)]
fn _keep_penode(_: &PenNode) {}

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
