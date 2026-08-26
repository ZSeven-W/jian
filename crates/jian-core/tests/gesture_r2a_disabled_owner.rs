//! R2A round 2 (Task B) regressions: disabled slider raw drag and
//! nearest-owner Pan.
//!
//! 1. `Runtime::handle_slider_drag` honors the slider's inert gate —
//!    static `gestures.disabledEvents` (onTap) and dynamic
//!    `gestures.disabled` — before ANY raw Down side effect (focus,
//!    `dragging`, value mutation, `bind:value` sync), and disarms an
//!    armed slider whose disable-flip happens before the next Move.
//! 2. The Pan owner is the NEAREST hit-path node owning any enabled
//!    nonempty `onPanStart`/`onPanUpdate`/`onPanEnd` (respecting
//!    `disabledEvents` + dynamic `gestures.disabled`); its authored
//!    threshold governs and its node is the semantic target, while
//!    delivery still bubbles each phase to the chain's handler.

use jian_core::document::NodeKey;
use jian_core::geometry::{point, Point};
use jian_core::gesture::{
    MouseButtons, PointerEvent, PointerId, PointerKind, PointerPhase, SemanticEvent,
};
use jian_core::Runtime;

fn runtime_with<S: AsRef<str>>(op: S) -> Runtime {
    let mut rt = Runtime::new();
    rt.load_str(op.as_ref()).unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    rt
}

fn node_key(rt: &Runtime, id: &str) -> NodeKey {
    rt.document.as_ref().unwrap().tree.get(id).expect(id)
}

fn names(evs: &[SemanticEvent]) -> Vec<&'static str> {
    evs.iter().map(|e| e.handler_key()).collect()
}

fn mouse(id: u32, phase: PointerPhase, position: Point, t_ms: u64) -> PointerEvent {
    PointerEvent {
        id: PointerId(id),
        kind: PointerKind::Mouse,
        phase,
        position,
        pressure: 0.0,
        buttons: MouseButtons::LEFT,
        modifiers: Default::default(),
        tilt: None,
        t_ms,
    }
}

fn slider_state(rt: &Runtime) -> (f64, bool) {
    // An un-seeded slider (never focused/armed/mutated) reports the
    // authored default and `dragging == false`.
    match rt.widget_states.get("sl") {
        Some(jian_core::widget_state::WidgetState::Slider {
            value, dragging, ..
        }) => (*value, *dragging),
        None => (0.0, false),
        other => panic!("expected slider state, got {other:?}"),
    }
}

fn vol(rt: &Runtime) -> Option<f64> {
    rt.state.app_get("vol").and_then(|v| v.as_f64())
}

// ---------------------------------------------------------------------
// 1. Disabled slider raw drag
// ---------------------------------------------------------------------

/// Slider gated STATICALLY via `gestures.disabledEvents` (onTap) and
/// bound to `$state.vol`.
const SLIDER_STATIC_DISABLED_OP: &str = r##"{"version":"0.8.0",
  "state":{"vol":{"type":"float","default":0}},
  "children":[{"type":"slider","id":"sl","x":10,"y":30,"width":200,"height":20,
    "min":0,"max":100,"step":1,"bindings":{"bind:value":"$state.vol"},
    "gestures":{"disabledEvents":["onTap"]}}]}"##;

/// Slider gated DYNAMICALLY via `gestures.disabled` (`$app.off`).
const SLIDER_DYNAMIC_DISABLED_OP: &str = r##"{"version":"0.8.0",
  "state":{"off":{"type":"bool","default":false},"vol":{"type":"float","default":0}},
  "children":[{"type":"slider","id":"sl","x":10,"y":30,"width":200,"height":20,
    "min":0,"max":100,"step":1,"bindings":{"bind:value":"$state.vol"},
    "gestures":{"disabled":"$app.off"}}]}"##;

/// A statically disabled Slider's Down is fully inert: no focus, no
/// `dragging`, no value mutation, no `bind:value` sync; later Move/Up
/// change nothing either.
#[test]
fn statically_disabled_slider_down_is_fully_inert() {
    let mut rt = runtime_with(SLIDER_STATIC_DISABLED_OP);
    let at = point(110.0, 40.0); // track midpoint → 50.0 if scrubbed

    assert_eq!(rt.focused_widget_id(), None);
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, at, 0));
    assert_eq!(
        rt.focused_widget_id(),
        None,
        "disabled Down must not focus the slider"
    );
    assert_eq!(slider_state(&rt), (0.0, false), "never armed/mutated");
    assert_eq!(vol(&rt), Some(0.0), "no bind sync on a disabled Down");

    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(210.0, 40.0), 10));
    assert_eq!(
        slider_state(&rt),
        (0.0, false),
        "disabled Move must not arm"
    );
    assert_eq!(vol(&rt), Some(0.0));
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Up, point(210.0, 40.0), 20));
    assert_eq!(slider_state(&rt), (0.0, false));
    assert_eq!(vol(&rt), Some(0.0), "no value/binding effect at all");
}

/// Dynamic gate: a disabled-from-Down slider is inert; an enabled Down
/// focuses/arms/scrubs/syncs, and a disable-flip before the next Move
/// disarms immediately with no further mutation or sync.
#[test]
fn dynamically_disabled_slider_down_is_inert_and_flip_disarms_mid_drag() {
    let mut rt = runtime_with(SLIDER_DYNAMIC_DISABLED_OP);
    let at = point(110.0, 40.0); // → 50.0 when the drag is live
    let far = point(210.0, 40.0); // → 100.0 if it scrubbed

    // Phase 1: disabled from the very Down — fully inert.
    rt.state.app_set("off", serde_json::json!(true));
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, at, 0));
    assert_eq!(
        rt.focused_widget_id(),
        None,
        "dynamic disabled Down: no focus"
    );
    assert_eq!(
        slider_state(&rt),
        (0.0, false),
        "dynamic disabled Down: no arm"
    );
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Move, far, 10));
    assert_eq!(slider_state(&rt), (0.0, false));
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Up, far, 20));
    assert_eq!(vol(&rt), Some(0.0), "no value or binding effect");

    // Phase 2: enabled Down arms and scrubs; the flip happens AFTER the
    // Down but BEFORE the Move — the Move must disarm and stop.
    rt.state.app_set("off", serde_json::json!(false));
    let _ = rt.dispatch_pointer(mouse(2, PointerPhase::Down, at, 100));
    assert_eq!(
        rt.focused_widget_id(),
        Some("sl".to_owned()),
        "enabled Down focuses"
    );
    assert_eq!(
        slider_state(&rt),
        (50.0, true),
        "enabled Down arms + scrubs"
    );
    assert_eq!(vol(&rt), Some(50.0), "enabled Down syncs the binding");

    rt.state.app_set("off", serde_json::json!(true));
    let _ = rt.dispatch_pointer(mouse(2, PointerPhase::Move, far, 110));
    assert_eq!(
        slider_state(&rt),
        (50.0, false),
        "disabled Move must disarm immediately and not scrub"
    );
    assert_eq!(
        vol(&rt),
        Some(50.0),
        "disabled Move must not re-sync the binding"
    );
    // Focus stays (only Down gates focus; disarming is not a blur).
    assert_eq!(rt.focused_widget_id(), Some("sl".to_owned()));

    // A follow-up Move (still disabled state) cannot re-arm or scrub.
    let _ = rt.dispatch_pointer(mouse(2, PointerPhase::Move, point(20.0, 40.0), 120));
    assert_eq!(slider_state(&rt), (50.0, false));
    assert_eq!(vol(&rt), Some(50.0));
    let _ = rt.dispatch_pointer(mouse(2, PointerPhase::Up, point(20.0, 40.0), 130));
    assert_eq!(vol(&rt), Some(50.0));

    // Phase 3: re-enabled, but arming is per-Down — a bare Move never
    // scrubs a disarmed slider.
    rt.state.app_set("off", serde_json::json!(false));
    let _ = rt.dispatch_pointer(mouse(3, PointerPhase::Move, far, 200));
    assert_eq!(slider_state(&rt), (50.0, false));
    assert_eq!(vol(&rt), Some(50.0));
}

// ---------------------------------------------------------------------
// 2. Nearest Pan owner across any Pan hook
// ---------------------------------------------------------------------

/// Child owns only `onPanUpdate` (authored threshold 30); the parent owns
/// `onPanStart` (authored threshold 8). The child is the nearest owner,
/// so its threshold governs the recognizer and the Pan semantics target
/// the child — the PanStart still BUBBLES to the parent's handler while
/// every PanUpdate executes the child's handler.
const SPLIT_PAN_OWNERS_OP: &str = r##"{"version":"0.8.0",
  "state":{"starts":{"type":"int","default":0},"updates":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","x":0,"y":0,"width":400,"height":400,
    "gestures":{"dragThreshold":8},
    "events":{"onPanStart":[{"set":{"$app.starts":"$app.starts + 1"}}]},
    "children":[{"type":"rectangle","id":"child","x":10,"y":10,"width":100,"height":100,
      "gestures":{"dragThreshold":30},
      "events":{"onPanUpdate":[{"set":{"$app.updates":"$app.updates + 1"}}]}}]}]}"##;

/// Child Pan owners are skipped when STATICALLY (`disabledEvents`) or
/// DYNAMICALLY (`gestures.disabled`) disabled: the owner scan walks past
/// them and the parent's authored threshold + node take over.
const SPLIT_PAN_DISABLED_OWNERS_OP: &str = r##"{"version":"0.8.0",
  "state":{"off":{"type":"bool","default":true},
           "starts":{"type":"int","default":0},"updates":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","x":0,"y":0,"width":500,"height":300,
    "gestures":{"dragThreshold":30},
    "events":{"onPanStart":[{"set":{"$app.starts":"$app.starts + 1"}}]},
    "children":[
      {"type":"rectangle","id":"left","x":10,"y":10,"width":100,"height":100,
       "gestures":{"dragThreshold":8,"disabledEvents":["onPanUpdate"]},
       "events":{"onPanUpdate":[{"set":{"$app.updates":"$app.updates + 1"}}]}},
      {"type":"rectangle","id":"right","x":200,"y":10,"width":100,"height":100,
       "gestures":{"dragThreshold":8,"disabled":"$app.off"},
       "events":{"onPanUpdate":[{"set":{"$app.updates":"$app.updates + 1"}}]}}]}]}"##;

/// The nearest Pan owner is found across ANY pan hook (not by handler
/// name): child `onPanUpdate` threshold 30 wins the owner slot over the
/// parent's `onPanStart` threshold 8; PanStart bubbles to the parent,
/// PanUpdate executes the child, and the counts prove both.
#[test]
fn nearest_pan_owner_across_any_hook_governs_threshold_and_semantics() {
    let mut rt = runtime_with(SPLIT_PAN_OWNERS_OP);
    let child_key = node_key(&rt, "child");
    let c = point(60.0, 60.0); // child center ((10+50), (10+50))

    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    // 15px: over the parent's 8px but under the nearest owner's 30px —
    // the child's authored threshold governs, nothing claims yet.
    let small = rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(75.0, 60.0), 100));
    assert!(small.is_empty(), "got {small:?}");
    assert_eq!(rt.state.app_get("starts").unwrap().as_i64(), Some(0));

    // 40px: claims with the child as the semantic target; onPanStart
    // bubbles child → parent, so the parent's handler runs.
    let start = rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(100.0, 60.0), 200));
    assert_eq!(names(&start), ["onPanStart"], "got {:?}", names(&start));
    assert_eq!(
        start[0].node(),
        child_key,
        "PanStart semantic targets the nearest owner (child), not the parent"
    );
    assert_eq!(rt.state.app_get("starts").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("updates").unwrap().as_i64(), Some(0));

    // Subsequent Move: the PanUpdate executes the CHILD's handler.
    let update = rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(120.0, 60.0), 300));
    assert_eq!(names(&update), ["onPanUpdate"], "got {:?}", names(&update));
    assert_eq!(
        update[0].node(),
        child_key,
        "PanUpdate also targets the child"
    );
    assert_eq!(rt.state.app_get("updates").unwrap().as_i64(), Some(1));
    let end = rt.dispatch_pointer(mouse(1, PointerPhase::Up, point(120.0, 60.0), 400));
    assert_eq!(names(&end), ["onPanEnd"], "got {:?}", names(&end));
    assert_eq!(rt.state.app_get("starts").unwrap().as_i64(), Some(1));
}

/// DisabledEvents-listed and dynamically-disabled child pan owners are
/// skipped: the parent becomes the owner (its threshold governs, its
/// node is the semantic target), and re-enabling hands ownership back to
/// the nearest child.
#[test]
fn disabled_child_pan_owners_are_skipped_to_the_parent() {
    let mut rt = runtime_with(SPLIT_PAN_DISABLED_OWNERS_OP);
    let left = point(60.0, 60.0); // left child center
    let right = point(250.0, 60.0); // right child center

    // `left`: onPanUpdate is disabledEvents-listed → skipped. `right`:
    // gestures.disabled = $app.off (true) → skipped. Owner = root
    // (threshold 30): 15px does not claim, 40px claims AT the root.
    for at in [left, right] {
        let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, at, 0));
        let small =
            rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(at.x + 15.0, at.y), 100));
        assert!(
            small.is_empty(),
            "disabled child threshold must not govern, got {small:?}"
        );
        let start =
            rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(at.x + 40.0, at.y), 200));
        assert_eq!(names(&start), ["onPanStart"], "got {:?}", names(&start));
        assert_eq!(
            start[0].node(),
            node_key(&rt, "root"),
            "disabled children skipped → the parent is the owner"
        );
        // PanUpdate targets the parent: the child (below it) never runs.
        let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(at.x + 60.0, at.y), 300));
        let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Up, point(at.x + 60.0, at.y), 400));
    }
    assert_eq!(rt.state.app_get("starts").unwrap().as_i64(), Some(2));
    assert_eq!(rt.state.app_get("updates").unwrap().as_i64(), Some(0));

    // Re-enable `right`: ownership returns to the nearest child; its 8px
    // threshold claims, its handler executes the updates, and the
    // PanStart still bubbles to the parent.
    rt.state.app_set("off", serde_json::json!(false));
    let _ = rt.dispatch_pointer(mouse(3, PointerPhase::Down, right, 500));
    let start = rt.dispatch_pointer(mouse(3, PointerPhase::Move, point(260.0, 60.0), 600));
    assert_eq!(names(&start), ["onPanStart"], "got {:?}", names(&start));
    assert_eq!(
        start[0].node(),
        node_key(&rt, "right"),
        "re-enabled child owns the pan again"
    );
    let update = rt.dispatch_pointer(mouse(3, PointerPhase::Move, point(280.0, 60.0), 700));
    assert_eq!(names(&update), ["onPanUpdate"], "got {:?}", names(&update));
    assert_eq!(update[0].node(), node_key(&rt, "right"));
    let _ = rt.dispatch_pointer(mouse(3, PointerPhase::Up, point(280.0, 60.0), 800));

    assert_eq!(rt.state.app_get("starts").unwrap().as_i64(), Some(3));
    assert_eq!(rt.state.app_get("updates").unwrap().as_i64(), Some(1));
}
