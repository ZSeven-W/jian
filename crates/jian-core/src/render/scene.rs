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
//!
//! This module is wasm-clean: it depends only on `jian-core`'s own
//! document / layout / state / render types plus `serde_json`, so the
//! future `op-canvas` wasm crate can reuse it without pulling in a
//! desktop host crate.

use crate::geometry::{point, rect, Point};
use crate::render::{
    BorderRadii, DrawOp, GradientStop, ImageSource, LinearGradient, MeshGradient, Paint,
    PathCommand, RadialGradient, ShaderSpec, ShaderUniform, ShadowSpec, StrokeOp, TextAlign,
    TextRun,
};
use crate::scene::Color;
use jian_ops_schema::node::text::canonical_line_height_multiplier;
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
    doc: &crate::document::RuntimeDocument,
    layout: &crate::layout::LayoutEngine,
) -> Vec<DrawOp> {
    let mut out = Vec::with_capacity(doc.tree.nodes.len());
    let mut visited: std::collections::HashSet<crate::document::NodeKey> =
        std::collections::HashSet::with_capacity(doc.tree.nodes.len());
    for &root in &doc.tree.roots {
        let offset = root_offset_for(doc, layout, root);
        walk(
            doc,
            layout,
            root,
            offset,
            None,
            None,
            &mut out,
            &mut visited,
        );
    }
    out
}

/// Like `collect_draws` but evaluates `bindings.<prop>` expressions
/// against `state` so dynamic content (e.g. `Count: ${$app.count}`)
/// reflects the live runtime value. Without this path the walker
/// emits the schema's static `content` and counter / live-state
/// labels never refresh.
pub fn collect_draws_with_state(
    doc: &crate::document::RuntimeDocument,
    layout: &crate::layout::LayoutEngine,
    state: &crate::state::StateGraph,
) -> Vec<DrawOp> {
    let mut out = Vec::with_capacity(doc.tree.nodes.len());
    let mut visited: std::collections::HashSet<crate::document::NodeKey> =
        std::collections::HashSet::with_capacity(doc.tree.nodes.len());
    for &root in &doc.tree.roots {
        let offset = root_offset_for(doc, layout, root);
        walk(
            doc,
            layout,
            root,
            offset,
            Some(state),
            None,
            &mut out,
            &mut visited,
        );
    }
    out
}

/// Extra context the walker needs to paint *live* interactive widgets
/// (typed text, caret blink, selection, focus ring). `None` — the
/// default for static thumbnails / the editor design surface — renders
/// the same box with the schema's placeholder/value and no live caret.
pub struct WidgetRenderCtx<'a> {
    pub states: &'a crate::widget_state::WidgetStateStore,
    pub theme: &'a crate::render::widget_style::WidgetTheme,
    pub focused_id: Option<&'a str>,
    pub now_ms: u64,
}

/// Like [`collect_draws_with_state`] but also paints live widget runtime
/// state (caret / selection / typed text) from `ctx`. Used by the OP
/// canvas preview mode and by `jian run`.
pub fn collect_draws_with_widgets(
    doc: &crate::document::RuntimeDocument,
    layout: &crate::layout::LayoutEngine,
    state: &crate::state::StateGraph,
    ctx: &WidgetRenderCtx,
) -> Vec<DrawOp> {
    let mut out = Vec::with_capacity(doc.tree.nodes.len());
    let mut visited: std::collections::HashSet<crate::document::NodeKey> =
        std::collections::HashSet::with_capacity(doc.tree.nodes.len());
    for &root in &doc.tree.roots {
        let offset = root_offset_for(doc, layout, root);
        walk(
            doc,
            layout,
            root,
            offset,
            Some(state),
            Some(ctx),
            &mut out,
            &mut visited,
        );
    }
    out
}

/// Authored `(x, y)` on a document ROOT, `(0.0, 0.0)` when unset.
///
/// `taffy` computes every document root as an independent tree with no
/// containing block, so `Position::Absolute` insets driven by
/// `layout::resolve::node_to_style` (from the same `x`/`y` schema
/// fields) are honoured for *children* but silently ignored for a
/// root itself — `LayoutEngine::node_rect` therefore reports every
/// root at a root-relative `(0, 0)` regardless of its authored
/// origin. That's intentional: `node_rect` must stay root-relative so
/// OpenPencil (which applies each root's offset itself) doesn't
/// double-offset. This walker is the single seam that turns the
/// authored root origin into an actual draw-position translation,
/// applied uniformly to the whole subtree below `root`.
///
/// KNOWN LIMITATION: only *draws* are translated. The spatial index
/// and pointer dispatch (`Runtime::rebuild_spatial` /
/// `dispatch_pointer` / slider scrub) still hit-test root-relative
/// `node_rect`s by the same contract (external consumers such as
/// OpenPencil translate root offsets themselves — offset-aware
/// runtime hit-testing would double-translate for them). A live
/// multi-root document with non-zero authored root offsets therefore
/// draws offset but hit-tests unoffset in jian-host-desktop; the fix
/// belongs at the host pointer-translation seam (tracked follow-up).
fn root_offset_for(
    doc: &crate::document::RuntimeDocument,
    layout: &crate::layout::LayoutEngine,
    root: crate::document::NodeKey,
) -> (f32, f32) {
    if layout.is_origin_normalized(root) {
        return (0.0, 0.0);
    }
    doc.tree
        .nodes
        .get(root)
        .and_then(|node| crate::layout::resolve::explicit_position(&node.schema))
        .unwrap_or((0.0, 0.0))
}

#[allow(clippy::too_many_arguments)]
fn walk(
    doc: &crate::document::RuntimeDocument,
    layout: &crate::layout::LayoutEngine,
    key: crate::document::NodeKey,
    root_offset: (f32, f32),
    state: Option<&crate::state::StateGraph>,
    widgets: Option<&WidgetRenderCtx>,
    out: &mut Vec<DrawOp>,
    visited: &mut std::collections::HashSet<crate::document::NodeKey>,
) {
    // Skip already-painted keys so a child cycle (NodeData.children
    // is `pub`, so a buggy mutation could install one) doesn't blow
    // the stack or paint the same node twice.
    if !visited.insert(key) {
        return;
    }
    let Some(node) = doc.tree.nodes.get(key) else {
        return;
    };
    // `node_rect` is root-relative by contract (see `root_offset_for`);
    // translate by the enclosing root's authored origin here, uniformly
    // across the whole subtree, so descendants keep their correct
    // relative position within the root while the root itself lands at
    // its authored `(x, y)` instead of always at `(0, 0)`.
    let r = layout.node_rect(key).map(|r| {
        rect(
            r.origin.x + root_offset.0,
            r.origin.y + root_offset.1,
            r.size.width,
            r.size.height,
        )
    });
    let mut json = serde_json::to_value(&node.schema).ok();

    let mut overrides = BindingOverrides::default();
    if let (Some(_), Some(j), Some(state)) = (r, json.as_mut(), state) {
        overrides = apply_bindings(j, state, doc.schema.is_responsive());
    }

    if let Some(json) = json.as_ref() {
        let visible = json
            .get("visible")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if !visible {
            // `visible: false` (whether static or via bindings) drops
            // the subtree — children of an invisible parent never paint.
            return;
        }
    }

    if let (Some(r), Some(json)) = (r, &json) {
        let r = overrides.apply_to_rect(r);
        // Live widget render: a focused/edited text widget with runtime
        // state paints from that state (typed text + caret + selection)
        // instead of the schema's static value. Falls back to the static
        // emit when no live state exists for this node.
        let handled = widgets
            .filter(|_| {
                matches!(
                    json.get("type").and_then(|t| t.as_str()),
                    Some("text_input" | "text_area" | "number_input")
                )
            })
            .and_then(|ctx| {
                let id = json.get("id")?.as_str()?;
                match ctx.states.get(id)? {
                    crate::widget_state::WidgetState::TextInput(st) => {
                        emit_live_text_input(r, json, st, ctx, id, out);
                        Some(())
                    }
                    _ => None,
                }
            })
            .is_some();
        if !handled {
            emit_for_node(r, json, doc.schema.is_responsive(), state, out);
        }
    }

    for &child in &node.children {
        walk(
            doc,
            layout,
            child,
            root_offset,
            state,
            widgets,
            out,
            visited,
        );
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
    fn apply_to_rect(self, r: crate::geometry::Rect) -> crate::geometry::Rect {
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
    state: &crate::state::StateGraph,
    responsive: bool,
) -> BindingOverrides {
    // Legacy documents keep today's draw-time rect overrides (M1c
    // suppresses them only for responsive docs, where geometry comes
    // from the installed layout) — and today's string-only `content`
    // coercion: widening it to numbers/bools would change legacy
    // rendered output, violating the §1.1 bit-identical promise.
    let allow_rect_overrides = !responsive;
    let mut overrides = BindingOverrides::default();
    let Some(obj) = node.as_object_mut() else {
        return overrides;
    };
    let bindings = match obj.get("bindings") {
        Some(Value::Object(b)) => b.clone(),
        _ => return overrides,
    };
    let node_id = obj.get("id").and_then(|v| v.as_str()).map(str::to_owned);
    for (prop, expr_v) in &bindings {
        let Some(src) = expr_v.as_str() else { continue };
        let compiled = match crate::expression::Expression::compile(src) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let (value, _warns) = compiled.eval(state, None, node_id.as_deref());
        match prop.as_str() {
            "content" => {
                let projected = if responsive {
                    bound_scalar_to_string(&value)
                } else {
                    // Legacy: strings only (pre-M1 behavior).
                    value.as_str().map(str::to_owned)
                };
                if let Some(projected) = projected {
                    obj.insert("content".into(), Value::String(projected));
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
            "x" if allow_rect_overrides => {
                overrides.x = number_from_runtime(&value).map(|n| n as f32)
            }
            "y" if allow_rect_overrides => {
                overrides.y = number_from_runtime(&value).map(|n| n as f32)
            }
            "width" if allow_rect_overrides => {
                overrides.w = number_from_runtime(&value).map(|n| n as f32)
            }
            "height" if allow_rect_overrides => {
                overrides.h = number_from_runtime(&value).map(|n| n as f32)
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
                if let Some(projected) = bound_scalar_to_string(&value) {
                    obj.insert("value".into(), Value::String(projected));
                }
            }
            _ => {}
        }
    }
    overrides
}

fn number_from_runtime(v: &crate::value::RuntimeValue) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        return Some(n);
    }
    v.as_i64().map(|i| i as f64)
}

/// Stringify a bound runtime value for textual `content` / `value` fields.
/// Strings come through unchanged; numbers / bools take their
/// natural display form; object / array values stringify to empty
/// so a misuse doesn't paint stale text. Null returns `None` —
/// the caller leaves the existing `value` alone, preserving any
/// static schema seed when the bound path hasn't been initialised.
fn bound_scalar_to_string(v: &crate::value::RuntimeValue) -> Option<String> {
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
    let Some(first) = arr[0].as_object_mut() else {
        return;
    };
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

fn emit_for_node(
    r: crate::geometry::Rect,
    json: &Value,
    responsive: bool,
    state: Option<&crate::state::StateGraph>,
    out: &mut Vec<DrawOp>,
) {
    let rect_logical = rect(r.min_x(), r.min_y(), r.size.width, r.size.height);

    // --- Image emission. Image nodes and `image` fills both paint
    // through `DrawOp::Image`, but they still want any drop-shadow
    // *under* and any stroke *around* the image. Compute shadow/stroke
    // up-front so the emit ordering is shadow → image → stroke even
    // when this branch returns early.
    let image_source = image_source_for(json, responsive, state);
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

    // --- Text-family inputs: styled rectangle + value/placeholder + caret.
    if let Some(k) = json.get("type").and_then(|t| t.as_str()) {
        if matches!(k, "text_input" | "text_area" | "number_input") {
            emit_text_input(rect_logical, r, json, out);
            return;
        }
        // --- Non-text family widgets paint composite visuals from props.
        if matches!(
            k,
            "switch" | "checkbox" | "slider" | "progress" | "select" | "radio_group"
        ) {
            emit_widget_visual(k, rect_logical, r, json, out);
            return;
        }
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

    if let Some(grad) = first_fill.and_then(try_mesh_gradient) {
        out.push(DrawOp::MeshGradientRect {
            rect: rect_logical,
            radii,
            gradient: grad,
            stroke,
        });
        return;
    }

    if let Some(shader) = first_fill.and_then(try_shader) {
        out.push(DrawOp::ShaderRect {
            rect: rect_logical,
            radii,
            shader,
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
    rect_logical: crate::geometry::Rect,
    r: crate::geometry::Rect,
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

    let pad_x = 6.0_f32;

    // --- text_area: wrap the value and stack each line. ---
    if json.get("type").and_then(|t| t.as_str()) == Some("text_area") {
        let lines = wrap_for_box(text, r.size.width, font_size, pad_x);
        let visible = clamp_visible_lines(json, &lines);
        let line_height = font_size * 1.3;
        let max_width = (r.size.width - pad_x * 2.0).max(0.0);
        for (i, line) in visible.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            out.push(DrawOp::Text(TextRun {
                content: line.to_string(),
                font_family: font_family.clone(),
                font_size,
                font_weight,
                color: text_color,
                origin: point(
                    r.min_x() + pad_x,
                    r.min_y() + pad_x + i as f32 * line_height,
                ),
                max_width,
                align: TextAlign::Start,
                line_height: 0.0,
            }));
        }
        // Caret approximation: end of the last wrapped line.
        let last = visible.last().map(String::as_str).unwrap_or("");
        let caret_x = r.min_x() + pad_x + last.chars().count() as f32 * font_size * 0.55;
        let caret_top = r.min_y() + pad_x + visible.len().saturating_sub(1) as f32 * line_height;
        out.push(DrawOp::Rect {
            rect: rect(caret_x, caret_top, 1.0, font_size),
            paint: Paint {
                fill: Some(Color::rgba(0x33, 0x33, 0x33, 0xa0)),
                stroke: None,
                opacity: node_opacity(json),
            },
        });
        return;
    }

    if !text.is_empty() {
        out.push(DrawOp::Text(TextRun {
            content: text.to_owned(),
            font_family,
            font_size,
            font_weight,
            color: text_color,
            origin: point(
                r.min_x() + 6.0,
                r.min_y() + (r.size.height - font_size) / 2.0,
            ),
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

/// Column budget for wrapping multi-line text inside a box of `width`.
/// Uses the same per-column glyph width the static caret uses
/// (`font_size * 0.55` px per column) so wrap positions and caret x stay
/// consistent. Returns the wrapped lines (always at least one).
fn wrap_for_box(text: &str, width: f32, font_size: f32, pad_x: f32) -> Vec<String> {
    let inner = (width - 2.0 * pad_x).max(0.0);
    let col_px = (font_size * 0.55).max(1.0);
    let max_cols = (inner / col_px).floor().max(1.0) as usize;
    crate::text_wrap::wrap_lines(text, max_cols)
}

/// Apply a `maxVisibleLines` cap to wrapped lines. When present and the
/// wrapped output is taller, we keep the LAST N lines so the caret line
/// (which sits at the end of the value) stays visible. Returns an owned
/// slice copy for simplicity (line counts here are small).
fn clamp_visible_lines(json: &Value, lines: &[String]) -> Vec<String> {
    let cap = json
        .get("maxVisibleLines")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .filter(|&n| n > 0);
    match cap {
        Some(n) if lines.len() > n => lines[lines.len() - n..].to_vec(),
        _ => lines.to_vec(),
    }
}

/// Live render of a text widget driven by its runtime [`crate::text_input::TextInputState`]
/// (typed text, blinking caret, selection) rather than the schema's
/// static `value`. Used by preview mode via [`collect_draws_with_widgets`].
/// Caret + selection only paint when the node is focused.
fn emit_live_text_input(
    r: crate::geometry::Rect,
    json: &Value,
    st: &crate::text_input::TextInputState,
    ctx: &WidgetRenderCtx,
    id: &str,
    out: &mut Vec<DrawOp>,
) {
    let rect_logical = rect(r.min_x(), r.min_y(), r.size.width, r.size.height);
    let radii = corner_radii(json).unwrap_or_else(BorderRadii::zero);
    let focused = ctx.focused_id == Some(id);

    // --- authored background box ---
    let fill = first_solid_color(json.get("fill"));
    let base_stroke = stroke_op(json);
    if fill.is_some() || base_stroke.is_some() {
        let paint = Paint {
            fill,
            stroke: base_stroke,
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
    let pad_x = 6.0_f32;
    let text_top = r.min_y() + (r.size.height - font_size) / 2.0;
    let char_w = font_size * 0.55;
    // Crude monospace-ish x for a byte offset (mirrors the static
    // caret approximation; real glyph metrics land with the painter).
    let x_at = |byte: usize, s: &str| -> f32 {
        let b = byte.min(s.len());
        r.min_x() + pad_x + s[..b].chars().count() as f32 * char_w
    };

    let live = st.text();
    // Inline the IME preedit at the caret for display.
    let display: String = match st.composition() {
        Some(c) if !c.text.is_empty() => {
            let caret = st.caret().min(live.len());
            let mut s = String::with_capacity(live.len() + c.text.len());
            s.push_str(&live[..caret]);
            s.push_str(&c.text);
            s.push_str(&live[caret..]);
            s
        }
        _ => live.to_owned(),
    };
    let placeholder = json
        .get("placeholder")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let (text, is_placeholder) = if display.is_empty() {
        (placeholder.to_owned(), true)
    } else {
        (display, false)
    };

    // --- text_area: multi-line live render (wrap the display text). ---
    // Precise per-line caret/selection geometry is deferred; an MVP caret
    // is placed at the end of the last visible wrapped line so the typing
    // position stays on screen.
    if json.get("type").and_then(|t| t.as_str()) == Some("text_area") {
        let lines = wrap_for_box(&text, r.size.width, font_size, pad_x);
        let visible = clamp_visible_lines(json, &lines);
        let line_height = font_size * 1.3;
        let max_width = (r.size.width - pad_x * 2.0).max(0.0);
        let color = if is_placeholder {
            Color::rgba(0x66, 0x66, 0x66, 0xff)
        } else {
            Color::rgb(0x11, 0x11, 0x11)
        };
        for (i, line) in visible.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            out.push(DrawOp::Text(TextRun {
                content: line.to_string(),
                font_family: font_family.clone(),
                font_size,
                font_weight,
                color,
                origin: point(
                    r.min_x() + pad_x,
                    r.min_y() + pad_x + i as f32 * line_height,
                ),
                max_width,
                align: TextAlign::Start,
                line_height: 0.0,
            }));
        }
        if focused && st.highlight_range().is_none() && st.caret_visible(ctx.now_ms) {
            let last = visible.last().map(String::as_str).unwrap_or("");
            let cx = r.min_x() + pad_x + last.chars().count() as f32 * char_w;
            let cy = r.min_y() + pad_x + visible.len().saturating_sub(1) as f32 * line_height;
            out.push(DrawOp::Rect {
                rect: rect(cx, cy, 1.0, font_size),
                paint: Paint {
                    fill: Some(Color::rgba(0x33, 0x33, 0x33, 0xff)),
                    stroke: None,
                    opacity: 1.0,
                },
            });
        }
        return;
    }

    // --- selection highlight (behind text) ---
    if focused {
        if let Some((a, b)) = st.highlight_range() {
            let x0 = x_at(a, live);
            let x1 = x_at(b, live);
            if x1 > x0 {
                out.push(DrawOp::Rect {
                    rect: rect(x0, text_top, x1 - x0, font_size),
                    paint: Paint {
                        fill: Some(ctx.theme.selection),
                        stroke: None,
                        opacity: 1.0,
                    },
                });
            }
        }
    }

    // --- text run ---
    if !text.is_empty() {
        let color = if is_placeholder {
            Color::rgba(0x66, 0x66, 0x66, 0xff)
        } else {
            Color::rgb(0x11, 0x11, 0x11)
        };
        out.push(DrawOp::Text(TextRun {
            content: text,
            font_family,
            font_size,
            font_weight,
            color,
            origin: point(r.min_x() + pad_x, text_top),
            max_width: (r.size.width - pad_x * 2.0).max(0.0),
            align: TextAlign::Start,
            line_height: 0.0,
        }));
    }

    // --- caret: focused, collapsed selection, blink-visible ---
    if focused && st.highlight_range().is_none() && st.caret_visible(ctx.now_ms) {
        let cx = match st.composition() {
            Some(c) => {
                // The host-supplied composition cursor is a byte offset
                // that may not be UTF-8 boundary-aligned; clamp it down to
                // the nearest boundary before slicing (else `c.text[..pre]`
                // panics on a multi-byte preedit).
                let pre = crate::text_input::prev_char_boundary(&c.text, c.cursor);
                x_at(st.caret(), live) + c.text[..pre].chars().count() as f32 * char_w
            }
            None => x_at(st.caret(), live),
        };
        out.push(DrawOp::Rect {
            rect: rect(cx, text_top, 1.0, font_size),
            paint: Paint {
                fill: Some(Color::rgba(0x33, 0x33, 0x33, 0xff)),
                stroke: None,
                opacity: 1.0,
            },
        });
    }
}

/// Static composite visuals for the non-text family widgets, driven by
/// the schema props (`checked` / `value` / `min` / `max`). In preview
/// the walker's live branch can override these from runtime state; this
/// is the design-surface / static-frame rendering. No oval primitive
/// exists, so circular knobs are rounded rects with radius = size/2.
fn emit_widget_visual(
    kind: &str,
    rect_logical: crate::geometry::Rect,
    r: crate::geometry::Rect,
    json: &Value,
    out: &mut Vec<DrawOp>,
) {
    let accent = first_solid_color(json.get("fill")).unwrap_or(Color::rgb(0x3b, 0x82, 0xf6));
    let track_off = Color::rgb(0xd1, 0xd5, 0xdb);
    let knob = Color::rgb(0xff, 0xff, 0xff);
    let opacity = node_opacity(json);
    let (x, y, w, h) = (r.min_x(), r.min_y(), r.size.width, r.size.height);
    let solid = |c: Color| Paint {
        fill: Some(c),
        stroke: None,
        opacity,
    };

    match kind {
        "switch" => {
            let on = json_bool(json, "checked");
            out.push(DrawOp::RoundedRect {
                rect: rect_logical,
                radii: BorderRadii::uniform(h / 2.0),
                paint: solid(if on { accent } else { track_off }),
            });
            let d = (h - 4.0).max(2.0);
            let kx = if on { x + w - d - 2.0 } else { x + 2.0 };
            out.push(DrawOp::RoundedRect {
                rect: rect(kx, y + 2.0, d, d),
                radii: BorderRadii::uniform(d / 2.0),
                paint: Paint {
                    fill: Some(knob),
                    stroke: None,
                    opacity: 1.0,
                },
            });
        }
        "checkbox" => {
            let on = json_bool(json, "checked");
            let radii = corner_radii(json).unwrap_or_else(|| BorderRadii::uniform(4.0));
            let box_stroke = stroke_op(json).unwrap_or(StrokeOp {
                color: track_off,
                width: 1.5,
            });
            out.push(DrawOp::RoundedRect {
                rect: rect_logical,
                radii,
                paint: Paint {
                    fill: if on { Some(accent) } else { None },
                    stroke: Some(box_stroke),
                    opacity,
                },
            });
            if on {
                out.push(DrawOp::Path {
                    commands: vec![
                        PathCommand::MoveTo(point(x + w * 0.24, y + h * 0.52)),
                        PathCommand::LineTo(point(x + w * 0.42, y + h * 0.70)),
                        PathCommand::LineTo(point(x + w * 0.76, y + h * 0.30)),
                    ],
                    paint: Paint {
                        fill: None,
                        stroke: Some(StrokeOp {
                            color: knob,
                            width: 2.0,
                        }),
                        opacity: 1.0,
                    },
                });
            }
        }
        "slider" => {
            let (min, max, _step) = slider_props(json);
            let v = json_number(json, "value").unwrap_or(min);
            let frac = if max > min {
                ((v - min) / (max - min)).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            let track_h = 4.0_f32;
            let cy = y + h / 2.0;
            out.push(DrawOp::RoundedRect {
                rect: rect(x, cy - track_h / 2.0, w, track_h),
                radii: BorderRadii::uniform(track_h / 2.0),
                paint: solid(track_off),
            });
            if frac > 0.0 {
                out.push(DrawOp::RoundedRect {
                    rect: rect(x, cy - track_h / 2.0, w * frac, track_h),
                    radii: BorderRadii::uniform(track_h / 2.0),
                    paint: Paint {
                        fill: Some(accent),
                        stroke: None,
                        opacity: 1.0,
                    },
                });
            }
            let d = h.clamp(10.0, 16.0);
            let kx = (x + w * frac - d / 2.0).clamp(x, x + w - d);
            out.push(DrawOp::RoundedRect {
                rect: rect(kx, cy - d / 2.0, d, d),
                radii: BorderRadii::uniform(d / 2.0),
                paint: Paint {
                    fill: Some(knob),
                    stroke: Some(StrokeOp {
                        color: track_off,
                        width: 1.0,
                    }),
                    opacity: 1.0,
                },
            });
        }
        "progress" => {
            let max = json_number(json, "max").unwrap_or(100.0);
            let v = json_number(json, "value").unwrap_or(0.0);
            let frac = if max > 0.0 {
                (v / max).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            let radii = BorderRadii::uniform(h / 2.0);
            out.push(DrawOp::RoundedRect {
                rect: rect_logical,
                radii,
                paint: solid(track_off),
            });
            if frac > 0.0 {
                out.push(DrawOp::RoundedRect {
                    rect: rect(x, y, w * frac, h),
                    radii,
                    paint: Paint {
                        fill: Some(accent),
                        stroke: None,
                        opacity: 1.0,
                    },
                });
            }
        }
        "select" => {
            let radii = corner_radii(json).unwrap_or_else(|| BorderRadii::uniform(6.0));
            let box_stroke = stroke_op(json).unwrap_or(StrokeOp {
                color: track_off,
                width: 1.0,
            });
            out.push(DrawOp::RoundedRect {
                rect: rect_logical,
                radii,
                paint: Paint {
                    fill: first_solid_color(json.get("fill")),
                    stroke: Some(box_stroke),
                    opacity,
                },
            });
            let selected = json.get("value").and_then(|v| v.as_str());
            let label = selected
                .and_then(|v| select_label_for(json, v))
                .or_else(|| {
                    json.get("placeholder")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                });
            if let Some(text) = label.filter(|s| !s.is_empty()) {
                let fs = 14.0_f32;
                out.push(DrawOp::Text(TextRun {
                    content: text,
                    font_family: String::new(),
                    font_size: fs,
                    font_weight: 400,
                    color: if selected.is_some() {
                        Color::rgb(0x11, 0x11, 0x11)
                    } else {
                        Color::rgba(0x66, 0x66, 0x66, 0xff)
                    },
                    origin: point(x + 8.0, y + (h - fs) / 2.0),
                    max_width: (w - 36.0).max(0.0),
                    align: TextAlign::Start,
                    line_height: 0.0,
                }));
            }
            // Down chevron on the trailing edge.
            let cw = 9.0_f32;
            let cx = x + w - 20.0;
            let cyc = y + h / 2.0;
            out.push(DrawOp::Path {
                commands: vec![
                    PathCommand::MoveTo(point(cx, cyc - cw * 0.22)),
                    PathCommand::LineTo(point(cx + cw / 2.0, cyc + cw * 0.33)),
                    PathCommand::LineTo(point(cx + cw, cyc - cw * 0.22)),
                ],
                paint: Paint {
                    fill: None,
                    stroke: Some(StrokeOp {
                        color: Color::rgb(0x66, 0x66, 0x66),
                        width: 1.5,
                    }),
                    opacity: 1.0,
                },
            });
        }
        "radio_group" => {
            let selected = json.get("value").and_then(|v| v.as_str());
            if let Some(opts) = json.get("options").and_then(|o| o.as_array()) {
                let n = opts.len().max(1);
                let row_h = (h / n as f32).clamp(0.0, 28.0);
                let d = 14.0_f32;
                let fs = 14.0_f32;
                for (i, opt) in opts.iter().enumerate() {
                    let ov = opt.get("value").and_then(|v| v.as_str()).unwrap_or("");
                    let ol = opt.get("label").and_then(|v| v.as_str()).unwrap_or(ov);
                    let on = selected == Some(ov);
                    let ry = y + i as f32 * row_h + (row_h - d) / 2.0;
                    out.push(DrawOp::RoundedRect {
                        rect: rect(x + 2.0, ry, d, d),
                        radii: BorderRadii::uniform(d / 2.0),
                        paint: Paint {
                            fill: if on { Some(accent) } else { None },
                            stroke: Some(StrokeOp {
                                color: track_off,
                                width: 1.5,
                            }),
                            opacity,
                        },
                    });
                    if on {
                        let inner = d * 0.4;
                        out.push(DrawOp::RoundedRect {
                            rect: rect(
                                x + 2.0 + (d - inner) / 2.0,
                                ry + (d - inner) / 2.0,
                                inner,
                                inner,
                            ),
                            radii: BorderRadii::uniform(inner / 2.0),
                            paint: Paint {
                                fill: Some(knob),
                                stroke: None,
                                opacity: 1.0,
                            },
                        });
                    }
                    out.push(DrawOp::Text(TextRun {
                        content: ol.to_owned(),
                        font_family: String::new(),
                        font_size: fs,
                        font_weight: 400,
                        color: Color::rgb(0x11, 0x11, 0x11),
                        origin: point(x + 2.0 + d + 8.0, ry + (d - fs) / 2.0),
                        max_width: (w - d - 14.0).max(0.0),
                        align: TextAlign::Start,
                        line_height: 0.0,
                    }));
                }
            }
        }
        _ => {}
    }
}

/// Look up a select option's display label by its `value`.
fn select_label_for(json: &Value, value: &str) -> Option<String> {
    json.get("options")?.as_array()?.iter().find_map(|o| {
        (o.get("value").and_then(|v| v.as_str()) == Some(value)).then(|| {
            o.get("label")
                .and_then(|v| v.as_str())
                .unwrap_or(value)
                .to_owned()
        })
    })
}

/// Literal boolean prop (expression strings read as `false`).
fn json_bool(json: &Value, key: &str) -> bool {
    matches!(json.get(key), Some(Value::Bool(true)))
}

/// Literal numeric prop (expression strings read as `None`).
fn json_number(json: &Value, key: &str) -> Option<f64> {
    json.get(key).and_then(|v| v.as_f64())
}

/// `(min, max, step)` from a slider's schema props (defaults 0/100/1).
fn slider_props(json: &Value) -> (f64, f64, f64) {
    (
        json_number(json, "min").unwrap_or(0.0),
        json_number(json, "max").unwrap_or(100.0),
        json_number(json, "step").unwrap_or(1.0),
    )
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
fn classify_source(
    src: &str,
    responsive: bool,
    state: Option<&crate::state::StateGraph>,
) -> ImageSource {
    if src.starts_with("data:") && responsive {
        ImageSource::Url(super::image_store::data_url_key(src))
    } else if src.starts_with("data:") {
        ImageSource::DataUrl(src.to_owned())
    } else {
        ImageSource::Url(
            state
                .and_then(|state| state.image_key(src))
                .unwrap_or_else(|| src.to_owned()),
        )
    }
}

/// Resolve which image source (if any) a node should paint with. Image
/// nodes win over image fills; fills only fire on non-image nodes with
/// `fill[0].type == "image"`. Returns `(source, opacity)`.
fn image_source_for(
    json: &Value,
    responsive: bool,
    state: Option<&crate::state::StateGraph>,
) -> Option<(ImageSource, f32)> {
    if json.get("type").and_then(|t| t.as_str()) == Some("image") {
        let src = json.get("src").and_then(|v| v.as_str())?;
        let opacity = json.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
        return Some((classify_source(src, responsive, state), opacity));
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
    Some((classify_source(&url, responsive, state), opacity))
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
    let radius = obj.get("radius").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
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

/// Parse a `mesh_gradient` fill entry into a `MeshGradient` (row-major
/// colour grid). Requires `rows >= 2` and `cols >= 2`. Each `stops[]`
/// entry carries `row`/`col` indices; vertices missing from the array
/// fall back to transparent black so a sparse mesh still triangulates.
fn try_mesh_gradient(fill: &Value) -> Option<MeshGradient> {
    let obj = fill.as_object()?;
    if obj.get("type").and_then(|t| t.as_str()) != Some("mesh_gradient") {
        return None;
    }
    let rows = obj.get("rows").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let cols = obj.get("cols").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if rows < 2 || cols < 2 {
        return None;
    }
    let stops_arr = obj.get("stops")?.as_array()?;
    let mut colors = vec![Color::rgba(0, 0, 0, 0); (rows * cols) as usize];
    let mut any = false;
    for s in stops_arr {
        let so = s.as_object()?;
        let row = so.get("row").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let col = so.get("col").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        if row >= rows || col >= cols {
            continue;
        }
        let hex = so.get("color")?.as_str()?;
        let color = Color::from_hex(hex)?;
        colors[(row * cols + col) as usize] = color;
        any = true;
    }
    if !any {
        return None;
    }
    let opacity = obj.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    Some(MeshGradient {
        rows,
        cols,
        colors,
        opacity,
    })
}

/// Parse a `shader` fill entry into a [`ShaderSpec`]. Requires a
/// non-empty `sksl` source string. Uniforms (optional) are resolved
/// here so the backend never re-walks JSON: a number → `float`, a
/// number array → `vec*`, a hex string → a 4-float premultiplied-RGBA
/// `vec4`. The `fallback` solid colour (used on compile failure) is the
/// first `color`-typed uniform if any, else mid-gray. The SkSL source
/// itself stays untrusted — it is NOT validated here; the backend
/// compiles it behind a non-panicking `Result`.
fn try_shader(fill: &Value) -> Option<ShaderSpec> {
    let obj = fill.as_object()?;
    if obj.get("type").and_then(|t| t.as_str()) != Some("shader") {
        return None;
    }
    let sksl = obj.get("sksl")?.as_str()?.to_string();
    if sksl.trim().is_empty() {
        return None;
    }
    let mut uniforms: Vec<ShaderUniform> = Vec::new();
    let mut fallback: Option<Color> = None;
    if let Some(map) = obj.get("uniforms").and_then(|u| u.as_object()) {
        for (name, val) in map {
            match val {
                Value::Number(n) => {
                    if let Some(f) = n.as_f64() {
                        uniforms.push(ShaderUniform {
                            name: name.clone(),
                            values: vec![f as f32],
                        });
                    }
                }
                Value::Array(arr) => {
                    let values: Vec<f32> = arr
                        .iter()
                        .filter_map(|v| v.as_f64())
                        .map(|f| f as f32)
                        .collect();
                    if !values.is_empty() {
                        uniforms.push(ShaderUniform {
                            name: name.clone(),
                            values,
                        });
                    }
                }
                Value::String(hex) => {
                    if let Some(c) = Color::from_hex(hex) {
                        // Expand the colour to a premultiplied-RGBA vec4.
                        let a = c.a() as f32 / 255.0;
                        uniforms.push(ShaderUniform {
                            name: name.clone(),
                            values: vec![
                                (c.r() as f32 / 255.0) * a,
                                (c.g() as f32 / 255.0) * a,
                                (c.b() as f32 / 255.0) * a,
                                a,
                            ],
                        });
                        if fallback.is_none() {
                            fallback = Some(c);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let opacity = obj.get("opacity").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    Some(ShaderSpec {
        sksl,
        uniforms,
        opacity,
        // Mid-gray when no colour uniform exists, so a compile failure
        // still paints a visible block instead of vanishing.
        fallback: fallback.unwrap_or(Color::rgb(128, 128, 128)),
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

fn try_text(json: &Value, r: crate::geometry::Rect) -> Option<DrawOp> {
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
    let line_height =
        canonical_line_height_multiplier(json.get("lineHeight").and_then(|v| v.as_f64()))
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
    use crate::Runtime;

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
    fn direct_scene_rejects_pixel_like_line_height_for_explicit_text_box() {
        // The scene walker reads a JSON projection rather than TextNode
        // directly. Keep this path on the same canonical multiplier semantics
        // as layout and the OpenPencil paint adapter.
        let rt = doc_with(
            r##"{ "formatVersion":"1.0", "version":"1.0.0", "id":"x",
                 "app": { "name":"x", "version":"1", "id":"x" },
                 "children": [
                   { "type":"text", "id":"bad", "width":180, "height":52,
                     "textGrowth":"fixed-width-height",
                     "content":"First line\nSecond line", "fontSize":14,
                     "lineHeight":17 },
                   { "type":"text", "id":"valid", "width":180, "height":52,
                     "content":"Valid multiplier", "fontSize":14,
                     "lineHeight":1.5 }
                 ]}"##,
        );
        let ops = collect_draws(rt.document.as_ref().unwrap(), &rt.layout);
        let text_runs: Vec<&TextRun> = ops
            .iter()
            .filter_map(|op| match op {
                DrawOp::Text(run) => Some(run),
                _ => None,
            })
            .collect();

        assert_eq!(text_runs.len(), 2);
        assert_eq!(
            text_runs[0].line_height, 0.0,
            "pixel-like lineHeight must use the renderer default"
        );
        assert_eq!(
            text_runs[1].line_height, 1.5,
            "valid unitless multiplier should remain authored"
        );
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

    #[test]
    fn live_text_input_renders_typed_text_and_blinking_caret() {
        let mut rt = doc_with(
            r##"{ "version":"1.1", "formatVersion":"1.1", "id":"x",
                 "app": { "name":"x", "version":"1", "id":"x" },
                 "children": [
                   { "type":"text_input", "id":"e", "width":200, "height":40,
                     "fill":[{ "type":"solid", "color":"#ffffff" }] }
                 ]}"##,
        );
        rt.focus_next().unwrap();
        rt.dispatch_text_input("hi").unwrap();
        let theme = crate::render::widget_style::WidgetTheme::default();
        let is_caret = |op: &DrawOp| matches!(op, DrawOp::Rect { rect, .. } if (rect.size.width - 1.0).abs() < 0.01);

        // Focused + start of blink cycle → typed text + caret painted.
        let ctx = WidgetRenderCtx {
            states: &rt.widget_states,
            theme: &theme,
            focused_id: Some("e"),
            now_ms: 0,
        };
        let ops =
            collect_draws_with_widgets(rt.document.as_ref().unwrap(), &rt.layout, &rt.state, &ctx);
        assert!(
            ops.iter()
                .any(|op| matches!(op, DrawOp::Text(t) if t.content == "hi")),
            "live typed text should render"
        );
        assert!(ops.iter().any(is_caret), "focused caret should render");

        // Half a blink period later → caret hidden.
        let ctx_off = WidgetRenderCtx { now_ms: 600, ..ctx };
        let ops_off = collect_draws_with_widgets(
            rt.document.as_ref().unwrap(),
            &rt.layout,
            &rt.state,
            &ctx_off,
        );
        assert!(
            !ops_off.iter().any(is_caret),
            "caret should be hidden mid-blink-off"
        );

        // Unfocused → no caret, but text still shows.
        let ctx_blur = WidgetRenderCtx {
            focused_id: None,
            now_ms: 0,
            ..ctx
        };
        let ops_blur = collect_draws_with_widgets(
            rt.document.as_ref().unwrap(),
            &rt.layout,
            &rt.state,
            &ctx_blur,
        );
        assert!(
            !ops_blur.iter().any(is_caret),
            "unfocused caret must not render"
        );
        assert!(ops_blur
            .iter()
            .any(|op| matches!(op, DrawOp::Text(t) if t.content == "hi")));
    }

    #[test]
    fn family_widget_static_visuals_render() {
        let rt = doc_with(
            r##"{ "version":"1.1", "formatVersion":"1.1", "id":"x",
                 "app": { "name":"x", "version":"1", "id":"x" },
                 "children": [
                   { "type":"frame", "id":"root", "width":400, "height":300,
                     "layout":"vertical",
                     "children": [
                       { "type":"switch",   "id":"sw", "width":44,  "height":24, "checked":true },
                       { "type":"checkbox", "id":"cb", "width":18,  "height":18, "checked":true },
                       { "type":"slider",   "id":"sl", "width":200, "height":20,
                         "min":0, "max":10, "value":5 },
                       { "type":"progress", "id":"pg", "width":200, "height":8,
                         "value":40, "max":100 }
                     ]}
                 ]}"##,
        );
        let ops = collect_draws(rt.document.as_ref().unwrap(), &rt.layout);
        // Checked checkbox draws its tick as a Path.
        assert!(
            ops.iter().any(|op| matches!(op, DrawOp::Path { .. })),
            "checkbox tick should be a Path op"
        );
        // switch (track+knob) + checkbox box + slider (track+fill+knob) +
        // progress (track+fill) → many rounded-rect parts.
        let rounded = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::RoundedRect { .. }))
            .count();
        assert!(
            rounded >= 6,
            "expected composite rounded-rect widget parts, got {rounded}"
        );
    }

    #[test]
    fn text_area_static_value_wraps_into_multiple_lines() {
        // A long single-line value in a narrow box must wrap to several
        // Text ops (one per wrapped line), unlike single-line text_input.
        let rt = doc_with(
            r##"{ "version":"1.1", "formatVersion":"1.1", "id":"x",
                 "app": { "name":"x", "version":"1", "id":"x" },
                 "children": [
                   { "type":"text_area", "id":"ta", "width":120, "height":120,
                     "fontSize":14,
                     "value":"the quick brown fox jumps over the lazy dog",
                     "fill":[{ "type":"solid", "color":"#ffffff" }] }
                 ]}"##,
        );
        let ops = collect_draws(rt.document.as_ref().unwrap(), &rt.layout);
        let text_ops = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::Text(_)))
            .count();
        assert!(
            text_ops >= 2,
            "text_area should wrap into multiple Text ops, got {text_ops}"
        );
    }

    #[test]
    fn text_area_honors_max_visible_lines_cap() {
        // Explicit `\n` newlines produce 4 lines; maxVisibleLines:2 keeps
        // only the last 2 (caret line stays visible).
        let rt = doc_with(
            r##"{ "version":"1.1", "formatVersion":"1.1", "id":"x",
                 "app": { "name":"x", "version":"1", "id":"x" },
                 "children": [
                   { "type":"text_area", "id":"ta", "width":400, "height":120,
                     "fontSize":14, "maxVisibleLines":2,
                     "value":"alpha\nbravo\ncharlie\ndelta",
                     "fill":[{ "type":"solid", "color":"#ffffff" }] }
                 ]}"##,
        );
        let ops = collect_draws(rt.document.as_ref().unwrap(), &rt.layout);
        let texts: Vec<&str> = ops
            .iter()
            .filter_map(|op| match op {
                DrawOp::Text(t) => Some(t.content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 2, "cap to last 2 lines, got {texts:?}");
        assert!(texts.contains(&"charlie") && texts.contains(&"delta"));
        assert!(!texts.contains(&"alpha"));
    }

    #[test]
    fn live_text_area_wraps_typed_text_into_multiple_lines() {
        // Live preview path: typed text longer than the box wraps to many
        // Text ops with a single end-of-last-line caret.
        let mut rt = doc_with(
            r##"{ "version":"1.1", "formatVersion":"1.1", "id":"x",
                 "app": { "name":"x", "version":"1", "id":"x" },
                 "children": [
                   { "type":"text_area", "id":"ta", "width":120, "height":120,
                     "fontSize":14,
                     "fill":[{ "type":"solid", "color":"#ffffff" }] }
                 ]}"##,
        );
        rt.focus_next().unwrap();
        rt.dispatch_text_input("the quick brown fox jumps over the lazy dog")
            .unwrap();
        let theme = crate::render::widget_style::WidgetTheme::default();
        let ctx = WidgetRenderCtx {
            states: &rt.widget_states,
            theme: &theme,
            focused_id: Some("ta"),
            now_ms: 0,
        };
        let ops =
            collect_draws_with_widgets(rt.document.as_ref().unwrap(), &rt.layout, &rt.state, &ctx);
        let text_ops = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::Text(_)))
            .count();
        assert!(
            text_ops >= 2,
            "live text_area should wrap into multiple Text ops, got {text_ops}"
        );
    }

    #[test]
    fn root_authored_offset_translates_its_whole_subtree() {
        // Two document ROOTS (not children of a shared frame) each author
        // an explicit x/y. taffy has no containing block for a root, so
        // `Position::Absolute` insets on a root are dropped by
        // `LayoutEngine::node_rect` (by design — `node_rect` must stay
        // root-relative for OpenPencil, which applies each root's offset
        // itself). The scene walker is the one seam that must place each
        // root's subtree at its authored origin before it's drawn.
        let rt = doc_with(
            r##"{ "formatVersion":"1.0", "version":"1.0.0", "id":"x",
                 "app": { "name":"x", "version":"1", "id":"x" },
                 "children": [
                   { "type":"rectangle", "id":"a", "x":0, "y":0,
                     "width":50, "height":50,
                     "fill":[{ "type":"solid", "color":"#ff0000" }] },
                   { "type":"rectangle", "id":"b", "x":140, "y":20,
                     "width":50, "height":50,
                     "fill":[{ "type":"solid", "color":"#00ff00" }] }
                 ]}"##,
        );
        let ops = collect_draws(rt.document.as_ref().unwrap(), &rt.layout);
        assert_eq!(ops.len(), 2);
        let rects: Vec<crate::geometry::Rect> = ops
            .iter()
            .map(|op| match op {
                DrawOp::Rect { rect, .. } => *rect,
                other => panic!("expected Rect, got {:?}", other),
            })
            .collect();
        assert_eq!(
            rects[0].origin,
            point(0.0, 0.0),
            "root 'a' has no authored offset, draws at the origin"
        );
        assert_eq!(
            rects[1].origin,
            point(140.0, 20.0),
            "root 'b' authors x=140,y=20 — its subtree must draw at that origin, \
             not collapse onto root 'a' at (0,0)"
        );
    }
}
