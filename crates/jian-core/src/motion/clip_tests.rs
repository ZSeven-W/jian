use super::*;
use crate::geometry::rect;
use crate::motion::test_support::{runtime, StubScene};
use crate::motion::timeline::MotionTimeline;
use crate::motion::triggers::MotionTriggers;
use jian_ops_schema::motion::MotionPreference;
use serde_json::json;

fn screen() -> Value {
    json!([{"type": "frame", "id": "screen", "width": 390, "height": 800, "children": [
        {"type": "rectangle", "id": "title", "width": 300, "height": 40,
         "animations": [{"trigger": "mount", "durationMs": 400, "easing": "emphasizedDecelerate",
            "keyframes": [{"offset": 0, "values": {"opacity": 0, "translateY": 16}},
                          {"offset": 1, "values": {"opacity": 1, "translateY": 0}}]}]},
        {"type": "rectangle", "id": "card", "width": 300, "height": 80,
         "animations": [{"trigger": "mount", "durationMs": 300, "delayMs": 120, "easing": "standard",
            "keyframes": [{"offset": 0, "values": {"opacity": 0, "scaleX": 0.8}},
                          {"offset": 0.6, "values": {"opacity": 1, "scaleX": 1.05}},
                          {"offset": 1, "values": {"scaleX": 1}}]}]},
        {"type": "rectangle", "id": "cta", "width": 300, "height": 40,
         "animations": [{"trigger": "mount", "durationMs": 200, "delayMs": 50, "easing": "ease",
            "iterations": 2, "fillMode": "none",
            "keyframes": [{"offset": 0, "values": {"fill": "#ff0000", "rotation": 0}},
                          {"offset": 0.5, "values": {"fill": "#0000ff", "rotation": 6}},
                          {"offset": 1, "values": {"fill": "#ff0000", "rotation": 0}}]}]},
        {"type": "rectangle", "id": "later", "width": 10, "height": 10,
         "animations": [{"trigger": "inView", "durationMs": 100,
            "keyframes": {"from": {"opacity": 0}, "to": {"opacity": 1}}}]}
    ]}])
}

fn scene() -> StubScene {
    StubScene::default()
        .node("screen", rect(0.0, 0.0, 390.0, 800.0))
        .node("title", rect(0.0, 0.0, 300.0, 40.0))
        .node("card", rect(0.0, 60.0, 300.0, 80.0))
        .node("cta", rect(0.0, 160.0, 300.0, 40.0))
        .node("later", rect(0.0, 700.0, 10.0, 10.0))
}

fn root(children: &Value) -> PenNode {
    let source = json!({"version": "1.1", "children": children});
    jian_ops_schema::load_str(&source.to_string())
        .expect("document")
        .value
        .children
        .remove(0)
}

#[test]
fn sampling_matches_the_runtime_timeline_frame_for_frame() {
    let children = screen();
    let runtime = runtime(children.clone());
    let scene = scene();
    let clip = Clip::build(&root(&children), &ClipTrigger::Mount);
    let (triggers, timeline) = (MotionTriggers::default(), MotionTimeline::default());
    triggers.start_mount_animations(&runtime, &scene, &timeline, MotionPreference::Full, 0);
    for t in (0..=700).step_by(16) {
        timeline.tick(t);
        let expected = timeline.binding_values();
        let actual: BTreeMap<_, _> = clip
            .sample(t, &scene)
            .into_iter()
            .filter_map(|((node, property), value)| {
                property
                    .binding_target()
                    .map(|target| ((node, target), value))
            })
            .collect();
        assert_eq!(actual, expected, "at {t} ms");
    }
}

#[test]
fn a_clip_holds_only_its_trigger_and_reports_duration_and_keyframes() {
    let children = screen();
    let mount = Clip::build(&root(&children), &ClipTrigger::Mount);
    // card ends at 120 + 300; cta at 50 + 200 x 2.
    assert_eq!(mount.duration_ms(), Some(450));
    let in_view = Clip::build(&root(&children), &ClipTrigger::InView);
    assert_eq!(in_view.duration_ms(), Some(100));
    let card_times: Vec<f32> = mount
        .keyframe_times()
        .into_iter()
        .filter(|keyframe| keyframe.node == "card")
        .map(|keyframe| keyframe.time_ms)
        .collect();
    assert_eq!(card_times.len(), 3);
    assert_eq!(card_times[0], 120.0);
    assert!((card_times[2] - 420.0).abs() < 0.01);
    // Offset 0.6 sits earlier in time on an ease-out-heavy curve.
    assert!(card_times[1] > 120.0 && card_times[1] < 120.0 + 300.0 * 0.6);
}

#[test]
fn manual_clips_gather_a_shared_id_and_infinite_loops_have_no_end() {
    let manual = |id: &str, iterations: Value| {
        json!({"type": "rectangle", "id": id, "width": 10, "height": 10,
            "animations": [{"trigger": "manual", "id": "pulse", "durationMs": 100,
                "iterations": iterations,
                "keyframes": {"from": {"opacity": 0}, "to": {"opacity": 1}}}]})
    };
    let children = json!([{"type": "frame", "id": "list", "width": 50, "height": 50,
        "children": [manual("a", json!(1)), manual("b", json!("infinite"))]}]);
    let root = root(&children);
    let clip = Clip::build(&root, &ClipTrigger::Manual("pulse".into()));
    assert_eq!(clip.keyframe_times().len(), 4);
    assert_eq!(
        clip.duration_ms(),
        Some(100),
        "the finite member sets the end"
    );
    let only_loop = Clip::build(
        &root.children_ref().expect("children")[1],
        &ClipTrigger::Manual("pulse".into()),
    );
    assert_eq!(only_loop.duration_ms(), None);
    assert!(Clip::build(&root, &ClipTrigger::Manual("other".into())).is_empty());
}
