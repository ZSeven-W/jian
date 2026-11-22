//! Write one bound or animated value onto a [`SceneNode`]: the
//! `BindingTarget` -> scene-field table that OpenPencil's binding overlay
//! and motion overrides both use. Moved from OpenPencil's preview so the
//! table sits next to jian-core's schema-side twin
//! (`jian_core::binding::application`).

use crate::layout_scene::{
    Effect, SceneFillLayer, SceneGradient, SceneNode, SceneStroke, SceneStrokeAlign,
};
use jian_core::binding::BindingTarget;
use jian_core::value::RuntimeValue;
use jian_widgets::Color;
use jian_widgets::ImageBlendMode;

/// Human-readable form of an expression result for scene text: strings
/// verbatim, null as empty, everything else as JSON (integers print without
/// a trailing `.0`).
pub fn display_string(value: &RuntimeValue) -> String {
    match &value.0 {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub(crate) fn uses_composite_opacity(node: &SceneNode) -> bool {
    !node.children.is_empty() || node.blend_mode != ImageBlendMode::Normal
}

pub fn apply_value(node: &mut SceneNode, target: BindingTarget, value: &RuntimeValue) {
    match target {
        BindingTarget::Content => {
            node.text = Some(display_string(value));
            node.text_runs.clear();
        }
        BindingTarget::Value => apply_widget_value(node, value),
        BindingTarget::Checked => {
            if let (Some(widget), Some(checked)) = (node.widget.as_mut(), value.as_bool()) {
                widget.checked = Some(checked);
            }
        }
        BindingTarget::SelectedValue => {
            if let (Some(widget), Some(selected)) = (node.widget.as_mut(), value.as_str()) {
                widget.value_str = Some(selected.to_owned());
            }
        }
        BindingTarget::Visible => {
            if let Some(visible) = value.as_bool() {
                node.hidden = !visible;
            }
        }
        BindingTarget::Opacity => {
            if let Some(opacity) = number(value) {
                // The loader leaves the paints of an isolated subtree at
                // full alpha. Fade the assembled subtree, not its own fill.
                let alpha = opacity.clamp(0.0, 1.0);
                if uses_composite_opacity(node) {
                    node.composite_opacity = alpha;
                } else {
                    set_leaf_opacity(node, alpha);
                }
            }
        }
        BindingTarget::Fill | BindingTarget::TextColor => {
            if let Some(color) = color(value) {
                node.fill = Some(color);
                if let Some(SceneFillLayer::Solid {
                    color: layer_color, ..
                }) = node.fill_layers.first_mut()
                {
                    *layer_color = color;
                }
            }
        }
        BindingTarget::Stroke => {
            if let Some(color) = color(value) {
                if let Some(stroke) = node.stroke.as_mut() {
                    stroke.color = color;
                } else {
                    node.stroke = Some(SceneStroke {
                        color,
                        width: 1.0,
                        sides: None,
                        align: SceneStrokeAlign::Center,
                    });
                }
            }
        }
        BindingTarget::CornerRadius => {
            if let Some(radius) = number(value) {
                node.corner_radius = radius.max(0.0);
                node.corner_radii = None;
            }
        }
        BindingTarget::X => {
            if let Some(x) = number(value) {
                node.bounds.origin.x = x;
            }
        }
        BindingTarget::Y => {
            if let Some(y) = number(value) {
                node.bounds.origin.y = y;
            }
        }
        // Paint-only visual offsets (the preview's `transform: translate`):
        // the whole subtree shifts on screen while the solved layout and
        // the hit-test tree stay where they are, which is what lets
        // `$scroll`-driven parallax and rise-in entrances pass the
        // PaintOnly gate.
        BindingTarget::TranslateX => {
            if let Some(dx) = number(value) {
                translate_subtree(node, dx, 0.0);
            }
        }
        BindingTarget::TranslateY => {
            if let Some(dy) = number(value) {
                translate_subtree(node, 0.0, dy);
            }
        }
        BindingTarget::Width => {
            if let Some(width) = number(value) {
                node.bounds.size.x = width.max(0.0);
            }
        }
        BindingTarget::Height => {
            if let Some(height) = number(value) {
                node.bounds.size.y = height.max(0.0);
            }
        }
        BindingTarget::Rotation => {
            if let Some(degrees) = number(value) {
                node.rotation = degrees.to_radians();
            }
        }
        BindingTarget::ScaleX => {
            if let Some(scale) = number(value) {
                scale_axis(node, scale, true);
            }
        }
        BindingTarget::ScaleY => {
            if let Some(scale) = number(value) {
                scale_axis(node, scale, false);
            }
        }
        BindingTarget::Variant | BindingTarget::ActiveState => {
            apply_structural_state(node, value);
        }
    }
}

fn apply_widget_value(node: &mut SceneNode, value: &RuntimeValue) {
    let Some(widget) = node.widget.as_mut() else {
        return;
    };
    match widget.kind.as_str() {
        "switch" | "checkbox" => widget.checked = value.as_bool(),
        "slider" | "progress" | "number_input" => widget.value_num = number(value),
        _ => widget.value_str = value.as_str().map(str::to_owned),
    }
}

fn apply_structural_state(node: &mut SceneNode, value: &RuntimeValue) {
    if let Some(widget) = node.widget.as_mut() {
        match widget.kind.as_str() {
            "switch" | "checkbox" => widget.checked = value.as_bool(),
            _ => widget.value_str = value.as_str().map(str::to_owned),
        }
        return;
    }
    let Some(active) = value.as_str() else {
        return;
    };
    for child in &mut node.children {
        child.hidden = child.id != active;
    }
}

fn number(value: &RuntimeValue) -> Option<f32> {
    value
        .as_f64()
        .map(|number| number as f32)
        .or_else(|| value.as_i64().map(|number| number as f32))
}

fn color(value: &RuntimeValue) -> Option<Color> {
    let color = jian_core::scene::Color::from_hex(value.as_str()?)?;
    Some(Color::from_jian(color))
}

fn scale_axis(node: &mut SceneNode, scale: f32, horizontal: bool) {
    let scale = scale.max(0.0);
    if horizontal {
        let center = node.bounds.origin.x + node.bounds.size.x / 2.0;
        node.bounds.size.x *= scale;
        node.bounds.origin.x = center - node.bounds.size.x / 2.0;
    } else {
        let center = node.bounds.origin.y + node.bounds.size.y / 2.0;
        node.bounds.size.y *= scale;
        node.bounds.origin.y = center - node.bounds.size.y / 2.0;
    }
}

/// Shift a scene subtree by a visual offset: bounds, cached aggregate
/// bounds, polygon points and path anchors all move together so
/// strokes, fills and text stay attached to the node. Non-finite or zero
/// offsets leave the subtree untouched.
pub fn translate_subtree(node: &mut SceneNode, dx: f32, dy: f32) {
    if !dx.is_finite() || !dy.is_finite() || (dx == 0.0 && dy == 0.0) {
        return;
    }
    node.bounds.origin.x += dx;
    node.bounds.origin.y += dy;
    if node.aggregate_bounds_cache.size.x > 0.0 || node.aggregate_bounds_cache.size.y > 0.0 {
        node.aggregate_bounds_cache.origin.x += dx;
        node.aggregate_bounds_cache.origin.y += dy;
    }
    for point in &mut node.points {
        point.x += dx;
        point.y += dy;
    }
    for anchor in &mut node.path_anchors {
        anchor.pos.x += dx;
        anchor.pos.y += dy;
        if let Some(handle) = &mut anchor.handle_in {
            handle.x += dx;
            handle.y += dy;
        }
        if let Some(handle) = &mut anchor.handle_out {
            handle.x += dx;
            handle.y += dy;
        }
    }
    for child in &mut node.children {
        translate_subtree(child, dx, dy);
    }
}

/// A leaf's paint opacity is baked into its fill, gradient, shader, stroke,
/// text-run and shadow colours at scene build, so writing `opacity` alone
/// would not repaint it. Rescale the baked paint to the new alpha. Fading to
/// zero goes through the composite layer instead, which keeps the paint
/// recoverable for a later fade back in.
fn set_leaf_opacity(node: &mut SceneNode, alpha: f32) {
    if alpha <= 0.0 {
        node.composite_opacity = 0.0;
        return;
    }
    let base = node.opacity.clamp(0.0, 1.0);
    if base <= 0.0 {
        // Nothing baked to rescale; the loader isolates such leaves.
        node.composite_opacity = alpha;
        return;
    }
    let k = alpha / base;
    if (k - 1.0).abs() > f32::EPSILON {
        rescale_leaf_paint(node, k);
    }
    node.opacity = alpha;
    node.composite_opacity = 1.0;
}

impl SceneNode {
    /// Multiply this node's own painted alpha by `k`: the colours baked at
    /// scene build (fill, gradient, shader, stroke, text runs, shadows) and
    /// `opacity`, which painters apply to fill stacks and images. Exporters
    /// that flatten a leaf's composite layer into its paint use this.
    pub fn scale_paint_alpha(&mut self, k: f32) {
        rescale_leaf_paint(self, k);
        self.opacity = (self.opacity * k).clamp(0.0, 1.0);
    }
}

fn rescale_leaf_paint(node: &mut SceneNode, k: f32) {
    let scale = |alpha: f32| (alpha * k).clamp(0.0, 1.0);
    if let Some(color) = node.fill.as_mut() {
        color.a = scale(color.a);
    }
    if let Some(gradient) = node.gradient.as_mut() {
        match gradient {
            SceneGradient::Linear { opacity, .. }
            | SceneGradient::Radial { opacity, .. }
            | SceneGradient::Mesh { opacity, .. } => *opacity = scale(*opacity),
        }
    }
    if let Some(shader) = node.shader.as_mut() {
        shader.opacity = scale(shader.opacity);
    }
    if let Some(stroke) = node.stroke.as_mut() {
        stroke.color.a = scale(stroke.color.a);
    }
    for run in &mut node.text_runs {
        if let Some(color) = run.fill.as_mut() {
            color.a = scale(color.a);
        }
    }
    for effect in &mut node.effects {
        if let Effect::DropShadow(shadow) = effect {
            shadow.color.a = scale(shadow.color.a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_scene::NodeKind;
    use jian_widgets::{Point2D, Rect};
    use serde_json::json;

    fn rect_node(id: &str) -> SceneNode {
        let mut node = SceneNode::leaf(id, NodeKind::Rect);
        node.bounds = Rect::xywh(10.0, 20.0, 100.0, 40.0);
        node
    }

    fn value(value: serde_json::Value) -> RuntimeValue {
        RuntimeValue(value)
    }

    #[test]
    fn leaf_opacity_rebakes_its_paint_and_fades_to_zero_through_the_layer() {
        // The loader bakes a leaf's opacity into its fill / stroke / text
        // colours, so animating it must go through the paint-time layer.
        let mut node = rect_node("leaf");
        node.opacity = 0.5;
        node.fill = Some(Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 0.5,
        });
        apply_value(&mut node, BindingTarget::Opacity, &value(json!(0.25)));
        assert_eq!(node.opacity, 0.25);
        assert!(
            (node.fill.unwrap().a - 0.25).abs() < 1e-6,
            "paint is rebaked"
        );
        apply_value(&mut node, BindingTarget::Opacity, &value(json!(0.0)));
        assert_eq!(node.composite_opacity, 0.0);
        assert!(
            (node.fill.unwrap().a - 0.25).abs() < 1e-6,
            "zero keeps paint"
        );
        apply_value(&mut node, BindingTarget::Opacity, &value(json!(1.0)));
        assert_eq!(node.composite_opacity, 1.0);
        assert!(
            (node.fill.unwrap().a - 1.0).abs() < 1e-6,
            "fades back above base"
        );
    }

    #[test]
    fn text_and_paint_targets_write_their_scene_fields() {
        let mut node = rect_node("n");
        apply_value(&mut node, BindingTarget::Content, &value(json!(42)));
        assert_eq!(node.text.as_deref(), Some("42"));
        apply_value(&mut node, BindingTarget::Opacity, &value(json!(1.7)));
        assert_eq!(node.opacity, 1.0);
        apply_value(&mut node, BindingTarget::Fill, &value(json!("#ff000080")));
        let fill = node.fill.expect("fill");
        assert_eq!((fill.r, fill.g, fill.b), (1.0, 0.0, 0.0));
        assert!((fill.a - 128.0 / 255.0).abs() < 1e-6);
        apply_value(&mut node, BindingTarget::Stroke, &value(json!("#00ff00")));
        assert_eq!(node.stroke.as_ref().map(|stroke| stroke.width), Some(1.0));
        apply_value(&mut node, BindingTarget::CornerRadius, &value(json!(-4)));
        assert_eq!(node.corner_radius, 0.0);
        apply_value(&mut node, BindingTarget::Rotation, &value(json!(90)));
        assert!((node.rotation - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn translation_moves_the_whole_subtree_and_scale_keeps_the_centre() {
        let mut parent = rect_node("parent");
        let mut child = rect_node("child");
        child.points = vec![Point2D::new(1.0, 2.0)];
        parent.children.push(child);
        apply_value(&mut parent, BindingTarget::TranslateY, &value(json!(16)));
        assert_eq!(parent.bounds.origin.y, 36.0);
        assert_eq!(parent.children[0].bounds.origin.y, 36.0);
        assert_eq!(parent.children[0].points[0].y, 18.0);
        apply_value(&mut parent, BindingTarget::ScaleX, &value(json!(0.5)));
        assert_eq!(parent.bounds.size.x, 50.0);
        assert_eq!(parent.bounds.origin.x, 35.0);
    }

    #[test]
    fn translation_ignores_non_finite_offsets() {
        let mut node = rect_node("n");
        translate_subtree(&mut node, f32::NAN, 4.0);
        translate_subtree(&mut node, 0.0, f32::INFINITY);
        assert_eq!(node.bounds, Rect::xywh(10.0, 20.0, 100.0, 40.0));
        translate_subtree(&mut node, -2.0, 4.0);
        assert_eq!(node.bounds, Rect::xywh(8.0, 24.0, 100.0, 40.0));
    }

    #[test]
    fn active_state_shows_only_the_named_child() {
        let mut parent = rect_node("tabs");
        parent.children = vec![rect_node("a"), rect_node("b")];
        apply_value(&mut parent, BindingTarget::ActiveState, &value(json!("b")));
        assert!(parent.children[0].hidden);
        assert!(!parent.children[1].hidden);
        assert_eq!(display_string(&value(json!(null))), "");
    }
}
