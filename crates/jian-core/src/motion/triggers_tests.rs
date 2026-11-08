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

fn transition_runtime(properties: Value) -> Runtime {
    runtime(
        json!([{"type": "rectangle", "id": "button", "width": 10, "height": 10,
        "transition": {"durationMs": 200, "easing": "linear", "properties": properties}}]),
    )
}

#[test]
fn a_changed_painted_value_starts_a_transition_from_the_old_one() {
    let runtime = transition_runtime(json!(["opacity"]));
    let mut scene = StubScene::default().node("button", rect(0.0, 0.0, 10.0, 10.0));
    let (triggers, timeline) = (MotionTriggers::default(), MotionTimeline::default());
    triggers.observe_transitions(&runtime, &scene, &timeline, MotionPreference::Full, 0);
    assert_eq!(
        timeline.track_count(),
        0,
        "the first observation only records"
    );
    scene
        .values
        .insert(("button".into(), "opacity".into()), json!(0.0));
    triggers.observe_transitions(&runtime, &scene, &timeline, MotionPreference::Full, 0);
    timeline.tick(100);
    assert_eq!(
        timeline.current_override("button", &opacity()),
        Some(json!(0.5))
    );
    // Retarget mid-tween: the new tween starts from what is on screen.
    scene
        .values
        .insert(("button".into(), "opacity".into()), json!(1.0));
    triggers.observe_transitions(&runtime, &scene, &timeline, MotionPreference::Full, 100);
    timeline.tick(200);
    assert_eq!(
        timeline.current_override("button", &opacity()),
        Some(json!(0.75))
    );
}

#[test]
fn reduced_motion_applies_transitions_instantly() {
    let runtime = transition_runtime(json!(["opacity"]));
    let mut scene = StubScene::default().node("button", rect(0.0, 0.0, 10.0, 10.0));
    let (triggers, timeline) = (MotionTriggers::default(), MotionTimeline::default());
    triggers.observe_transitions(&runtime, &scene, &timeline, MotionPreference::Reduced, 0);
    scene
        .values
        .insert(("button".into(), "opacity".into()), json!(0.2));
    triggers.observe_transitions(&runtime, &scene, &timeline, MotionPreference::Reduced, 0);
    assert_eq!(timeline.track_count(), 0);
    // The instant value equals the painted base, so no override remains.
    assert_eq!(timeline.current_override("button", &opacity()), None);
}

#[test]
fn layout_properties_in_a_transition_are_rejected_with_a_warning() {
    let runtime = transition_runtime(json!(["width", "opacity"]));
    let scene = StubScene::default().node("button", rect(0.0, 0.0, 10.0, 10.0));
    let (triggers, timeline) = (MotionTriggers::default(), MotionTimeline::default());
    let warnings = Rc::new(Warnings::default());
    timeline.set_observer(warnings.clone());
    triggers.observe_transitions(&runtime, &scene, &timeline, MotionPreference::Full, 0);
    assert_eq!(
        *warnings.0.borrow(),
        ["motion property width on node button rejected"]
    );
}

#[test]
fn a_toggled_switch_tweens_with_the_material_default() {
    let runtime = runtime(json!([{"type": "switch", "id": "toggle", "width": 60, "height": 32}]));
    let mut scene = StubScene::default().node("toggle", rect(0.0, 0.0, 60.0, 32.0));
    scene.switches = vec![("toggle".into(), false)];
    let (triggers, timeline) = (MotionTriggers::default(), MotionTimeline::default());
    let progress = AnimationProperty::ToggleProgress;
    triggers.observe_toggle_progress(&runtime, &scene, &timeline, MotionPreference::Full, 0);
    assert_eq!(
        timeline.track_count(),
        0,
        "the first observation only records"
    );
    scene.switches = vec![("toggle".into(), true)];
    triggers.observe_toggle_progress(&runtime, &scene, &timeline, MotionPreference::Full, 0);
    assert_eq!(timeline.track_count(), 1);
    timeline.tick(crate::motion::declarations::SWITCH_TOGGLE_DURATION_MS);
    assert_eq!(timeline.track_count(), 0);
    // Reduced motion snaps straight to the new state.
    scene.switches = vec![("toggle".into(), false)];
    triggers.observe_toggle_progress(&runtime, &scene, &timeline, MotionPreference::Reduced, 200);
    assert_eq!(timeline.track_count(), 0);
    assert_eq!(timeline.current_override("toggle", &progress), None);
}

fn in_view_runtime(once: bool) -> Runtime {
    runtime(
        json!([{"type": "frame", "id": "screen", "width": 390, "height": 1600,
        "children": [{"type": "rectangle", "id": "row", "width": 390, "height": 80,
            "animations": [{"trigger": "inView", "durationMs": 100, "once": once,
                "fillMode": "none",
                "keyframes": {"from": {"opacity": 0}, "to": {"opacity": 1}}}]}]}]),
    )
}

fn page_scene() -> StubScene {
    let mut scene = StubScene::default()
        .node("screen", rect(0.0, 0.0, 390.0, 1600.0))
        .node("row", rect(0.0, 1200.0, 390.0, 80.0));
    scene.root = Some(rect(0.0, 0.0, 390.0, 1600.0));
    scene
}

#[test]
fn in_view_waits_for_the_lifecycle_and_the_window() {
    let runtime = in_view_runtime(true);
    let scene = page_scene();
    let (triggers, timeline) = (MotionTriggers::default(), MotionTimeline::default());
    triggers.set_in_view_window(Some(InViewWindow::new(0.0, 0.0, 844.0, Vec::new())));
    triggers.observe_in_view(&runtime, &scene, &timeline, MotionPreference::Full, 0);
    assert_eq!(
        timeline.track_count(),
        0,
        "nothing fires before the lifecycle"
    );
    triggers.begin_lifecycle();
    triggers.observe_in_view(&runtime, &scene, &timeline, MotionPreference::Full, 0);
    assert_eq!(
        timeline.track_count(),
        0,
        "the row is below the first screen"
    );
    triggers.set_in_view_window(Some(InViewWindow::new(900.0, 0.0, 844.0, Vec::new())));
    triggers.observe_in_view(&runtime, &scene, &timeline, MotionPreference::Full, 10);
    assert_eq!(timeline.track_count(), 1);
}

#[test]
fn without_a_window_the_whole_root_counts_as_visible() {
    let runtime = in_view_runtime(true);
    let scene = page_scene();
    let (triggers, timeline) = (MotionTriggers::default(), MotionTimeline::default());
    triggers.begin_lifecycle();
    triggers.observe_in_view(&runtime, &scene, &timeline, MotionPreference::Full, 0);
    assert_eq!(timeline.track_count(), 1);
}

#[test]
fn once_fires_a_single_time_and_repeat_fires_after_leaving() {
    for (once, expected_fires) in [(true, 1), (false, 2)] {
        let runtime = in_view_runtime(once);
        let scene = page_scene();
        let (triggers, timeline) = (MotionTriggers::default(), MotionTimeline::default());
        triggers.begin_lifecycle();
        let mut fires = 0;
        for (scroll, now) in [(900.0, 0), (0.0, 500), (900.0, 1000)] {
            triggers.set_in_view_window(Some(InViewWindow::new(scroll, 0.0, 844.0, Vec::new())));
            let before = timeline.track_count();
            triggers.observe_in_view(&runtime, &scene, &timeline, MotionPreference::Full, now);
            fires += usize::from(timeline.track_count() > before);
            timeline.tick(now + 200);
        }
        assert_eq!(fires, expected_fires, "once = {once}");
    }
}

#[test]
fn pinned_nodes_fire_wherever_they_sit() {
    let runtime = in_view_runtime(true);
    let scene = page_scene();
    let (triggers, timeline) = (MotionTriggers::default(), MotionTimeline::default());
    triggers.begin_lifecycle();
    triggers.set_in_view_window(Some(InViewWindow::new(0.0, 0.0, 844.0, vec!["row".into()])));
    triggers.observe_in_view(&runtime, &scene, &timeline, MotionPreference::Full, 0);
    assert_eq!(timeline.track_count(), 1);
}
