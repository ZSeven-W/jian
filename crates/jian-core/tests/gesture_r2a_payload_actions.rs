//! R2A end-to-end ActionContext tests: handlers read `$event` fields and
//! CHANGE state, proving the runtime wiring — `SemanticEventEnvelope`
//! → one payload path → `ActionContext.event` → `EventHandlers`
//! ActionList execution. These are NOT envelope-inspection tests; every
//! assertion is on executed action results (app/self state).
//!
//! Covered: bubbled parent Tap ($event.pointerId / .button / .local.x +
//! `$self` owner scope), Hover Enter/Leave ($event.phase / .local.x /
//! .pointerType, touch suppressed), Pan start/update/end
//! ($event.start/.delta/.translation/.velocity), and exact Scale/Rotate
//! start + per-frame delta snapshots through actions.

use jian_core::geometry::{point, Point};
use jian_core::gesture::{MouseButtons, PointerEvent, PointerId, PointerKind, PointerPhase};
use jian_core::Runtime;

fn runtime_with<S: AsRef<str>>(op: S) -> Runtime {
    let mut rt = Runtime::new();
    rt.load_str(op.as_ref()).unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    rt
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

// ---------------------------------------------------------------------
// 1. Bubbled parent Tap: reads pointerId / button / local.x, owner scope
// ---------------------------------------------------------------------

const PARENT_TAP_OP: &str = r##"{"version":"0.8.0",
  "children":[{"type":"frame","id":"root","x":100,"y":100,"width":400,"height":400,
    "events":{"onTap":[
      {"set":{"$self.ptrId":"$event.pointerId"}},
      {"set":{"$self.button":"$event.button"}},
      {"set":{"$self.localX":"$event.local.x"}},
      {"set":{"$self.phase":"$event.phase"}}
    ]},
    "children":[{"type":"rectangle","id":"child","x":10,"y":10,"width":100,"height":100}]}]}"##;

/// Hit the handler-less child; the parent owns onTap. The action reads
/// id/button/local from `$event` and writes `$self` — the assertions
/// prove the payload reached the ActionContext AND that `$self`/`local`
/// resolve against the bubbled OWNER, not the hit child.
#[test]
fn bubbled_parent_tap_action_reads_pointer_fields_and_uses_owner_scope() {
    let mut rt = runtime_with(PARENT_TAP_OP);
    let child_key = rt.document.as_ref().unwrap().tree.get("child").unwrap();
    // Child rect: absolute (110,110)-(210,210); tap center (160,160).
    let at = point(160.0, 160.0);

    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, at, 0));
    let up = rt.dispatch_pointer_events(mouse(1, PointerPhase::Up, at, 50));
    assert_eq!(up.len(), 1, "one Tap envelope");
    assert_eq!(
        up[0].event.node(),
        child_key,
        "semantic targets the HIT child; the handler owner is the parent"
    );

    // Executed action results on the PARENT scope:
    assert_eq!(
        rt.state
            .self_get("", "root", "ptrId")
            .and_then(|v| v.as_f64()),
        Some(1.0),
        "$event.pointerId read by the action"
    );
    assert_eq!(
        rt.state
            .self_get("", "root", "button")
            .and_then(|v| v.as_str().map(str::to_owned)),
        Some("left".to_owned()),
        "initiating Down's provable LEFT retained on the Tap Up"
    );
    assert_eq!(
        rt.state
            .self_get("", "root", "localX")
            .and_then(|v| v.as_f64()),
        Some(60.0),
        "$event.local.x is parent-relative: (160,160) − (100,100)"
    );
    assert_eq!(
        rt.state
            .self_get("", "root", "phase")
            .and_then(|v| v.as_str().map(str::to_owned)),
        Some("up".to_owned()),
        "phase stays from the triggering Up"
    );
    // The hit child scope never received the `$self` writes.
    assert!(
        rt.state.self_get("", "child", "ptrId").is_none(),
        "$self must scope to the resolved handler owner"
    );
}

// ---------------------------------------------------------------------
// 2. Hover: read $event, change state; touch stays suppressed
// ---------------------------------------------------------------------

const HOVER_OP: &str = r##"{"version":"0.8.0",
  "state":{"hovers":{"type":"int","default":0},"leaves":{"type":"int","default":0}},
  "children":[{"type":"rectangle","id":"btn","x":40,"y":60,"width":200,"height":100,
    "events":{
      "onHoverEnter":[
        {"set":{"$app.hovers":"$app.hovers + 1"}},
        {"set":{"$app.hoverType":"$event.pointerType"}},
        {"set":{"$app.hoverPhase":"$event.phase"}},
        {"set":{"$app.hoverX":"$event.local.x"}}
      ],
      "onHoverLeave":[
        {"set":{"$app.leaves":"$app.leaves + 1"}},
        {"set":{"$app.leaveX":"$event.local.x"}}
      ]
    }}]}"##;

/// A real onHoverEnter/onHoverLeave chain executes with `$event` (type,
/// phase, node-local x) and mutates state. A Touch Hover phase must stay
/// fully suppressed: no envelope, no handler, no state change, and the
/// hover cache is not poisoned (a later Mouse Enter still fires).
#[test]
fn hover_actions_read_event_fields_and_touch_hover_stays_suppressed() {
    let mut rt = runtime_with(HOVER_OP);

    // Mouse enter at (140,110) → local (100,50).
    let first = rt.dispatch_pointer_events(mouse(3, PointerPhase::Hover, point(140.0, 110.0), 100));
    assert_eq!(first.len(), 1, "HoverEnter envelope");
    assert_eq!(rt.state.app_get("hovers").unwrap().as_i64(), Some(1));
    assert_eq!(
        rt.state
            .app_get("hoverType")
            .and_then(|v| v.as_str().map(str::to_owned)),
        Some("mouse".to_owned())
    );
    assert_eq!(
        rt.state
            .app_get("hoverPhase")
            .and_then(|v| v.as_str().map(str::to_owned)),
        Some("hover".to_owned())
    );
    assert_eq!(
        rt.state.app_get("hoverX").and_then(|v| v.as_f64()),
        Some(100.0)
    );

    // Touch hover is suppressed even inside the rect: no envelope, no
    // handler, no state change.
    let suppressed =
        rt.dispatch_pointer_events(touch(4, PointerPhase::Hover, point(140.0, 110.0), 200));
    assert!(
        suppressed.is_empty(),
        "touch hover must never emit, got {suppressed:?}"
    );
    assert_eq!(rt.state.app_get("hovers").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("leaves").unwrap().as_i64(), Some(0));

    // Mouse leave to the void: HoverLeave action runs with $event.
    let leave = rt.dispatch_pointer_events(mouse(3, PointerPhase::Hover, point(700.0, 500.0), 300));
    assert_eq!(leave.len(), 1, "HoverLeave envelope");
    assert_eq!(rt.state.app_get("leaves").unwrap().as_i64(), Some(1));
    assert_eq!(
        rt.state.app_get("leaveX").and_then(|v| v.as_f64()),
        Some(660.0)
    );

    // Touch hover must not have poisoned the cache: re-enter fires.
    let reenter =
        rt.dispatch_pointer_events(mouse(3, PointerPhase::Hover, point(140.0, 110.0), 400));
    assert_eq!(reenter.len(), 1, "re-enter after leave");
    assert_eq!(rt.state.app_get("hovers").unwrap().as_i64(), Some(2));
}

// ---------------------------------------------------------------------
// 3. Pan: start/update/end read $event.start/.delta/.translation/.velocity
// ---------------------------------------------------------------------

const PAN_ACTIONS_OP: &str = r##"{"version":"0.8.0",
  "state":{
    "panStarts":{"type":"int","default":0},"panUpdates":{"type":"int","default":0},
    "panEnds":{"type":"int","default":0}},
  "children":[{"type":"rectangle","id":"btn","x":40,"y":60,"width":200,"height":100,
    "events":{
      "onPanStart":[
        {"set":{"$app.panStarts":"$app.panStarts + 1"}},
        {"set":{"$app.panStartX":"$event.start.x"}},
        {"set":{"$app.panStartY":"$event.start.y"}},
        {"set":{"$app.panStartCurY":"$event.current.y"}},
        {"set":{"$app.panStartDeltaX":"$event.delta.x"}},
        {"set":{"$app.panStartTransX":"$event.translation.x"}},
        {"set":{"$app.panStartVelX":"$event.velocity.x"}},
        {"set":{"$app.panStartPhase":"$event.phase"}},
        {"set":{"$app.panStartButton":"$event.button"}}
      ],
      "onPanUpdate":[
        {"set":{"$app.panUpdates":"$app.panUpdates + 1"}},
        {"set":{"$app.panUpdDeltaX":"$event.delta.x"}},
        {"set":{"$app.panUpdTransX":"$event.translation.x"}},
        {"set":{"$app.panUpdVelX":"$event.velocity.x"}}
      ],
      "onPanEnd":[
        {"set":{"$app.panEnds":"$app.panEnds + 1"}},
        {"set":{"$app.panEndDeltaX":"$event.delta.x"}},
        {"set":{"$app.panEndVelX":"$event.velocity.x"}}
      ]
    }}]}"##;

/// The onPan* ActionLists read the factual gesture fields from the
/// `ActionContext.event` and write state — each handler ran once with
/// the exact values from the recognizer (start=Down, current/delta/
/// translation/velocity at the threshold-crossing Move; final-segment
/// end velocity).
#[test]
fn pan_actions_read_start_delta_translation_velocity() {
    let mut rt = runtime_with(PAN_ACTIONS_OP);
    let mut down = mouse(2, PointerPhase::Down, point(140.0, 110.0), 0);
    down.buttons = MouseButtons::LEFT;
    let _ = rt.dispatch_pointer_events(down);
    // Crossing Move: previous sample is the Down → delta (20,0), dt 0.125.
    let start = rt.dispatch_pointer_events(mouse(2, PointerPhase::Move, point(160.0, 110.0), 125));
    assert_eq!(start.len(), 1);
    let update = rt.dispatch_pointer_events(mouse(2, PointerPhase::Move, point(200.0, 110.0), 250));
    assert_eq!(update.len(), 1);
    let end = rt.dispatch_pointer_events(mouse(2, PointerPhase::Up, point(200.0, 110.0), 375));
    assert_eq!(end.len(), 1);

    // Every phase's ActionList executed exactly once.
    assert_eq!(rt.state.app_get("panStarts").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("panUpdates").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("panEnds").unwrap().as_i64(), Some(1));

    // PanStart facts: start = Down, current = Move, delta = current −
    // previous sample, translation = current − start, velocity = delta/dt,
    // phase/button from the triggering Move + retained initiating LEFT.
    assert_eq!(
        rt.state.app_get("panStartX").and_then(|v| v.as_f64()),
        Some(140.0)
    );
    assert_eq!(
        rt.state.app_get("panStartY").and_then(|v| v.as_f64()),
        Some(110.0)
    );
    assert_eq!(
        rt.state.app_get("panStartCurY").and_then(|v| v.as_f64()),
        Some(110.0)
    );
    assert_eq!(
        rt.state.app_get("panStartDeltaX").and_then(|v| v.as_f64()),
        Some(20.0)
    );
    assert_eq!(
        rt.state.app_get("panStartTransX").and_then(|v| v.as_f64()),
        Some(20.0)
    );
    assert_eq!(
        rt.state.app_get("panStartVelX").and_then(|v| v.as_f64()),
        Some(160.0)
    );
    assert_eq!(
        rt.state
            .app_get("panStartPhase")
            .and_then(|v| v.as_str().map(str::to_owned)),
        Some("move".to_owned())
    );
    assert_eq!(
        rt.state
            .app_get("panStartButton")
            .and_then(|v| v.as_str().map(str::to_owned)),
        Some("left".to_owned())
    );

    // PanUpdate: 40px over 125ms → 320 px/s.
    assert_eq!(
        rt.state.app_get("panUpdDeltaX").and_then(|v| v.as_f64()),
        Some(40.0)
    );
    assert_eq!(
        rt.state.app_get("panUpdTransX").and_then(|v| v.as_f64()),
        Some(60.0)
    );
    assert_eq!(
        rt.state.app_get("panUpdVelX").and_then(|v| v.as_f64()),
        Some(320.0)
    );

    // PanEnd: final segment (200,110) → (200,110) over 125ms = 0 px/s.
    assert_eq!(
        rt.state.app_get("panEndDeltaX").and_then(|v| v.as_f64()),
        Some(0.0)
    );
    assert_eq!(
        rt.state.app_get("panEndVelX").and_then(|v| v.as_f64()),
        Some(0.0)
    );
}

// ---------------------------------------------------------------------
// 4. Scale actions: exact start scale + per-frame delta
// ---------------------------------------------------------------------

const SCALE_ACTIONS_OP: &str = r##"{"version":"0.8.0",
  "state":{
    "scaleStarts":{"type":"int","default":0},"scaleUpdates":{"type":"int","default":0},
    "zoom":{"type":"float","default":1.0}},
  "children":[{"type":"rectangle","id":"canvas","width":800,"height":600,
    "events":{
      "onScaleStart":[
        {"set":{"$app.scaleStarts":"$app.scaleStarts + 1"}},
        {"set":{"$app.scaleStart":"$event.scale"}},
        {"set":{"$app.scaleStartDelta":"$event.deltaScale"}},
        {"set":{"$app.scaleStartFocalX":"$event.focal.x"}}
      ],
      "onScaleUpdate":[
        {"set":{"$app.scaleUpdates":"$app.scaleUpdates + 1"}},
        {"set":{"$app.scaleUpd":"$event.scale"}},
        {"set":{"$app.scaleUpdDelta":"$event.deltaScale"}},
        {"set":{"$app.zoom":"$event.scale"}}
      ],
      "onScaleEnd":[
        {"set":{"$app.zoom":"$event.scale"}}
      ]
    }}]}"##;

/// ScaleStart carries the ACTUAL threshold-crossing scale (1.25) and
/// deltaScale = scale − 1 (0.25); ScaleUpdate carries absolute scale and
/// the per-frame delta (0.25) — all read through `$event` by actions.
#[test]
fn scale_actions_carry_exact_start_scale_and_delta() {
    let mut rt = runtime_with(SCALE_ACTIONS_OP);
    let _ = rt.dispatch_pointer_events(mouse(0, PointerPhase::Down, point(200.0, 300.0), 0));
    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, point(400.0, 300.0), 10));
    // dist 200 → 250: scale 1.25, deltaScale 0.25, focal (275, 300).
    let start = rt.dispatch_pointer_events(mouse(0, PointerPhase::Move, point(150.0, 300.0), 20));
    assert_eq!(start.len(), 1, "ScaleStart runs before any update");
    // dist 200 → 300: scale 1.5, deltaScale 0.25.
    let update = rt.dispatch_pointer_events(mouse(0, PointerPhase::Move, point(100.0, 300.0), 30));
    assert_eq!(update.len(), 1);
    let end = rt.dispatch_pointer_events(mouse(0, PointerPhase::Up, point(100.0, 300.0), 40));
    assert_eq!(end.len(), 1);

    assert_eq!(rt.state.app_get("scaleStarts").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("scaleUpdates").unwrap().as_i64(), Some(1));
    assert_eq!(
        rt.state.app_get("scaleStart").and_then(|v| v.as_f64()),
        Some(1.25)
    );
    assert_eq!(
        rt.state.app_get("scaleStartDelta").and_then(|v| v.as_f64()),
        Some(0.25)
    );
    assert_eq!(
        rt.state
            .app_get("scaleStartFocalX")
            .and_then(|v| v.as_f64()),
        Some(275.0)
    );
    assert_eq!(
        rt.state.app_get("scaleUpd").and_then(|v| v.as_f64()),
        Some(1.5)
    );
    assert_eq!(
        rt.state.app_get("scaleUpdDelta").and_then(|v| v.as_f64()),
        Some(0.25)
    );
    assert_eq!(rt.state.app_get("zoom").and_then(|v| v.as_f64()), Some(1.5));
}

// ---------------------------------------------------------------------
// 5. Rotate actions: exact start rotation + wrapped per-frame delta
// ---------------------------------------------------------------------

const ROTATE_ACTIONS_OP: &str = r##"{"version":"0.8.0",
  "state":{
    "rotStarts":{"type":"int","default":0},"rotUpdates":{"type":"int","default":0},
    "rotation":{"type":"float","default":0.0}},
  "children":[{"type":"rectangle","id":"canvas","width":800,"height":600,
    "events":{
      "onRotateStart":[
        {"set":{"$app.rotStarts":"$app.rotStarts + 1"}},
        {"set":{"$app.rotStart":"$event.rotation"}},
        {"set":{"$app.rotStartDelta":"$event.deltaRotation"}}
      ],
      "onRotateUpdate":[
        {"set":{"$app.rotUpdates":"$app.rotUpdates + 1"}},
        {"set":{"$app.rotUpd":"$event.rotation"}},
        {"set":{"$app.rotUpdDelta":"$event.deltaRotation"}},
        {"set":{"$app.rotation":"$event.radians"}}
      ],
      "onRotateEnd":[
        {"set":{"$app.rotation":"$event.rotation"}}
      ]
    }}]}"##;

/// RotateStart reports the ACTUAL threshold-crossing rotation (45° =
/// π/4) with deltaRotation = rotation; RotateUpdate reports absolute
/// rotation, the wrapped per-frame delta (rotation − π/4), and keeps the
/// legacy `radians` key readable.
#[test]
fn rotate_actions_carry_exact_start_rotation_and_wrapped_delta() {
    let mut rt = runtime_with(ROTATE_ACTIONS_OP);
    let _ = rt.dispatch_pointer_events(mouse(0, PointerPhase::Down, point(300.0, 300.0), 0));
    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, point(600.0, 300.0), 10));
    let start = rt.dispatch_pointer_events(mouse(1, PointerPhase::Move, point(600.0, 600.0), 20));
    assert_eq!(start.len(), 1);
    let update = rt.dispatch_pointer_events(mouse(1, PointerPhase::Move, point(600.0, 900.0), 30));
    assert_eq!(update.len(), 1);
    let end = rt.dispatch_pointer_events(mouse(1, PointerPhase::Up, point(600.0, 900.0), 40));
    assert_eq!(end.len(), 1);

    assert_eq!(rt.state.app_get("rotStarts").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("rotUpdates").unwrap().as_i64(), Some(1));

    let start_rot = rt
        .state
        .app_get("rotStart")
        .and_then(|v| v.as_f64())
        .unwrap();
    let start_delta = rt
        .state
        .app_get("rotStartDelta")
        .and_then(|v| v.as_f64())
        .unwrap();
    assert!(
        (start_rot - std::f64::consts::FRAC_PI_4).abs() < 1e-5,
        "start rotation must be 45°, got {start_rot}"
    );
    assert_eq!(start_rot, start_delta, "deltaRotation = rotation at start");

    let upd_rot = rt.state.app_get("rotUpd").and_then(|v| v.as_f64()).unwrap();
    let upd_delta = rt
        .state
        .app_get("rotUpdDelta")
        .and_then(|v| v.as_f64())
        .unwrap();
    assert!((upd_rot - 1.1071).abs() < 1e-3, "got {upd_rot}");
    assert!(
        (upd_delta - (upd_rot - std::f64::consts::FRAC_PI_4)).abs() < 1e-6,
        "per-frame delta = current − previous rotation, got {upd_delta}"
    );
    // The legacy `radians` key stays readable on RotateUpdate.
    let radians = rt
        .state
        .app_get("rotation")
        .and_then(|v| v.as_f64())
        .unwrap();
    assert!((radians - upd_rot).abs() < 1e-6);
}

/// End reports the retained last rotation (and the legacy End still
/// delivers a payload through `$event.rotation`).
#[test]
fn rotate_end_action_reads_retained_rotation() {
    let mut rt = runtime_with(ROTATE_ACTIONS_OP);
    let _ = rt.dispatch_pointer_events(mouse(0, PointerPhase::Down, point(300.0, 300.0), 0));
    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, point(600.0, 300.0), 10));
    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Move, point(600.0, 600.0), 20));
    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Move, point(600.0, 900.0), 30));
    let end = rt.dispatch_pointer_events(mouse(1, PointerPhase::Up, point(600.0, 900.0), 40));
    assert_eq!(end.len(), 1);
    let end_rot = rt
        .state
        .app_get("rotation")
        .and_then(|v| v.as_f64())
        .unwrap();
    assert!(
        (end_rot - 1.1071).abs() < 1e-3,
        "end keeps the last rotation, got {end_rot}"
    );
}
