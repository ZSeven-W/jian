use super::*;
use crate::geometry::{rect, Rect};
use crate::motion::timeline::MotionObserver;
use serde_json::json;

/// A hand-built scene: node rects, painted property values, switches, and
/// a parent map for subtree checks.
#[derive(Default)]
pub(super) struct StubScene {
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
pub(super) struct Warnings(pub RefCell<Vec<String>>);

impl MotionObserver for Warnings {
    fn animation_requested(&self, _: &str, _: &str, _: u64) {}
    fn motion_warning(&self, message: &str) {
        self.0.borrow_mut().push(message.to_owned());
    }
}

pub(super) fn runtime(children: Value) -> Runtime {
    let source = json!({"version": "1.1", "children": children});
    let document = jian_ops_schema::load_str(&source.to_string())
        .expect("document")
        .value;
    Runtime::new_from_document(document).expect("runtime")
}

pub(super) fn fade(id: &str, trigger: &str, delay_ms: u64) -> Value {
    json!({"type": "rectangle", "id": id, "width": 10, "height": 10,
        "animations": [{"trigger": trigger, "durationMs": 100, "delayMs": delay_ms,
            "easing": "linear", "keyframes": [
                {"offset": 0, "values": {"opacity": 0}},
                {"offset": 1, "values": {"opacity": 1}}]}]})
}

fn opacity() -> AnimationProperty {
    AnimationProperty::Opacity
}

#[test]
fn the_lifecycle_starts_once_per_screen() {
    let triggers = MotionTriggers::default();
    assert!(!triggers.lifecycle_started());
    assert!(triggers.begin_lifecycle());
    assert!(!triggers.begin_lifecycle());
    triggers.reset_screen();
    assert!(!triggers.lifecycle_started());
    assert!(triggers.begin_lifecycle());
}

#[test]
fn the_in_view_window_survives_a_screen_reset() {
    let triggers = MotionTriggers::default();
    let window = InViewWindow::new(100.0, 0.0, 800.0, Vec::new());
    assert!(triggers.set_in_view_window(Some(window.clone())));
    assert!(!triggers.set_in_view_window(Some(window.clone())));
    triggers.reset_screen();
    assert_eq!(*triggers.in_view_window.borrow(), Some(window));
    assert!(triggers.set_in_view_window(None));
}

#[test]
fn mount_animations_start_with_their_delay_and_hold_keyframe_zero() {
    let runtime = runtime(json!([fade("a", "mount", 0), fade("b", "mount", 50)]));
    let scene = StubScene::default()
        .node("a", rect(0.0, 0.0, 10.0, 10.0))
        .node("b", rect(0.0, 20.0, 10.0, 10.0));
    let (triggers, timeline) = (MotionTriggers::default(), MotionTimeline::default());
    triggers.start_mount_animations(&runtime, &scene, &timeline, MotionPreference::Full, 0);
    assert_eq!(timeline.track_count(), 2);
    timeline.tick(25);
    assert_eq!(
        timeline.current_override("a", &opacity()),
        Some(json!(0.25))
    );
    // The delayed node holds its authored first keyframe until its delay elapses.
    assert_eq!(timeline.current_override("b", &opacity()), Some(json!(0)));
    timeline.tick(100);
    assert_eq!(timeline.current_override("b", &opacity()), Some(json!(0.5)));
}

#[test]
fn reduced_motion_lands_forwards_animations_on_their_final_value() {
    let runtime = runtime(json!([fade("a", "mount", 0)]));
    let mut scene = StubScene::default().node("a", rect(0.0, 0.0, 10.0, 10.0));
    scene
        .values
        .insert(("a".into(), "opacity".into()), json!(0.4));
    let (triggers, timeline) = (MotionTriggers::default(), MotionTimeline::default());
    triggers.start_mount_animations(&runtime, &scene, &timeline, MotionPreference::Reduced, 0);
    assert_eq!(timeline.track_count(), 0);
    assert_eq!(timeline.current_override("a", &opacity()), Some(json!(1)));
}

#[test]
fn missing_nodes_are_skipped_and_the_budget_caps_tracks() {
    let nodes: Vec<Value> = (0..MOTION_TRACK_BUDGET + 2)
        .map(|index| fade(&format!("n{index}"), "mount", 0))
        .collect();
    let runtime = runtime(Value::Array(nodes));
    let mut scene = StubScene::default();
    for index in 0..=MOTION_TRACK_BUDGET {
        scene = scene.node(&format!("n{index}"), rect(0.0, 0.0, 10.0, 10.0));
    }
    let (triggers, timeline) = (MotionTriggers::default(), MotionTimeline::default());
    let warnings = Rc::new(Warnings::default());
    timeline.set_observer(warnings.clone());
    triggers.start_mount_animations(&runtime, &scene, &timeline, MotionPreference::Full, 0);
    assert_eq!(timeline.track_count(), MOTION_TRACK_BUDGET);
    assert_eq!(warnings.0.borrow().len(), 1, "{:?}", warnings.0.borrow());
}

#[test]
fn lifecycle_animations_paint_keyframe_zero_before_they_start() {
    let runtime = runtime(json!([
        fade("mounted", "mount", 0),
        fade("revealed", "inView", 0),
        {"type": "rectangle", "id": "played", "width": 10, "height": 10,
         "animations": [{"trigger": "manual", "id": "pop", "durationMs": 100,
            "keyframes": {"from": {"opacity": 0}, "to": {"opacity": 1}}}]}
    ]));
    let scene = StubScene::default()
        .node("mounted", rect(0.0, 0.0, 10.0, 10.0))
        .node("revealed", rect(0.0, 20.0, 10.0, 10.0))
        .node("played", rect(0.0, 40.0, 10.0, 10.0));
    let (triggers, timeline) = (MotionTriggers::default(), MotionTimeline::default());
    triggers.set_initial_lifecycle_values(&runtime, &scene, &timeline);
    assert_eq!(
        timeline.current_override("mounted", &opacity()),
        Some(json!(0))
    );
    assert_eq!(
        timeline.current_override("revealed", &opacity()),
        Some(json!(0))
    );
    assert_eq!(timeline.current_override("played", &opacity()), None);
    assert_eq!(timeline.track_count(), 0);
}
