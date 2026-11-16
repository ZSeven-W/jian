//! Standalone scene adapter for the shared motion triggers.

use super::scene::{apply_bindings, apply_live_widget_state, first_solid_color, stroke_op};
use super::WidgetRenderCtx;
use crate::action::services::AnimationProperty;
use crate::document::NodeKey;
use crate::geometry::Rect;
use crate::motion::{MotionOverrides, MotionSceneView};
use crate::Runtime;
use serde_json::{json, Value};

pub(crate) struct RuntimeMotionView<'a> {
    pub runtime: &'a Runtime,
    pub widgets: &'a WidgetRenderCtx<'a>,
    pub interaction: &'a PlaybackInteraction,
}

impl RuntimeMotionView<'_> {
    fn key(&self, id: &str) -> Option<NodeKey> {
        self.runtime.document.as_ref()?.tree.by_id.get(id).copied()
    }

    fn node(&self, id: &str) -> Option<(Rect, Value)> {
        let doc = self.runtime.document.as_ref()?;
        let key = self.key(id)?;
        let bounds = self.runtime.layout.node_rect(key)?;
        let mut node = serde_json::to_value(&doc.tree.nodes[key].schema).ok()?;
        let binding = apply_bindings(&mut node, &self.runtime.state, doc.schema.is_responsive());
        apply_live_widget_state(&mut node, self.widgets);
        self.interaction
            .apply(key, &mut node, self.widgets.focused_id);
        Some((binding.apply_to_rect(bounds), node))
    }
}

impl MotionSceneView for RuntimeMotionView<'_> {
    fn node_rect(&self, id: &str) -> Option<Rect> {
        self.node(id).map(|(rect, _)| rect)
    }

    fn property(&self, id: &str, property: &AnimationProperty) -> Option<Value> {
        use AnimationProperty::*;
        let (bounds, node) = self.node(id)?;
        Some(match property {
            TranslateX | TranslateY => json!(0.0),
            ScaleX | ScaleY => json!(1.0),
            Rotation | CornerRadius => json!(node[property.name()].as_f64().unwrap_or(0.0)),
            Opacity => json!(node["opacity"].as_f64().unwrap_or(1.0)),
            Fill => json!(crate::motion::interpolate::color_hex(first_solid_color(
                node.get("fill")
            )?)),
            Stroke => json!(crate::motion::interpolate::color_hex(
                stroke_op(&node)?.color
            )),
            X => json!(bounds.origin.x),
            Y => json!(bounds.origin.y),
            Width => json!(bounds.size.width),
            Height => json!(bounds.size.height),
            ToggleProgress => json!(if node["checked"].as_bool().unwrap_or(false) {
                1.0
            } else {
                0.0
            }),
            ShaderUniform(_) | Custom(_) => return None,
        })
    }

    fn root_rect(&self) -> Option<Rect> {
        let root = *self.runtime.document.as_ref()?.tree.roots.first()?;
        self.runtime.layout.node_rect(root)
    }

    fn switches(&self) -> Vec<(String, bool)> {
        let Some(doc) = self.runtime.document.as_ref() else {
            return Vec::new();
        };
        doc.tree
            .nodes
            .values()
            .filter_map(|node| {
                if !matches!(node.schema, jian_ops_schema::node::PenNode::Switch(_)) {
                    return None;
                }
                let id = crate::document::tree::node_schema_id(&node.schema);
                let (_, node) = self.node(id)?;
                Some((id.to_owned(), node["checked"].as_bool().unwrap_or(false)))
            })
            .collect()
    }

    fn subtree_contains(&self, ancestor: &str, id: &str) -> bool {
        let Some(doc) = self.runtime.document.as_ref() else {
            return false;
        };
        let ancestor = self.key(ancestor);
        let mut key = self.key(id);
        while let Some(current) = key {
            if Some(current) == ancestor {
                return true;
            }
            key = doc.tree.nodes[current].parent;
        }
        false
    }
}

#[derive(Default)]
pub(crate) struct PlaybackInteraction {
    pub hovered: Option<NodeKey>,
    pub pressed: std::collections::BTreeMap<u32, NodeKey>,
}

impl PlaybackInteraction {
    pub fn apply(&self, key: NodeKey, json: &mut Value, focused: Option<&str>) {
        use super::widget_style::{active_override, InteractionState};
        use jian_ops_schema::state_override::WidgetStates;

        let Some(states) = json.get("states").cloned() else {
            return;
        };
        let Ok(states) = serde_json::from_value::<WidgetStates>(states) else {
            return;
        };
        let interaction = InteractionState {
            hovered: self.hovered == Some(key),
            pressed: self.pressed.values().any(|node| *node == key),
            focused: json["id"]
                .as_str()
                .zip(focused)
                .is_some_and(|(id, focused)| id == focused),
            disabled: json["disabled"].as_bool().unwrap_or_default(),
        };
        if let Some(style) = active_override(Some(&states), interaction) {
            if let (Some(node), Ok(Value::Object(style))) =
                (json.as_object_mut(), serde_json::to_value(style))
            {
                node.extend(style);
            }
        }
    }
}

pub(crate) struct SceneMotion<'a> {
    pub overrides: &'a MotionOverrides,
    pub interaction: &'a PlaybackInteraction,
}
