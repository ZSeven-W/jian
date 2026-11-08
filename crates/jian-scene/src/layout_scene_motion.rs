//! Motion on a [`LayoutScene`]: read a node's current value for a motion
//! property, and paint a frame of motion overrides onto the active page.
//! Moved from OpenPencil's preview so every Jian host plays `.op` motion
//! through the same code.

use crate::layout_scene::{LayoutScene, SceneNode};
use crate::layout_scene_binding::apply_value;
use jian_core::action::services::AnimationProperty;
use jian_core::motion::interpolate::color_hex;
use jian_core::motion::{MotionOverrides, MotionSceneView};
use jian_core::value::RuntimeValue;
use jian_widgets::Color;
use serde_json::{json, Value};

/// `#rrggbb` (or `#rrggbbaa` when translucent) for a scene colour.
pub fn scene_color(color: Color) -> String {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    color_hex(jian_core::scene::Color::rgba(
        channel(color.r),
        channel(color.g),
        channel(color.b),
        channel(color.a),
    ))
}

/// The value `property` currently has on node `target` of the active page:
/// the baseline a motion animates from. `None` when the node is missing or
/// has nothing to animate (no fill, no switch widget, custom properties).
pub fn sample_scene_property(
    scene: &LayoutScene,
    target: &str,
    property: &AnimationProperty,
) -> Option<Value> {
    let node = scene.active_page()?.find(target)?;
    Some(match property {
        AnimationProperty::Opacity => json!(node.opacity),
        AnimationProperty::X => json!(node.bounds.origin.x),
        AnimationProperty::Y => json!(node.bounds.origin.y),
        AnimationProperty::Rotation => json!(node.rotation.to_degrees()),
        AnimationProperty::ScaleX | AnimationProperty::ScaleY => json!(1.0),
        // A visual offset is applied on top of the solved rect and never
        // stored on the scene node, so the sampled baseline is always 0.
        AnimationProperty::TranslateX | AnimationProperty::TranslateY => json!(0.0),
        AnimationProperty::Fill => Value::String(scene_color(node.fill?)),
        AnimationProperty::Stroke => Value::String(scene_color(node.stroke.as_ref()?.color)),
        AnimationProperty::CornerRadius => json!(node.corner_radius),
        AnimationProperty::Width => json!(node.bounds.size.x),
        AnimationProperty::Height => json!(node.bounds.size.y),
        AnimationProperty::ToggleProgress => {
            let widget = node.widget.as_ref()?;
            json!(widget
                .toggle_progress
                .unwrap_or(if widget.checked.unwrap_or(false) {
                    1.0
                } else {
                    0.0
                }))
        }
        AnimationProperty::ShaderUniform(_) | AnimationProperty::Custom(_) => return None,
    })
}

/// Paint one frame of motion overrides onto the active page. Each node's
/// overrides apply in binding application order (geometry before
/// translation), and a switch's toggle progress drives its widget directly.
pub fn apply_motion_overrides(scene: &mut LayoutScene, overrides: &MotionOverrides) {
    let Some(page) = scene.pages.get_mut(scene.active_page_index) else {
        return;
    };
    for node in &mut page.children {
        apply_node_overrides(node, overrides);
    }
}

fn apply_node_overrides(node: &mut SceneNode, overrides: &MotionOverrides) {
    for ((target, property), value) in overrides {
        if target == &node.id && *property == AnimationProperty::ToggleProgress {
            if let (Some(widget), Some(progress)) = (node.widget.as_mut(), value.as_f64()) {
                widget.toggle_progress = Some(progress as f32);
            }
            break;
        }
    }
    let mut node_overrides: Vec<_> = overrides
        .iter()
        .filter(|((target, _), _)| target == &node.id)
        .filter_map(|((_, property), value)| {
            property
                .binding_target()
                .map(|target| (target, RuntimeValue(value.clone())))
        })
        .collect();
    node_overrides.sort_by_key(|(target, _)| target.application_order());
    for (target, value) in node_overrides {
        apply_value(node, target, &value);
    }
    for child in &mut node.children {
        apply_node_overrides(child, overrides);
    }
}

/// The scene Motion triggers watch: OpenPencil's Preview and Jian hosts
/// both paint a `LayoutScene`, so they share this view.
impl MotionSceneView for LayoutScene {
    fn node_rect(&self, id: &str) -> Option<jian_core::geometry::Rect> {
        let node = self.active_page()?.find(id)?;
        Some(core_rect(node.bounds))
    }

    fn property(&self, id: &str, property: &AnimationProperty) -> Option<Value> {
        sample_scene_property(self, id, property)
    }

    fn root_rect(&self) -> Option<jian_core::geometry::Rect> {
        let root = self.active_page()?.children.first()?;
        Some(core_rect(root.bounds))
    }

    fn switches(&self) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        if let Some(page) = self.active_page() {
            collect_switches(&page.children, &mut out);
        }
        out
    }

    fn subtree_contains(&self, ancestor: &str, id: &str) -> bool {
        ancestor == id
            || self
                .active_page()
                .and_then(|page| page.find(ancestor))
                .is_some_and(|node| node.find(id).is_some())
    }
}

fn core_rect(rect: jian_widgets::Rect) -> jian_core::geometry::Rect {
    jian_core::geometry::rect(rect.origin.x, rect.origin.y, rect.size.x, rect.size.y)
}

fn collect_switches(nodes: &[SceneNode], out: &mut Vec<(String, bool)>) {
    for node in nodes {
        if let Some(widget) = node
            .widget
            .as_ref()
            .filter(|widget| widget.kind == "switch")
        {
            out.push((node.id.clone(), widget.checked.unwrap_or(false)));
        }
        collect_switches(&node.children, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_scene::{NodeKind, ScenePage, SceneWidget};
    use jian_widgets::Rect;

    fn scene() -> LayoutScene {
        let mut card = SceneNode::leaf("card", NodeKind::Rect);
        card.bounds = Rect::xywh(10.0, 20.0, 100.0, 40.0);
        card.fill = Some(Color::rgb_u8(255, 0, 0));
        let mut toggle = SceneNode::leaf("toggle", NodeKind::Rect);
        toggle.widget = Some(SceneWidget {
            kind: "switch".into(),
            checked: Some(true),
            ..SceneWidget::default()
        });
        card.children.push(toggle);
        LayoutScene {
            pages: vec![ScenePage {
                id: "page".into(),
                name: "Page".into(),
                children: vec![card],
            }],
            active_page_index: 0,
        }
    }

    fn key(id: &str, property: AnimationProperty) -> (String, AnimationProperty) {
        (id.to_owned(), property)
    }

    #[test]
    fn samples_read_the_animation_baseline() {
        let scene = scene();
        let sample = |id, property| sample_scene_property(&scene, id, &property);
        assert_eq!(sample("card", AnimationProperty::X), Some(json!(10.0)));
        assert_eq!(sample("card", AnimationProperty::Width), Some(json!(100.0)));
        assert_eq!(
            sample("card", AnimationProperty::TranslateY),
            Some(json!(0.0))
        );
        assert_eq!(sample("card", AnimationProperty::ScaleX), Some(json!(1.0)));
        assert_eq!(
            sample("card", AnimationProperty::Fill),
            Some(json!("#ff0000"))
        );
        assert_eq!(sample("card", AnimationProperty::Stroke), None);
        assert_eq!(
            sample("toggle", AnimationProperty::ToggleProgress),
            Some(json!(1.0))
        );
        assert_eq!(sample("missing", AnimationProperty::Opacity), None);
    }

    #[test]
    fn translucent_colours_keep_their_alpha() {
        assert_eq!(scene_color(Color::rgba_u8(0, 0, 255, 0.5)), "#0000ff80");
        assert_eq!(scene_color(Color::rgb_u8(0, 0, 255)), "#0000ff");
    }

    #[test]
    fn overrides_paint_nested_nodes_and_drive_switches() {
        let mut scene = scene();
        let overrides = MotionOverrides::from([
            (key("card", AnimationProperty::Opacity), json!(0.25)),
            (key("card", AnimationProperty::TranslateX), json!(5)),
            (key("toggle", AnimationProperty::ToggleProgress), json!(0.5)),
        ]);
        apply_motion_overrides(&mut scene, &overrides);
        let card = scene
            .active_page()
            .and_then(|page| page.find("card"))
            .unwrap();
        assert_eq!(card.opacity, 0.25);
        assert_eq!(card.bounds.origin.x, 15.0);
        let toggle = &card.children[0];
        assert_eq!(toggle.bounds.origin.x, 5.0);
        assert_eq!(
            toggle
                .widget
                .as_ref()
                .and_then(|widget| widget.toggle_progress),
            Some(0.5)
        );
    }

    #[test]
    fn the_scene_view_reads_the_active_page() {
        let scene = scene();
        let view: &dyn MotionSceneView = &scene;
        let rect = view.node_rect("card").expect("card");
        assert_eq!((rect.origin.x, rect.origin.y), (10.0, 20.0));
        assert_eq!((rect.size.width, rect.size.height), (100.0, 40.0));
        assert_eq!(view.root_rect(), Some(rect));
        assert_eq!(view.node_rect("missing"), None);
        assert_eq!(view.switches(), vec![("toggle".to_owned(), true)]);
        assert!(view.subtree_contains("card", "toggle"));
        assert!(view.subtree_contains("toggle", "toggle"));
        assert!(!view.subtree_contains("toggle", "card"));
        assert_eq!(
            view.property("card", &AnimationProperty::Opacity),
            Some(json!(1.0))
        );
    }
}
