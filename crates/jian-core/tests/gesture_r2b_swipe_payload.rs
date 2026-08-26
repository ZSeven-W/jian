//! R2B Swipe payload tests — the ONE `$event` payload path and the
//! ActionContext end-to-end wiring.
//!
//! Assertions are on exact payload JSON (direction/distance/velocity
//! riding the gesture facts, node-local `local` from the resolved
//! handler owner, phase/position/timestamp/button from the triggering
//! Move + initiating Down) and on executed action results, so both the
//! envelope → payload step and the ActionList execution step are proven.

use jian_core::geometry::{point, Point};
use jian_core::gesture::{
    MouseButtons, PointerEvent, PointerId, PointerKind, PointerPhase, SemanticEvent,
    SemanticEventEnvelope, SwipeDirection,
};
use jian_core::Runtime;

fn runtime_with<S: AsRef<str>>(op: S) -> Runtime {
    let mut rt = Runtime::new();
    rt.load_str(op.as_ref()).unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    rt
}

/// Compute the payload exactly like the runtime does: the handler node's
/// layout-rect origin feeds `local`.
fn payload_json(rt: &mut Runtime, env: &SemanticEventEnvelope) -> String {
    let handler_node = env.event.node();
    let rect = rt.layout.node_rect(handler_node).unwrap();
    let origin = point(rect.min_x(), rect.min_y());
    serde_json::to_string(&env.payload(Some(origin)).expect("payload")).expect("serialize payload")
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

fn touch(id: u32, phase: PointerPhase, position: Point, t_ms: u64) -> PointerEvent {
    PointerEvent {
        id: PointerId(id),
        kind: PointerKind::Touch,
        phase,
        position,
        pressure: 1.0,
        buttons: Default::default(),
        modifiers: Default::default(),
        tilt: None,
        t_ms,
    }
}

/// Parent frame owns onSwipe; the handler-less child inside it is the
/// hit target. Offsets make `local` provably owner-relative.
const BUBBLED_SWIPE_OP: &str = r##"{"version":"0.8.0",
  "state":{"swipes":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","x":100,"y":100,"width":400,"height":400,
    "events":{"onSwipe":[{"set":{"$app.swipes":"$app.swipes + 1"}}]},
    "children":[{"type":"rectangle","id":"child","x":10,"y":10,"width":100,"height":100}]}]}"##;

#[test]
fn swipe_payload_exact_json() {
    let mut rt = runtime_with(BUBBLED_SWIPE_OP);
    let owner_key = rt.document.as_ref().unwrap().tree.get("root").unwrap();
    let at = point(160.0, 160.0); // child center, absolute

    let down = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, at, 0));
    assert!(down.is_empty(), "no press handler on the chain");
    let swipe = rt.dispatch_pointer_events(mouse(1, PointerPhase::Move, point(220.0, 160.0), 100));
    assert_eq!(swipe.len(), 1, "one Swipe envelope");
    assert_eq!(
        swipe[0].event.node(),
        owner_key,
        "semantic targets the nearest enabled onSwipe owner (the parent)"
    );
    assert!(
        matches!(
            &swipe[0].event,
            SemanticEvent::Swipe {
                direction: SwipeDirection::Right,
                distance: 60.0,
                ..
            }
        ),
        "got {:?}",
        swipe[0].event
    );
    assert_eq!(
        payload_json(&mut rt, &swipe[0]),
        r#"{"pointerId":1,"pointerType":"mouse","phase":"move","position":{"x":220.0,"y":160.0},"local":{"x":120.0,"y":60.0},"button":"left","buttons":["left"],"modifiers":[],"timestamp":100,"direction":"right","distance":60.0,"velocity":{"x":600.0,"y":0.0}}"#
    );
}

#[test]
fn four_direction_payload_strings() {
    // [Move delta, expected wire direction]
    let cases: [(f32, f32, &str); 4] = [
        (60.0, 0.0, "right"),
        (-60.0, 0.0, "left"),
        (0.0, -60.0, "up"),
        (0.0, 60.0, "down"),
    ];
    for (i, (dx, dy, expected)) in cases.into_iter().enumerate() {
        let mut rt = runtime_with(BUBBLED_SWIPE_OP);
        let at = point(160.0, 160.0);
        let _ = rt.dispatch_pointer_events(touch(i as u32, PointerPhase::Down, at, 0));
        let swipe = rt.dispatch_pointer_events(touch(
            i as u32,
            PointerPhase::Move,
            point(at.x + dx, at.y + dy),
            100,
        ));
        assert_eq!(swipe.len(), 1, "case {expected}");
        let json = payload_json(&mut rt, &swipe[0]);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            value["direction"],
            serde_json::json!(expected),
            "case {expected}"
        );
        assert_eq!(
            value["distance"],
            serde_json::json!(60.0),
            "case {expected}"
        );
        // Touch swipes carry no provable button (Down bitmask empty).
        assert!(value.get("button").is_none(), "case {expected}: {json}");
        // 60px over 100ms → segment velocity (600.0, 0.0) etc.
        assert_eq!(
            value["velocity"],
            serde_json::json!({ "x": dx * 10.0, "y": dy * 10.0 }),
            "case {expected}"
        );
    }
}

/// onSwipe ActionList reads `$event.direction/distance/velocity/local/
/// button/phase/pointerId` and changes state — the exact ActionContext
/// fields through the real runtime delivery path.
const SWIPE_ACTIONS_OP: &str = r##"{"version":"0.8.0",
  "state":{
    "swipes":{"type":"int","default":0},
    "dir":{"type":"string","default":""},
    "dist":{"type":"float","default":0.0},
    "velX":{"type":"float","default":0.0},
    "velY":{"type":"float","default":0.0},
    "localX":{"type":"float","default":0.0},
    "localY":{"type":"float","default":0.0},
    "button":{"type":"string","default":""},
    "phase":{"type":"string","default":""},
    "pointerType":{"type":"string","default":""},
    "pointerId":{"type":"float","default":0.0},
    "timestamp":{"type":"float","default":0.0}
  },
  "children":[{"type":"frame","id":"root","x":100,"y":100,"width":400,"height":400,
    "events":{"onSwipe":[
      {"set":{"$app.swipes":"$app.swipes + 1"}},
      {"set":{"$app.dir":"$event.direction"}},
      {"set":{"$app.dist":"$event.distance"}},
      {"set":{"$app.velX":"$event.velocity.x"}},
      {"set":{"$app.velY":"$event.velocity.y"}},
      {"set":{"$app.localX":"$event.local.x"}},
      {"set":{"$app.localY":"$event.local.y"}},
      {"set":{"$app.button":"$event.button"}},
      {"set":{"$app.phase":"$event.phase"}},
      {"set":{"$app.pointerType":"$event.pointerType"}},
      {"set":{"$app.pointerId":"$event.pointerId"}},
      {"set":{"$app.timestamp":"$event.timestamp"}}
    ]},
    "children":[{"type":"rectangle","id":"child","x":10,"y":10,"width":100,"height":100}]}]}"##;

#[test]
fn swipe_actions_read_payload_fields_and_update_state() {
    let mut rt = runtime_with(SWIPE_ACTIONS_OP);
    let at = point(160.0, 160.0); // child center; owner origin (100,100)

    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, at, 0));
    let swipe = rt.dispatch_pointer_events(mouse(1, PointerPhase::Move, point(220.0, 160.0), 100));
    assert_eq!(swipe.len(), 1, "Swipe envelope");
    assert_eq!(rt.state.app_get("swipes").unwrap().as_i64(), Some(1));

    assert_eq!(
        rt.state
            .app_get("dir")
            .and_then(|v| v.as_str().map(str::to_owned)),
        Some("right".to_owned()),
        "$event.direction"
    );
    assert_eq!(
        rt.state.app_get("dist").and_then(|v| v.as_f64()),
        Some(60.0),
        "$event.distance = total travel from the Down"
    );
    assert_eq!(
        rt.state.app_get("velX").and_then(|v| v.as_f64()),
        Some(600.0),
        "$event.velocity.x = segment velocity"
    );
    assert_eq!(rt.state.app_get("velY").and_then(|v| v.as_f64()), Some(0.0));
    assert_eq!(
        rt.state.app_get("localX").and_then(|v| v.as_f64()),
        Some(120.0),
        "$event.local.x is owner-relative: (220,160) − (100,100)"
    );
    assert_eq!(
        rt.state.app_get("localY").and_then(|v| v.as_f64()),
        Some(60.0)
    );
    assert_eq!(
        rt.state
            .app_get("button")
            .and_then(|v| v.as_str().map(str::to_owned)),
        Some("left".to_owned()),
        "initiating Down's provable LEFT retained on the triggering Move"
    );
    assert_eq!(
        rt.state
            .app_get("phase")
            .and_then(|v| v.as_str().map(str::to_owned)),
        Some("move".to_owned()),
        "phase stays from the triggering Move"
    );
    assert_eq!(
        rt.state
            .app_get("pointerType")
            .and_then(|v| v.as_str().map(str::to_owned)),
        Some("mouse".to_owned())
    );
    assert_eq!(
        rt.state.app_get("pointerId").and_then(|v| v.as_f64()),
        Some(1.0)
    );
    assert_eq!(
        rt.state.app_get("timestamp").and_then(|v| v.as_f64()),
        Some(100.0)
    );
}

/// Touch swipes deliver no `button` key at all (a touch Down has an
/// empty buttons bitmask → nothing provable) and the ActionList still
/// runs with every authored field read.
#[test]
fn touch_swipe_payload_omits_button() {
    let mut rt = runtime_with(SWIPE_ACTIONS_OP);
    let at = point(160.0, 160.0);

    let _ = rt.dispatch_pointer_events(touch(2, PointerPhase::Down, at, 0));
    let swipe =
        rt.dispatch_pointer_events(touch(2, PointerPhase::Move, point(at.x + 60.0, at.y), 100));
    assert_eq!(swipe.len(), 1);
    let json = payload_json(&mut rt, &swipe[0]);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(value.get("button").is_none(), "got {json}");
    assert_eq!(value["pointerType"], serde_json::json!("touch"));
    // The ActionList kept running: the absent `$event.button` writes
    // null (not a crash); the other fields all executed.
    assert_eq!(rt.state.app_get("swipes").unwrap().as_i64(), Some(1));
    assert!(
        matches!(rt.state.app_get("button"), Some(v) if v.is_null()),
        "no button key → the set writes null, not a value"
    );
    assert_eq!(
        rt.state
            .app_get("dir")
            .and_then(|v| v.as_str().map(str::to_owned)),
        Some("right".to_owned())
    );
}

/// The claimed Swipe's payload is a one-shot: a following Up produces no
/// envelope and no second ActionList execution.
#[test]
fn swipe_action_runs_exactly_once_for_the_sequence() {
    let mut rt = runtime_with(SWIPE_ACTIONS_OP);
    let at = point(160.0, 160.0);

    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, at, 0));
    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Move, point(220.0, 160.0), 100));
    let up = rt.dispatch_pointer_events(mouse(1, PointerPhase::Up, point(220.0, 160.0), 200));
    assert!(up.is_empty(), "one-shot Swipe, got {up:?}");
    assert_eq!(rt.state.app_get("swipes").unwrap().as_i64(), Some(1));
}
