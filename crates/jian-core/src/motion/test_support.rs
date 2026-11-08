//! Shared fixtures for the motion unit tests.

use crate::action::services::AnimationProperty;
use crate::geometry::Rect;
use crate::motion::scene_view::MotionSceneView;
use crate::motion::timeline::MotionObserver;
use crate::Runtime;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// A hand-built scene: node rects, painted property values, switches, and
/// a parent map for subtree checks.
#[derive(Default)]
pub struct StubScene {
    pub rects: BTreeMap<String, Rect>,
    pub values: BTreeMap<(String, String), Value>,
    pub switches: Vec<(String, bool)>,
    pub parents: BTreeMap<String, String>,
    pub root: Option<Rect>,
}

impl StubScene {
    pub fn node(mut self, id: &str, bounds: Rect) -> Self {
        self.rects.insert(id.to_owned(), bounds);
        self
    }
}

impl MotionSceneView for StubScene {
    fn node_rect(&self, id: &str) -> Option<Rect> {
        self.rects.get(id).copied()
    }

    fn property(&self, id: &str, property: &AnimationProperty) -> Option<Value> {
        self.rects.get(id)?;
        let key = (id.to_owned(), property.name().to_owned());
        Some(
            self.values
                .get(&key)
                .cloned()
                .unwrap_or_else(|| match property.name() {
                    "opacity" | "scaleX" | "scaleY" => json!(1.0),
                    "fill" | "stroke" => json!("#ffffff"),
                    _ => json!(0.0),
                }),
        )
    }

    fn root_rect(&self) -> Option<Rect> {
        self.root
    }

    fn switches(&self) -> Vec<(String, bool)> {
        self.switches.clone()
    }

    fn subtree_contains(&self, ancestor: &str, id: &str) -> bool {
        let mut current = Some(id);
        while let Some(node) = current {
            if node == ancestor {
                return true;
            }
            current = self.parents.get(node).map(String::as_str);
        }
        false
    }
}

#[derive(Default)]
pub struct Warnings(pub RefCell<Vec<String>>);

impl MotionObserver for Warnings {
    fn animation_requested(&self, _: &str, _: &str, _: u64) {}
    fn motion_warning(&self, message: &str) {
        self.0.borrow_mut().push(message.to_owned());
    }
}

pub fn runtime(children: Value) -> Runtime {
    let source = json!({"version": "1.1", "children": children});
    let document = jian_ops_schema::load_str(&source.to_string())
        .expect("document")
        .value;
    Runtime::new_from_document(document).expect("runtime")
}

pub fn fade(id: &str, trigger: &str, delay_ms: u64) -> Value {
    json!({"type": "rectangle", "id": id, "width": 10, "height": 10,
        "animations": [{"trigger": trigger, "durationMs": 100, "delayMs": delay_ms,
            "easing": "linear", "keyframes": [
                {"offset": 0, "values": {"opacity": 0}},
                {"offset": 1, "values": {"opacity": 1}}]}]})
}
