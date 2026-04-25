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
//! Gradient fills (`linear_gradient`, `radial_gradient`) and drop-shadow
//! effects emit dedicated draw-ops (`LinearGradientRect` /
//! `RadialGradientRect` / `ShadowedRect`). Image nodes + image fills
//! emit `DrawOp::Image` carrying an `ImageSource` (data: URLs decode
//! inline in the skia backend; remote URLs need a host resolver and
//! currently fall back to a grey placeholder). Background blur still
//! waits on the jian-skia sampler path (Plan 12).

use jian_core::geometry::{point, rect, Point};
use jian_core::render::{
    BorderRadii, DrawOp, GradientStop, ImageSource, LinearGradient, Paint, PathCommand,
    RadialGradient, ShadowSpec, StrokeOp, TextAlign, TextRun,
};
use jian_core::scene::Color;
use jian_ops_schema::node::PenNode;
use serde_json::Value;

/// Build a flat draw-op list for the given document + layout. Callers
/// pump each op through `RenderBackend::draw` between
/// `begin_frame` / `end_frame`.
///
/// Static-only walker: `bindings.<prop>` expressions are NOT evaluated
/// — `content` etc. comes straight from the schema. Use
/// [`collect_draws_with_state`] when you have a live `StateGraph` and
/// want bindings reflected in the output (the player / dev paths
/// always use that one).
pub fn collect_draws(
    doc: &jian_core::document::RuntimeDocument,
    layout: &jian_core::layout::LayoutEngine,
) -> Vec<DrawOp> {
    let mut out = Vec::with_capacity(doc.tree.nodes.len());
    for &root in &doc.tree.roots {
        walk(doc, layout, root, None, &mut out);
    }
    out
}

/// Like `collect_draws` but evaluates `bindings.<prop>` expressions
/// against `state` so dynamic content (e.g. `Count: ${$app.count}`)
/// reflects the live runtime value. Without this path the walker
/// emits the schema's static `content` and counter / live-state
/// labels never refresh.
pub fn collect_draws_with_state(
    doc: &jian_core::document::RuntimeDocument,
    layout: &jian_core::layout::LayoutEngine,
    state: &jian_core::state::StateGraph,
) -> Vec<DrawOp> {
    let mut out = Vec::with_capacity(doc.tree.nodes.len());
    for &root in &doc.tree.roots {
        walk(doc, layout, root, Some(state), &mut out);
    }
    out
}

fn walk(
    doc: &jian_core::document::RuntimeDocument,
    layout: &jian_core::layout::LayoutEngine,
    key: jian_core::document::NodeKey,
    state: Option<&jian_core::state::StateGraph>,
    out: &mut Vec<DrawOp>,
) {
    let Some(node) = doc.tree.nodes.get(key) else {
        return;
    };
    let r = layout.node_rect(key);
    let mut json = serde_json::to_value(&node.schema).ok();

    let mut overrides = BindingOverrides::default();
    if let (Some(_), Some(j), Some(state)) = (r, json.as_mut(), state) {
        overrides = apply_bindings(j, state);
    }

    if let Some(json) = json.as_ref() {
        let visible = json.get("visible").and_then(|v| v.as_bool()).unwrap_or(true);
        if !visible {
            // `visible: false` (whether static or via bindings) drops
            // the subtree — children of an invisible parent never paint.
            return;
        }
    }

    if let (Some(r), Some(json)) = (r, &json) {
        let r = overrides.apply_to_rect(r);
        emit_for_node(r, json, out);
    }

    for &child in &node.children {
        walk(doc, layout, child, state, out);
    }
}

/// Records which `bindings.<rect-prop>` fired this frame so the walker
/// can override the laid-out rect *only* where a binding is authoritative.
/// Without this, the walker would mis-read the static `x` / `y` from
/// nested children's schema (parent-relative coords) and clobber the
/// layout engine's already-resolved absolute coords.
#[derive(Default, Clone, Copy)]
struct BindingOverrides {
    x: Option<f32>,
    y: Option<f32>,
    w: Option<f32>,
    h: Option<f32>,
}

impl BindingOverrides {
    fn apply_to_rect(self, r: jian_core::geometry::Rect) -> jian_core::geometry::Rect {
        rect(
            self.x.unwrap_or(r.origin.x),
            self.y.unwrap_or(r.origin.y),
            self.w.unwrap_or(r.size.width),
            self.h.unwrap_or(r.size.height),
        )
    }
}

/// Walk a node's `bindings` map and overwrite any matching field on
/// the JSON view with the binding's evaluated value. Recompiles every
/// expression on every frame — the perf-driven effect-driven scene
/// cache lands once the corpus shows real cost.
///
/// Supported binding keys:
/// - `content` (string projection on text nodes)
/// - `visible` (bool — emit_for_node drops the node if false)
/// - `disabled` (bool — written through; consumed by the action-surface
///   state-gate, not the renderer)
/// - `opacity` (number — multiplied into Paint.opacity)
/// - `x` / `y` / `width` / `height` (numbers — override the layout-engine
///   rect at emit time. Children of a width/height-bound parent do *not*
///   relayout; that needs the effect cache. For absolute-positioned
///   leaves this is enough to move them around.)
/// - `fill[0].color` (hex string — written into the first fill's color
///   field, defaulting `type` to `"solid"`)
fn apply_bindings(
    node: &mut Value,
    state: &jian_core::state::StateGraph,
) -> BindingOverrides {
    let mut overrides = BindingOverrides::default();
    let Some(obj) = node.as_object_mut() else {
        return overrides;
    };
    let bindings = match obj.get("bindings") {
        Some(Value::Object(b)) => b.clone(),
        _ => return overrides,
    };
    let node_id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    for (prop, expr_v) in &bindings {
        let Some(src) = expr_v.as_str() else { continue };
        let compiled = match jian_core::expression::Expression::compile(src) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let (value, _warns) = compiled.eval(state, None, node_id.as_deref());
        match prop.as_str() {
            "content" => {
                if let Some(s) = value.as_str() {
                    obj.insert("content".into(), Value::String(s.to_owned()));
                }
            }
            "visible" => {
                if let Some(b) = value.as_bool() {
                    obj.insert("visible".into(), Value::Bool(b));
                }
            }
            "disabled" => {
                if let Some(b) = value.as_bool() {
                    obj.insert("disabled".into(), Value::Bool(b));
                }
            }
            "opacity" => {
                if let Some(n) = number_from_runtime(&value) {
                    if let Some(num) = serde_json::Number::from_f64(n) {
                        obj.insert("opacity".into(), Value::Number(num));
                    }
                }
            }
            "x" => {
                if let Some(n) = number_from_runtime(&value) {
                    overrides.x = Some(n as f32);
                }
            }
            "y" => {
                if let Some(n) = number_from_runtime(&value) {
                    overrides.y = Some(n as f32);
                }
            }
            "width" => {
                if let Some(n) = number_from_runtime(&value) {
                    overrides.w = Some(n as f32);
                }
            }
            "height" => {
                if let Some(n) = number_from_runtime(&value) {
                    overrides.h = Some(n as f32);
                }
            }
            "fill[0].color" => {
                if let Some(s) = value.as_str() {
                    set_first_fill_color(obj, s);
                }
            }
            // Two-way input binding: project the bound state value
            // into the node's `value` field so `emit_text_input`
            // (and any future writable surfaces) repaint from
            // current state. Without this, a SetValue dispatch
            // mutates state but the input still shows the static
            // schema `value`. We coerce scalars to a string form
            // because the only consumer today (`emit_text_input`)
            // reads `value` as text. A null projection (missing
            // path, eval error, deliberately-null state) keeps the
            // static schema `value` rather than blanking it — that
            // way an author-set placeholder/seed isn't silently
            // wiped by a path that hasn't been seeded yet.
            "bind:value" => {
                if let Some(projected) = bind_value_to_string(&value) {
                    obj.insert("value".into(), Value::String(projected));
                }
            }
            _ => {}
        }
    }
    overrides
}

fn number_from_runtime(v: &jian_core::value::RuntimeValue) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        return Some(n);
    }
    v.as_i64().map(|i| i as f64)
}

/// Stringify a bound runtime value for an input's `value` field.
/// Strings come through unchanged; numbers / bools take their
/// natural display form; object / array values stringify to empty
/// so a misuse doesn't paint stale text. Null returns `None` —
/// the caller leaves the existing `value` alone, preserving any
/// static schema seed when the bound path hasn't been initialised.
fn bind_value_to_string(v: &jian_core::value::RuntimeValue) -> Option<String> {
    if v.is_null() {
        return None;
    }
    if let Some(s) = v.as_str() {
        return Some(s.to_owned());
    }
    if let Some(b) = v.as_bool() {
        return Some(b.to_string());
    }
    if let Some(i) = v.as_i64() {
        return Some(i.to_string());
    }
    if let Some(f) = v.as_f64() {
        return Some(f.to_string());
    }
    Some(String::new())
}

fn set_first_fill_color(obj: &mut serde_json::Map<String, Value>, color: &str) {
    let entry = obj
        .entry("fill".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    let arr = match entry.as_array_mut() {
        Some(a) => a,
        None => return,
    };
    if arr.is_empty() {
        arr.push(serde_json::json!({ "type": "solid", "color": color }));
        return;
    }
    // Only mutate the first fill when it's already a solid colour.
    // Gradient and image fills don't carry a flat `color` field, so
    // writing one would either be a silent no-op (renderer keeps
    // reading the gradient stops) or, worse, leave the node with a
    // bogus mixed shape. The binding name itself — `fill[0].color`
    // — implies a solid fill, so restricting to that contract keeps
    // the binding honest.
    let Some(first) = arr[0].as_object_mut() else { return };
    let kind = first
        .get("type")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    match kind.as_deref() {
        // No type yet → assume solid (matches the `arr.is_empty()`
        // branch above where we materialise a fresh solid fill).
        None => {
            first.insert("type".into(), Value::String("solid".into()));
            first.insert("color".into(), Value::String(color.to_owned()));
        }
        Some("solid") => {
            first.insert("color".into(), Value::String(color.to_owned()));
        }
        // Gradient / image / unknown types: leave untouched.
        _ => {}
    }
}

fn emit_for_node(r: jian_core::geometry::Rect, json: &Value, out: &mut Vec<DrawOp>) {
    let rect_logical = rect(r.min_x(), r.min_y(), r.size.width, r.size.height);

    // --- Image emission. Image nodes and `image` fills both paint
    // through `DrawOp::Image`, but they still want any drop-shadow
    // *under* and any stroke *around* the image. Compute shadow/stroke
    // up-front so the emit ordering is shadow → image → stroke even
    // when this branch returns early.
    let image_source = image_source_for(json);
    if let Some((source, opacity)) = image_source {
        let radii = corner_radii(json).unwrap_or_else(BorderRadii::zero);
        if let Some(shadow) = first_shadow(json) {
            out.push(DrawOp::ShadowedRect {
                rect: rect_logical,
                radii,
                shadow,
            });
        }
        out.push(DrawOp::Image {
            source,
            dst: rect_logical,
            opacity,
        });
        if let Some(stroke) = stroke_op(json) {
            // Image carries no built-in stroke; emit a stroke-only
            // rect on top so border styling round-trips. Rounded
            // corners use RoundedRect for a matching outline.
            let paint = Paint {
                fill: None,
                stroke: Some(stroke),
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
        return;
    }

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

    // --- Text-input: styled rectangle + value-or-placeholder text + caret.
    if json.get("type").and_then(|t| t.as_str()) == Some("text_input") {
        emit_text_input(rect_logical, r, json, out);
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

    if let Some(grad) = first_fill.and_then(try_radial_gradient) {
        out.push(DrawOp::RadialGradientRect {
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
        opacity: node_opacity(json),
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

/// Render a `text_input` node: background rect (using its fill /
/// stroke / cornerRadius) → text run for `value` (or placeholder when
/// value is empty) → 1px caret line at the run's end. Full IME and
/// focus painting live in the host once the gesture arena gains a
/// Focus recognizer; this is the static-frame approximation.
fn emit_text_input(
    rect_logical: jian_core::geometry::Rect,
    r: jian_core::geometry::Rect,
    json: &Value,
    out: &mut Vec<DrawOp>,
) {
    let radii = corner_radii(json).unwrap_or_else(BorderRadii::zero);
    let stroke = stroke_op(json);
    let fill = first_solid_color(json.get("fill"));
    if fill.is_some() || stroke.is_some() {
        let paint = Paint {
            fill,
            stroke,
            opacity: node_opacity(json),
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

    let value = json.get("value").and_then(|v| v.as_str()).unwrap_or("");
    let placeholder = json
        .get("placeholder")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let (text, is_placeholder) = if value.is_empty() {
        (placeholder, true)
    } else {
        (value, false)
    };

    let font_size = json
        .get("fontSize")
        .and_then(|v| v.as_f64())
        .unwrap_or(14.0) as f32;
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
    // Placeholder text gets dimmed; resolved value uses the input's
    // own foreground colour (defaulting to near-black when unset).
    let text_color = if is_placeholder {
        Color::rgba(0x66, 0x66, 0x66, 0xff)
    } else {
        Color::rgb(0x11, 0x11, 0x11)
    };

    if !text.is_empty() {
        out.push(DrawOp::Text(TextRun {
            content: text.to_owned(),
            font_family,
            font_size,
            font_weight,
            color: text_color,
            origin: point(r.min_x() + 6.0, r.min_y() + (r.size.height - font_size) / 2.0),
            max_width: (r.size.width - 12.0).max(0.0),
            align: TextAlign::Start,
            line_height: 0.0,
        }));
    }

    // Caret approximation: 1px-wide vertical line near the end of the
    // value text, or at the left padding when the field is empty.
    let caret_x = r.min_x() + 6.0 + (value.len() as f32) * font_size * 0.55;
    let caret_top = r.min_y() + (r.size.height - font_size) / 2.0;
    let caret_height = font_size;
    out.push(DrawOp::Rect {
        rect: rect(caret_x, caret_top, 1.0, caret_height),
        paint: Paint {
            fill: Some(Color::rgba(0x33, 0x33, 0x33, 0xa0)),
            stroke: None,
            opacity: node_opacity(json),
        },
    });
}

/// Resolve the node's effective opacity. `bindings.opacity` writes the
/// value in via `apply_bindings`; the schema's static `opacity` field
/// is the fallback. Defaults to 1.0 when neither is set or the value
/// isn't numeric.
fn node_opacity(json: &Value) -> f32 {
    json.get("opacity")
        .and_then(|v| v.as_f64())
        .map(|n| n.clamp(0.0, 1.0) as f32)
        .unwrap_or(1.0)
}

/// Treat `data:` strings as inline base64 payloads; everything else is
/// a host-resolved URL (the skia backend's image cache draws a grey
/// placeholder if no resolver is wired up).
fn classify_source(src: &str) -> ImageSource {
    if src.starts_with("data:") {
        ImageSource::DataUrl(src.to_owned())
    } else {
        ImageSource::Url(src.to_owned())
    }
}

/// Resolve which image source (if any) a node should paint with. Image
/// nodes win over image fills; fills only fire on non-image nodes with
/// `fill[0].type == "image"`. Returns `(source, opacity)`.
fn image_source_for(json: &Value) -> Option<(ImageSource, f32)> {
    if json.get("type").and_then(|t| t.as_str()) == Some("image") {
        let src = json.get("src").and_then(|v| v.as_str())?;
        let opacity = json
            .get("opacity")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0) as f32;
        return Some((classify_source(src), opacity));
    }
    let first_fill = json
        .get("fill")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())?;
    let obj = first_fill.as_object()?;
    if obj.get("type").and_then(|t| t.as_str()) != Some("image") {
        return None;
    }
    let url = obj.get("url").and_then(|v| v.as_str())?.to_owned();
    let opacity = obj.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    Some((classify_source(&url), opacity))
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

fn try_radial_gradient(fill: &Value) -> Option<RadialGradient> {
    let obj = fill.as_object()?;
    if obj.get("type").and_then(|t| t.as_str()) != Some("radial_gradient") {
        return None;
    }
    let cx = obj.get("cx").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
    let cy = obj.get("cy").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
    let radius = obj
        .get("radius")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5) as f32;
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
    let opacity = obj.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    Some(RadialGradient {
        cx,
        cy,
        radius,
        stops,
        opacity,
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
            obj.get("uniform")
                .and_then(|u| u.as_f64())
                .map(|n| n as f32)
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
                if let Some(t) = seg
                    .as_object()
                    .and_then(|o| o.get("text"))
                    .and_then(|t| t.as_str())
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
