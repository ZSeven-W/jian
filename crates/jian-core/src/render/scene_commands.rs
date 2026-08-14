//! Structured scene traversal for production render backends.

use super::scene::{
    active_tab_index, apply_bindings, apply_live_widget_state, emit_for_node, emit_live_text_input,
    is_tabs_node, WidgetRenderCtx,
};
use super::{DrawOp, RichTextPlan, ScenePaintCommand, ShadowSpec};
use crate::geometry::{Affine2, Rect};
use crate::scene::Color;
use serde_json::Value;
use std::collections::HashSet;

enum LayerEffect {
    Blur(f32),
    Shadow(ShadowSpec),
}

/// Collect a balanced backend command stream while preserving the stable flat
/// `DrawOp` collectors used by native and external consumers.
pub fn collect_scene_paint_commands_with_state(
    doc: &crate::document::RuntimeDocument,
    layout: &crate::layout::LayoutEngine,
    state: &crate::state::StateGraph,
) -> Vec<ScenePaintCommand> {
    collect(doc, layout, state, None)
}

/// Like [`collect_scene_paint_commands_with_state`] but paints live widget
/// state — typed text, caret, selection — instead of the schema's authored
/// `value`. Hosts that accept input MUST use this: without the context a
/// `text_input` renders what the document was authored with, so every edit is
/// invisible no matter how many frames are drawn.
pub fn collect_scene_paint_commands_with_widgets(
    doc: &crate::document::RuntimeDocument,
    layout: &crate::layout::LayoutEngine,
    state: &crate::state::StateGraph,
    widgets: &WidgetRenderCtx,
) -> Vec<ScenePaintCommand> {
    collect(doc, layout, state, Some(widgets))
}

fn collect(
    doc: &crate::document::RuntimeDocument,
    layout: &crate::layout::LayoutEngine,
    state: &crate::state::StateGraph,
    widgets: Option<&WidgetRenderCtx>,
) -> Vec<ScenePaintCommand> {
    let mut commands = Vec::with_capacity(doc.tree.nodes.len() * 2);
    let mut visited = HashSet::with_capacity(doc.tree.nodes.len());
    for &root in &doc.tree.roots {
        walk(
            doc,
            layout,
            state,
            widgets,
            root,
            &mut commands,
            &mut visited,
        );
    }
    commands
}

#[allow(clippy::too_many_arguments)]
fn walk(
    doc: &crate::document::RuntimeDocument,
    layout: &crate::layout::LayoutEngine,
    state: &crate::state::StateGraph,
    widgets: Option<&WidgetRenderCtx>,
    key: crate::document::NodeKey,
    commands: &mut Vec<ScenePaintCommand>,
    visited: &mut HashSet<crate::document::NodeKey>,
) {
    if !visited.insert(key) {
        return;
    }
    let Some(node) = doc.tree.nodes.get(key) else {
        return;
    };
    let Some(mut bounds) = layout.node_rect(key) else {
        return;
    };
    let Ok(mut json) = serde_json::to_value(&node.schema) else {
        return;
    };
    let overrides = apply_bindings(&mut json, state, doc.schema.is_responsive());
    if let Some(ctx) = widgets {
        apply_live_widget_state(&mut json, ctx);
    }
    if !json.get("visible").and_then(Value::as_bool).unwrap_or(true) {
        return;
    }
    bounds = overrides.apply_to_rect(bounds);

    let transformed = node_transform(&json, bounds);
    if let Some(transform) = transformed {
        commands.push(ScenePaintCommand::PushTransform(transform));
    }

    let effects = layer_effects(&json);
    let mut bounds_visited = HashSet::with_capacity(doc.tree.nodes.len());
    let content_bounds = subtree_content_bounds(
        doc,
        layout,
        state,
        widgets,
        key,
        bounds,
        &json,
        &mut bounds_visited,
    );
    let layer_bounds = nested_effect_bounds(content_bounds, &effects);
    for (effect, layer_bounds) in effects.iter().zip(layer_bounds) {
        match effect {
            LayerEffect::Blur(sigma) => commands.push(ScenePaintCommand::ApplyBlur(*sigma)),
            LayerEffect::Shadow(shadow) => {
                commands.push(ScenePaintCommand::ApplyShadow(shadow.clone()));
            }
        }
        commands.push(ScenePaintCommand::PushLayer(layer_bounds));
    }

    let mut ops = Vec::new();
    let mut text_runs = Vec::new();
    // Live widget render takes precedence, exactly as in the flat collector:
    // a text widget with runtime state paints that state, not the authored
    // value. Falls back when no context or no state exists for this node.
    let live = widgets
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
                    emit_live_text_input(bounds, &json, st, ctx, id, &mut ops);
                    Some(())
                }
                _ => None,
            }
        })
        .is_some();
    if !live {
        emit_for_node(
            bounds,
            &json,
            doc.schema.is_responsive(),
            Some(state),
            &mut ops,
            &mut text_runs,
            false,
        );
    }
    // A text field clips to its own box. Its content is independent of its
    // height — a short field can hold thousands of characters — and without
    // this the overflow paints straight down the page over whatever follows.
    // Only the command stream can express this; the flat DrawOp collectors
    // have no clip, which is why this lives here rather than in the emitter.
    let clips_text = matches!(
        json.get("type").and_then(|t| t.as_str()),
        Some("text_input" | "text_area" | "number_input")
    );
    if clips_text {
        commands.push(ScenePaintCommand::PushClip(bounds));
    }
    append_draws(commands, ops, text_runs);
    if clips_text {
        commands.push(ScenePaintCommand::Pop);
    }

    let clipped = clip_content(&json);
    if clipped {
        commands.push(ScenePaintCommand::PushClip(bounds));
    }
    let tabs_node = is_tabs_node(&json);
    let active_tab = active_tab_index(&json);
    for (index, &child) in node.children.iter().enumerate() {
        if tabs_node && active_tab != Some(index) {
            continue;
        }
        walk(doc, layout, state, widgets, child, commands, visited);
    }
    if clipped {
        commands.push(ScenePaintCommand::Pop);
    }

    commands.extend((0..effects.len()).map(|_| ScenePaintCommand::PopLayer));
    if transformed.is_some() {
        commands.push(ScenePaintCommand::Pop);
    }
}

fn append_draws(
    commands: &mut Vec<ScenePaintCommand>,
    ops: Vec<DrawOp>,
    text_runs: Vec<RichTextPlan>,
) {
    let mut text_runs = text_runs.into_iter().peekable();
    for (index, op) in ops.into_iter().enumerate() {
        if text_runs.peek().is_some_and(|plan| plan.op_index == index) {
            let plan = text_runs.next().expect("peeked rich text plan");
            let DrawOp::Text(run) = op else {
                debug_assert!(false, "rich text plan indexed a non-text op");
                commands.push(ScenePaintCommand::Draw(op));
                continue;
            };
            commands.push(ScenePaintCommand::RichText { run, plan });
        } else {
            commands.push(ScenePaintCommand::Draw(op));
        }
    }
}

fn node_transform(json: &Value, bounds: Rect) -> Option<Affine2> {
    let degrees = json.get("rotation").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let flip_x = json.get("flipX").and_then(Value::as_bool).unwrap_or(false);
    let flip_y = json.get("flipY").and_then(Value::as_bool).unwrap_or(false);
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

fn clip_content(json: &Value) -> bool {
    json.get("clipContent")
        .or_else(|| json.get("clip"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn layer_effects(json: &Value) -> Vec<LayerEffect> {
    let Some(effects) = json.get("effects").and_then(Value::as_array) else {
        return Vec::new();
    };
    effects
        .iter()
        .filter_map(|effect| match effect.get("type").and_then(Value::as_str) {
            Some("blur") => effect
                .get("radius")
                .and_then(Value::as_f64)
                .map(|radius| LayerEffect::Blur((radius as f32).max(0.0))),
            Some("shadow" | "drop-shadow")
                if !effect
                    .get("inner")
                    .and_then(Value::as_bool)
                    .unwrap_or(false) =>
            {
                Some(LayerEffect::Shadow(ShadowSpec {
                    dx: effect.get("offsetX").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    dy: effect.get("offsetY").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    blur: effect.get("blur").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    spread: effect.get("spread").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    color: effect
                        .get("color")
                        .and_then(Value::as_str)
                        .and_then(Color::from_hex)
                        .unwrap_or(Color::rgba(0, 0, 0, 128)),
                }))
            }
            _ => None,
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn subtree_content_bounds(
    doc: &crate::document::RuntimeDocument,
    layout: &crate::layout::LayoutEngine,
    state: &crate::state::StateGraph,
    widgets: Option<&WidgetRenderCtx>,
    key: crate::document::NodeKey,
    bounds: Rect,
    json: &Value,
    visited: &mut HashSet<crate::document::NodeKey>,
) -> Rect {
    if !visited.insert(key) {
        return bounds;
    }
    let mut aggregate = outset(bounds, stroke_outset(json));
    if clip_content(json) {
        return aggregate;
    }

    let Some(node) = doc.tree.nodes.get(key) else {
        return aggregate;
    };
    let tabs_node = is_tabs_node(json);
    let active_tab = active_tab_index(json);
    for (index, &child) in node.children.iter().enumerate() {
        if tabs_node && active_tab != Some(index) {
            continue;
        }
        if let Some(child_bounds) = painted_node_bounds(doc, layout, state, widgets, child, visited)
        {
            aggregate = union(aggregate, child_bounds);
        }
    }
    aggregate
}

fn painted_node_bounds(
    doc: &crate::document::RuntimeDocument,
    layout: &crate::layout::LayoutEngine,
    state: &crate::state::StateGraph,
    widgets: Option<&WidgetRenderCtx>,
    key: crate::document::NodeKey,
    visited: &mut HashSet<crate::document::NodeKey>,
) -> Option<Rect> {
    if visited.contains(&key) {
        return None;
    }
    let node = doc.tree.nodes.get(key)?;
    let mut bounds = layout.node_rect(key)?;
    let mut json = serde_json::to_value(&node.schema).ok()?;
    let overrides = apply_bindings(&mut json, state, doc.schema.is_responsive());
    if let Some(ctx) = widgets {
        apply_live_widget_state(&mut json, ctx);
    }
    if !json.get("visible").and_then(Value::as_bool).unwrap_or(true) {
        return None;
    }
    bounds = overrides.apply_to_rect(bounds);

    let content = subtree_content_bounds(doc, layout, state, widgets, key, bounds, &json, visited);
    let effects = layer_effects(&json);
    let painted = nested_effect_bounds(content, &effects)
        .first()
        .copied()
        .unwrap_or(content);
    Some(node_transform(&json, bounds).map_or(painted, |transform| {
        outer_transformed_rect(painted, &transform)
    }))
}

fn union(a: Rect, b: Rect) -> Rect {
    let min_x = a.min_x().min(b.min_x());
    let min_y = a.min_y().min(b.min_y());
    let max_x = a.max_x().max(b.max_x());
    let max_y = a.max_y().max(b.max_y());
    crate::geometry::rect(min_x, min_y, max_x - min_x, max_y - min_y)
}

fn outer_transformed_rect(bounds: Rect, transform: &Affine2) -> Rect {
    let corners = [
        transform.transform_point(bounds.origin),
        transform.transform_point(crate::geometry::point(bounds.max_x(), bounds.min_y())),
        transform.transform_point(crate::geometry::point(bounds.min_x(), bounds.max_y())),
        transform.transform_point(crate::geometry::point(bounds.max_x(), bounds.max_y())),
    ];
    let min_x = corners
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min);
    let min_y = corners
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = corners
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    crate::geometry::rect(min_x, min_y, max_x - min_x, max_y - min_y)
}

fn stroke_outset(json: &Value) -> f32 {
    let Some(thickness) = json
        .get("stroke")
        .and_then(Value::as_object)
        .and_then(|stroke| {
            stroke.get("thickness").and_then(|value| {
                value
                    .as_f64()
                    .or_else(|| value.get("uniform").and_then(Value::as_f64))
            })
        })
    else {
        return 0.0;
    };
    (thickness as f32).max(0.0) * 0.5
}

fn outset(mut bounds: Rect, amount: f32) -> Rect {
    let amount = amount.max(0.0);
    bounds.origin.x -= amount;
    bounds.origin.y -= amount;
    bounds.size.width += amount * 2.0;
    bounds.size.height += amount * 2.0;
    bounds
}

fn nested_effect_bounds(content: Rect, effects: &[LayerEffect]) -> Vec<Rect> {
    let mut layers = vec![content; effects.len()];
    let mut child_bounds = content;
    for (index, effect) in effects.iter().enumerate().rev() {
        child_bounds = expand_for_effect(child_bounds, effect);
        layers[index] = child_bounds;
    }
    layers
}

fn expand_for_effect(mut bounds: Rect, effect: &LayerEffect) -> Rect {
    let (left, top, right, bottom) = match effect {
        LayerEffect::Blur(sigma) => {
            let tail = sigma.max(0.0) * 3.0;
            (tail, tail, tail, tail)
        }
        LayerEffect::Shadow(shadow) => {
            let tail = shadow.blur.max(0.0) * 3.0 + shadow.spread.max(0.0);
            (
                tail + (-shadow.dx).max(0.0),
                tail + (-shadow.dy).max(0.0),
                tail + shadow.dx.max(0.0),
                tail + shadow.dy.max(0.0),
            )
        }
    };
    bounds.origin.x -= left;
    bounds.origin.y -= top;
    bounds.size.width += left + right;
    bounds.size.height += top + bottom;
    bounds
}
