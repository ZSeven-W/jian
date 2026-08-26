//! R2B Swipe semantics — recognizer gates and router installation.
//!
//! Part 1 drives `SwipeRecognizer` directly (four directions, distance +
//! velocity gates, axis locks, wrong axis, boundary equality,
//! Cancel/Up-before-claim, initiating button, no invented timestamps).
//! Part 2 drives the public `Runtime` pipeline (Pan-wins-over-Swipe
//! installation, disabled/empty Pan, dynamic owners, nearest-owner
//! thresholds, exact `[PressCancel, Swipe]` order, Tap never firing
//! after a Swipe, and the legacy eager-Pan fallback).

use jian_core::document::NodeKey;
use jian_core::geometry::{point, Point};
use jian_core::gesture::recognizers::SwipeRecognizer;
use jian_core::gesture::{
    ArenaHandle, MouseButtons, PointerEvent, PointerId, PointerKind, PointerPhase, Recognizer,
    RecognizerState, SemanticEvent, SemanticEventEnvelope, SwipeDirection,
};
use jian_core::Runtime;
use jian_ops_schema::gestures::AxisLock;
use slotmap::SlotMap;

// ---------------------------------------------------------------------
// Part 1 — recognizer-level gates
// ---------------------------------------------------------------------

fn make_key() -> NodeKey {
    let mut sm: SlotMap<NodeKey, u32> = SlotMap::with_key();
    sm.insert(0)
}

fn mouse(id: u32, phase: PointerPhase, x: f32, y: f32, t_ms: u64) -> PointerEvent {
    PointerEvent {
        id: PointerId(id),
        kind: PointerKind::Mouse,
        phase,
        position: point(x, y),
        pressure: 0.0,
        buttons: MouseButtons::LEFT,
        modifiers: Default::default(),
        tilt: None,
        t_ms,
    }
}

/// Feed one event; return the pending envelope (claim events are only
/// emitted from `accept`, so call accept afterwards for those).
fn feed(r: &mut SwipeRecognizer, ev: PointerEvent) -> Option<SemanticEventEnvelope> {
    let mut pending = None;
    let mut h = ArenaHandle {
        pending_semantic: &mut pending,
    };
    let _ = r.handle_pointer(&ev, &mut h);
    pending
}

/// Drive Down → one Move → accept, returning the claim envelope.
fn claim_with_move(x: f32, y: f32, t_ms: u64, axis_lock: AxisLock) -> SemanticEventEnvelope {
    let mut r = SwipeRecognizer::new(1, make_key()).with_axis_lock(axis_lock);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, x, y, t_ms));
    assert_eq!(r.state(), RecognizerState::Claimed, "expected claim");
    let mut pending = None;
    let mut h = ArenaHandle {
        pending_semantic: &mut pending,
    };
    r.accept(&mut h);
    pending.expect("claim envelope")
}

#[test]
fn four_directions_judge_from_total_displacement() {
    // Down (0,0) → Move (60,0): Right (velocity 600 px/s).
    let right = claim_with_move(60.0, 0.0, 100, AxisLock::Auto);
    assert!(
        matches!(
            right.event,
            SemanticEvent::Swipe {
                direction: SwipeDirection::Right,
                distance,
                velocity,
                ..
            } if (distance - 60.0).abs() < f32::EPSILON && velocity == point(600.0, 0.0)
        ),
        "got {:?}",
        right.event
    );
    assert_eq!(
        right.gesture.swipe_direction.as_deref(),
        Some("right"),
        "gesture facts carry the wire direction string"
    );

    let down = claim_with_move(0.0, 60.0, 100, AxisLock::Auto);
    assert!(matches!(
        down.event,
        SemanticEvent::Swipe {
            direction: SwipeDirection::Down,
            ..
        }
    ));

    let left = claim_with_move(-60.0, 0.0, 100, AxisLock::Auto);
    assert!(matches!(
        left.event,
        SemanticEvent::Swipe {
            direction: SwipeDirection::Left,
            ..
        }
    ));

    let up = claim_with_move(0.0, -60.0, 100, AxisLock::Auto);
    assert!(matches!(
        up.event,
        SemanticEvent::Swipe {
            direction: SwipeDirection::Up,
            ..
        }
    ));

    // The y axis points down: upward motion is dy < 0 → Up.
    assert_eq!(up.gesture.swipe_velocity, Some(point(0.0, -600.0)));
}

#[test]
fn distance_and_velocity_gates_both_must_pass() {
    let node = make_key();

    // Distance 48 ✓ but velocity 240 px/s ✗ (60px over 250ms).
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 0.0, 250));
    assert_eq!(r.state(), RecognizerState::Possible, "slow gate holds");
    // Distance then rises to 80 while the segment stays slow.
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 80.0, 0.0, 500));
    assert_eq!(
        r.state(),
        RecognizerState::Possible,
        "slow segment must not claim even past 48px"
    );

    // Velocity 600 ✓ but distance 30 ✗ — no claim at any time.
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 30.0, 0.0, 50));
    assert_eq!(r.state(), RecognizerState::Possible, "distance gate holds");

    // Both pass on the same Move → claim.
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 0.0, 100));
    assert_eq!(r.state(), RecognizerState::Claimed);

    // A prior slow Move that later accelerates: the fast segment claims.
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 40.0, 0.0, 400));
    assert_eq!(r.state(), RecognizerState::Possible);
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 0.0, 450));
    assert_eq!(r.state(), RecognizerState::Claimed, "fast segment claims");
}

#[test]
fn t_ms_zero_sequences_never_pass_the_velocity_gate() {
    let node = make_key();
    let mut r = SwipeRecognizer::new(1, node);
    // t_ms = 0 everywhere: no segment has a measurable dt, so no
    // velocity fact exists and the recognizer must never claim.
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 0.0, 0));
    assert_eq!(r.state(), RecognizerState::Possible);
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 100.0, 0.0, 0));
    assert_eq!(
        r.state(),
        RecognizerState::Possible,
        "no timestamps → no velocity → no claim"
    );
    // Up rejects the never-claimed sequence.
    let _ = feed(&mut r, mouse(1, PointerPhase::Up, 100.0, 0.0, 0));
    assert_eq!(r.state(), RecognizerState::Rejected);
}

#[test]
fn boundary_equality_distance_and_velocity_are_inclusive() {
    let node = make_key();
    // Exactly 48px, fast (480 px/s): distance boundary is inclusive.
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 48.0, 0.0, 100));
    assert_eq!(
        r.state(),
        RecognizerState::Claimed,
        "48px must pass `>= min_distance`"
    );
    let mut pending = None;
    let mut h = ArenaHandle {
        pending_semantic: &mut pending,
    };
    r.accept(&mut h);
    assert!(matches!(
        pending.map(|e| e.event),
        Some(SemanticEvent::Swipe { distance, .. }) if (distance - 48.0).abs() < f32::EPSILON
    ));

    // 47.5px stays under.
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 47.5, 0.0, 100));
    assert_eq!(r.state(), RecognizerState::Possible);

    // Exactly 320 px/s (= 80px over the exactly-representable 250ms) is
    // the inclusive velocity boundary — 79px over the same time is under.
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 80.0, 0.0, 250));
    assert_eq!(r.state(), RecognizerState::Claimed, "320 px/s passes");
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 79.0, 0.0, 250));
    assert_eq!(r.state(), RecognizerState::Possible, "316 px/s holds");
}

#[test]
fn axis_lock_auto_horizontal_vertical() {
    // Auto: a 45° tie resolves horizontal; horizontal-dominant Down.
    let tie = claim_with_move(60.0, 60.0, 100, AxisLock::Auto);
    assert!(matches!(
        tie.event,
        SemanticEvent::Swipe {
            direction: SwipeDirection::Right,
            ..
        }
    ));
    let vertical = claim_with_move(30.0, 60.0, 100, AxisLock::Auto);
    assert!(matches!(
        vertical.event,
        SemanticEvent::Swipe {
            direction: SwipeDirection::Down,
            ..
        }
    ));

    // Horizontal lock accepts a horizontal-dominant diagonal...
    let diagonal = claim_with_move(60.0, 50.0, 100, AxisLock::Horizontal);
    assert!(matches!(
        diagonal.event,
        SemanticEvent::Swipe {
            direction: SwipeDirection::Right,
            ..
        }
    ));
    // ...and Vertical lock the vertical-dominant diagonal.
    let diagonal = claim_with_move(50.0, 60.0, 100, AxisLock::Vertical);
    assert!(matches!(
        diagonal.event,
        SemanticEvent::Swipe {
            direction: SwipeDirection::Down,
            ..
        }
    ));
}

#[test]
fn wrong_axis_sequence_never_claims() {
    let node = make_key();

    // Horizontal lock, vertical-dominant Move: permanent rejection.
    let mut r = SwipeRecognizer::new(1, node).with_axis_lock(AxisLock::Horizontal);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 20.0, 80.0, 100));
    assert_eq!(
        r.state(),
        RecognizerState::Rejected,
        "vertical movement under Horizontal lock"
    );
    // A later horizontal Move must not resurrect the sequence.
    let mut pending = None;
    let mut h = ArenaHandle {
        pending_semantic: &mut pending,
    };
    let _ = r.handle_pointer(&mouse(1, PointerPhase::Move, 100.0, 80.0, 200), &mut h);
    assert_eq!(r.state(), RecognizerState::Rejected);
    assert!(pending.is_none(), "wrong-axis sequence never emits");

    // Vertical lock, horizontal-dominant Move: permanent rejection.
    let mut r = SwipeRecognizer::new(1, node).with_axis_lock(AxisLock::Vertical);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 80.0, 20.0, 100));
    assert_eq!(r.state(), RecognizerState::Rejected);

    // Even an exact 45° diagonal (tie) counts as horizontal under
    // Horizontal lock but is NOT horizontal under Vertical lock.
    let mut r = SwipeRecognizer::new(1, node).with_axis_lock(AxisLock::Vertical);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 60.0, 100));
    assert_eq!(r.state(), RecognizerState::Rejected);
}

#[test]
fn horizontal_lock_tolerates_sub_threshold_wrong_axis_jitter() {
    // A small vertical jitter before the decisive horizontal stroke must
    // not reject the sequence: the axis is judged from the total
    // displacement once it becomes meaningful (≥ min_distance).
    let node = make_key();
    let mut r = SwipeRecognizer::new(1, node).with_axis_lock(AxisLock::Horizontal);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 2.0, -8.0, 50));
    assert_eq!(
        r.state(),
        RecognizerState::Possible,
        "jitter is sub-threshold"
    );
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, -8.0, 100));
    assert_eq!(
        r.state(),
        RecognizerState::Claimed,
        "decisive horizontal stroke claims past the jitter"
    );
}

#[test]
fn cancel_and_up_before_claim_reject_without_emit() {
    let node = make_key();

    // Cancel mid-sequence (before the gates pass): rejected, no pending
    // event of its own.
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 30.0, 0.0, 50));
    assert_eq!(r.state(), RecognizerState::Possible, "under both gates");
    let cancel = feed(&mut r, mouse(1, PointerPhase::Cancel, 30.0, 0.0, 150));
    assert_eq!(r.state(), RecognizerState::Rejected);
    assert!(cancel.is_none());

    // Up before the claim: rejected, no Swipe.
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 30.0, 0.0, 50));
    assert_eq!(r.state(), RecognizerState::Possible);
    let up = feed(&mut r, mouse(1, PointerPhase::Up, 30.0, 0.0, 80));
    assert_eq!(r.state(), RecognizerState::Rejected);
    assert!(up.is_none());
}

#[test]
fn after_claim_no_duplicate_swipe_and_no_end_event() {
    let node = make_key();
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    assert!(feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 0.0, 100)).is_none());
    assert!(feed(&mut r, mouse(1, PointerPhase::Move, 120.0, 0.0, 200)).is_none());
    let up = feed(&mut r, mouse(1, PointerPhase::Up, 120.0, 0.0, 300));
    assert!(up.is_none(), "one-shot Swipe: no end event");
    let cancel = feed(&mut r, mouse(1, PointerPhase::Cancel, 120.0, 0.0, 400));
    assert!(cancel.is_none());
}

#[test]
fn claim_facts_keep_initiating_button_and_triggering_move() {
    let node = make_key();
    let mut r = SwipeRecognizer::new(1, node);
    // Down is a provable single LEFT button press.
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 0.0, 100));
    let mut pending = None;
    let mut h = ArenaHandle {
        pending_semantic: &mut pending,
    };
    r.accept(&mut h);
    let env = pending.expect("claim envelope");
    let f = env.pointer_facts.as_ref().expect("facts");
    // Facts stay from the triggering Move; the initiating LEFT survives.
    assert_eq!(f.phase, PointerPhase::Move);
    assert_eq!(f.position, point(60.0, 0.0));
    assert_eq!(f.t_ms, 100);
    assert_eq!(f.button, Some(MouseButtons::LEFT));
    assert_eq!(f.buttons, Some(MouseButtons::LEFT));
    assert_eq!(env.gesture.swipe_distance, Some(60.0));
    assert_eq!(env.gesture.swipe_velocity, Some(point(600.0, 0.0)));
}

// ---------------------------------------------------------------------
// Part 2 — router installation and competition
// ---------------------------------------------------------------------

fn runtime_with<S: AsRef<str>>(op: S) -> Runtime {
    let mut rt = Runtime::new();
    rt.load_str(op.as_ref()).unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    rt
}

fn node_center(rt: &Runtime, id: &str) -> Point {
    let key = rt.document.as_ref().unwrap().tree.get(id).expect(id);
    let rect = rt.layout.node_rect(key).unwrap();
    point(
        rect.min_x() + rect.size.width / 2.0,
        rect.min_y() + rect.size.height / 2.0,
    )
}

fn names(evs: &[SemanticEventEnvelope]) -> Vec<&'static str> {
    evs.iter().map(|e| e.event.handler_key()).collect()
}

fn mouse_event(id: u32, phase: PointerPhase, position: Point, t_ms: u64) -> PointerEvent {
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

const SWIPE_ONLY_OP: &str = r##"{"version":"0.8.0",
  "state":{"swipes":{"type":"int","default":0}},
  "children":[{"type":"rectangle","id":"btn","width":200,"height":100,
    "events":{"onSwipe":[{"set":{"$app.swipes":"$app.swipes + 1"}}]}}]}"##;

#[test]
fn runtime_emits_one_swipe_with_no_other_events() {
    let mut rt = runtime_with(SWIPE_ONLY_OP);
    let c = node_center(&rt, "btn");

    let down = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
    assert!(down.is_empty(), "no press handler → silent Down");

    let swipe = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 60.0, c.y),
        100,
    ));
    assert_eq!(names(&swipe), ["onSwipe"], "got {:?}", names(&swipe));
    assert!(matches!(
        swipe[0].event,
        SemanticEvent::Swipe {
            direction: SwipeDirection::Right,
            distance: 60.0,
            velocity,
            ..
        } if velocity == point(600.0, 0.0)
    ));

    let up = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Up,
        point(c.x + 60.0, c.y),
        200,
    ));
    assert!(up.is_empty(), "claim-time Swipe is one-shot, got {up:?}");
    assert_eq!(rt.state.app_get("swipes").unwrap().as_i64(), Some(1));
}

#[test]
fn pan_owner_precedence_wins_over_swipe() {
    let op = r##"{"version":"0.8.0",
      "state":{"swipes":{"type":"int","default":0},"pans":{"type":"int","default":0}},
      "children":[{"type":"rectangle","id":"btn","width":200,"height":100,
        "events":{
          "onPanStart":[{"set":{"$app.pans":"$app.pans + 1"}}],
          "onSwipe":[{"set":{"$app.swipes":"$app.swipes + 1"}}]}}]}"##;
    let mut rt = runtime_with(op);
    let c = node_center(&rt, "btn");

    let _ = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
    let move_ev = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 60.0, c.y),
        100,
    ));
    assert_eq!(
        names(&move_ev),
        ["onPanStart"],
        "ANY nonempty Pan hook installs Pan and never Swipe, got {:?}",
        names(&move_ev)
    );
    let up = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Up,
        point(c.x + 60.0, c.y),
        200,
    ));
    assert_eq!(names(&up), ["onPanEnd"]);
    assert_eq!(rt.state.app_get("pans").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("swipes").unwrap().as_i64(), Some(0));
}

#[test]
fn disabled_or_empty_pan_allows_swipe() {
    // Pan handlers statically slated via disabledEvents → the enabled
    // onSwipe owner installs Swipe instead of Pan.
    let disabled = r##"{"version":"0.8.0",
      "state":{"swipes":{"type":"int","default":0}},
      "children":[{"type":"rectangle","id":"btn","width":200,"height":100,
        "gestures":{"disabledEvents":["onPanStart","onPanUpdate","onPanEnd"]},
        "events":{
          "onPanStart":[{"set":{"$app.swipes":"$app.swipes + 1"}}],
          "onSwipe":[{"set":{"$app.swipes":"$app.swipes + 1"}}]}}]}"##;
    // Empty ActionLists declare no handler — same outcome.
    let empty = r##"{"version":"0.8.0",
      "state":{"swipes":{"type":"int","default":0}},
      "children":[{"type":"rectangle","id":"btn","width":200,"height":100,
        "events":{
          "onPanStart":[],
          "onSwipe":[{"set":{"$app.swipes":"$app.swipes + 1"}}]}}]}"##;
    for op in [disabled, empty] {
        let mut rt = runtime_with(op);
        let c = node_center(&rt, "btn");
        let _ = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
        let move_ev = rt.dispatch_pointer_events(mouse_event(
            1,
            PointerPhase::Move,
            point(c.x + 60.0, c.y),
            100,
        ));
        assert_eq!(
            names(&move_ev),
            ["onSwipe"],
            "disabled/empty Pan must allow Swipe, got {:?}",
            names(&move_ev)
        );
        assert_eq!(rt.state.app_get("swipes").unwrap().as_i64(), Some(1));
    }
}

/// Parent owns onSwipe at defaults (48px/320px/s); child owns onSwipe
/// with authored 30px/100px/s and a dynamic `gestures.disabled`.
const NEAREST_OWNER_OP: &str = r##"{"version":"0.8.0",
  "state":{"off":{"type":"bool","default":true},
           "swipes":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","x":0,"y":0,"width":400,"height":400,
    "events":{"onSwipe":[{"set":{"$app.swipes":"$app.swipes + 1"}}]},
    "children":[{"type":"rectangle","id":"child","x":10,"y":10,"width":100,"height":100,
      "gestures":{"disabled":"$app.off","swipeMinDistance":30,"swipeMinVelocity":100},
      "events":{"onSwipe":[{"set":{"$app.swipes":"$app.swipes + 1"}}]}}]}]}"##;

#[test]
fn nearest_owner_thresholds_govern_swipe() {
    let mut rt = runtime_with(NEAREST_OWNER_OP);
    let c = node_center(&rt, "child"); // (60,60)
    let child_key = rt.document.as_ref().unwrap().tree.get("child").unwrap();

    // Disabled child → parent governs: 40px at 200px/s is far enough and
    // fast enough for the child but not for the parent (48/320).
    let _ = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
    let slow = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 40.0, c.y),
        200,
    ));
    assert_eq!(
        slow.len(),
        0,
        "parent threshold must not claim, got {slow:?}"
    );
    let _ = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Up,
        point(c.x + 40.0, c.y),
        300,
    ));

    // Re-enable the child: the same 40px at 200px/s claims at the CHILD
    // (its authored thresholds govern; its node is the semantic target).
    rt.state.app_set("off", serde_json::json!(false));
    let _ = rt.dispatch_pointer_events(mouse_event(2, PointerPhase::Down, c, 400));
    let swipe = rt.dispatch_pointer_events(mouse_event(
        2,
        PointerPhase::Move,
        point(c.x + 40.0, c.y),
        600,
    ));
    assert_eq!(names(&swipe), ["onSwipe"], "got {:?}", names(&swipe));
    assert_eq!(
        swipe[0].event.node(),
        child_key,
        "nearest enabled owner is the semantic target"
    );
    assert_eq!(rt.state.app_get("swipes").unwrap().as_i64(), Some(1));
}

/// Press + Swipe fixture: exact cancellation ordering at the claim.
const PRESS_SWIPE_OP: &str = r##"{"version":"0.8.0",
  "state":{"swipes":{"type":"int","default":0},"cancelled":{"type":"int","default":0}},
  "children":[{"type":"rectangle","id":"btn","width":200,"height":100,
    "events":{
      "onPressStart":[{"set":{"$app.swipes":"$app.swipes + 0"}}],
      "onPressCancel":[{"set":{"$app.cancelled":"$app.cancelled + 1"}}],
      "onSwipe":[{"set":{"$app.swipes":"$app.swipes + 1"}}]}}]}"##;

#[test]
fn press_cancel_precedes_swipe_exactly_once() {
    let mut rt = runtime_with(PRESS_SWIPE_OP);
    let c = node_center(&rt, "btn");

    let down = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
    assert_eq!(names(&down), ["onPressStart"]);
    let swipe = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 60.0, c.y),
        100,
    ));
    assert_eq!(
        names(&swipe),
        ["onPressCancel", "onSwipe"],
        "arena must cancel the press before the claim-time Swipe, got {:?}",
        names(&swipe)
    );
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(1));
    let up = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Up,
        point(c.x + 60.0, c.y),
        200,
    ));
    assert!(up.is_empty());
    assert_eq!(rt.state.app_get("swipes").unwrap().as_i64(), Some(1));
}

/// Tap + Swipe fixture: a claimed Swipe must never leave a Tap behind.
const TAP_SWIPE_OP: &str = r##"{"version":"0.8.0",
  "state":{"taps":{"type":"int","default":0},"swipes":{"type":"int","default":0}},
  "children":[{"type":"rectangle","id":"btn","width":200,"height":100,
    "events":{
      "onTap":[{"set":{"$app.taps":"$app.taps + 1"}}],
      "onSwipe":[{"set":{"$app.swipes":"$app.swipes + 1"}}]}}]}"##;

#[test]
fn tap_never_fires_after_swipe() {
    let mut rt = runtime_with(TAP_SWIPE_OP);
    let c = node_center(&rt, "btn");

    let _ = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
    let swipe = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 60.0, c.y),
        100,
    ));
    assert_eq!(names(&swipe), ["onSwipe"]);
    let up = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Up,
        point(c.x + 60.0, c.y),
        200,
    ));
    assert!(up.is_empty(), "no Tap may follow a Swipe: {up:?}");
    assert_eq!(rt.state.app_get("swipes").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(0));
}

#[test]
fn slow_gesture_never_emits_swipe_and_tap_also_stays_silent() {
    let mut rt = runtime_with(TAP_SWIPE_OP);
    let c = node_center(&rt, "btn");

    let _ = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
    // 60px over 400ms = 150 px/s — distance ✓, velocity ✗. Every
    // timestamp stays BELOW the 500ms LongPress deadline so this test
    // isolates the Swipe slow-gate (timer-before-current arbitration is
    // its own regression; see gesture_r2b_swipe_regressions.rs).
    let slow_move = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 60.0, c.y),
        400,
    ));
    assert!(
        slow_move.is_empty(),
        "slow drag must not claim, got {slow_move:?}"
    );
    let up = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Up,
        point(c.x + 60.0, c.y),
        490,
    ));
    assert!(
        up.iter().all(|e| e.event.handler_key() != "onSwipe"),
        "Up after a slow drag must not emit Swipe, got {:?}",
        names(&up)
    );
    assert_eq!(rt.state.app_get("swipes").unwrap().as_i64(), Some(0));
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(0));
}

#[test]
fn wrong_axis_runtime_sequence_never_emits_swipe() {
    let op = r##"{"version":"0.8.0",
      "state":{"swipes":{"type":"int","default":0}},
      "children":[{"type":"rectangle","id":"btn","width":400,"height":400,
        "gestures":{"axisLock":"horizontal"},
        "events":{"onSwipe":[{"set":{"$app.swipes":"$app.swipes + 1"}}]}}]}"##;
    let mut rt = runtime_with(op);
    let c = node_center(&rt, "btn");

    // Fast, far vertical drag under `horizontal` lock.
    let _ = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
    let vertical = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x, c.y + 100.0),
        100,
    ));
    assert!(
        vertical.is_empty(),
        "wrong-axis Move must not emit, got {:?}",
        names(&vertical)
    );
    // A later fast horizontal Move cannot resurrect the sequence.
    let horizontal = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 100.0, c.y + 100.0),
        200,
    ));
    assert!(
        horizontal
            .iter()
            .all(|e| e.event.handler_key() != "onSwipe"),
        "wrong-axis sequence never emits, got {:?}",
        names(&horizontal)
    );
    let up = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Up,
        point(c.x + 100.0, c.y + 100.0),
        300,
    ));
    assert!(up.iter().all(|e| e.event.handler_key() != "onSwipe"));
    assert_eq!(rt.state.app_get("swipes").unwrap().as_i64(), Some(0));
}

/// No Pan and no Swipe handler: the R2A legacy eager Pan recognizer is
/// preserved (semantic stream still emits PanStart/PanEnd; the
/// dispatcher drops them when no handler exists).
const NO_PAN_SWIPE_OP: &str = r##"{"version":"0.8.0",
  "state":{"taps":{"type":"int","default":0}},
  "children":[{"type":"rectangle","id":"btn","width":200,"height":100,
    "events":{"onTap":[{"set":{"$app.taps":"$app.taps + 1"}}]}}]}"##;

#[test]
fn no_handler_chain_preserves_legacy_eager_pan_stream() {
    let mut rt = runtime_with(NO_PAN_SWIPE_OP);
    let c = node_center(&rt, "btn");
    let _ = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
    let move_ev = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 40.0, c.y),
        100,
    ));
    assert_eq!(
        names(&move_ev),
        ["onPanStart"],
        "legacy eager Pan semantic stream preserved, got {:?}",
        names(&move_ev)
    );
    let up = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Up,
        point(c.x + 40.0, c.y),
        200,
    ));
    assert_eq!(names(&up), ["onPanEnd"]);
}

/// A `t_ms = 0` click sequence on an onSwipe chain never claims: no
/// timestamp, no velocity fact, no Swipe.
#[test]
fn zero_timestamp_sequence_never_claims_at_runtime() {
    let mut rt = runtime_with(SWIPE_ONLY_OP);
    let c = node_center(&rt, "btn");
    let zt = |phase, x, y| mouse_event(1, phase, point(x, y), 0);
    let _ = rt.dispatch_pointer_events(zt(PointerPhase::Down, c.x, c.y));
    let move_ev = rt.dispatch_pointer_events(zt(PointerPhase::Move, c.x + 60.0, c.y));
    assert!(
        move_ev.iter().all(|e| e.event.handler_key() != "onSwipe"),
        "t_ms=0 must not invent velocity: {:?}",
        names(&move_ev)
    );
    let up = rt.dispatch_pointer_events(zt(PointerPhase::Up, c.x + 60.0, c.y));
    assert!(up.iter().all(|e| e.event.handler_key() != "onSwipe"));
    assert_eq!(rt.state.app_get("swipes").unwrap().as_i64(), Some(0));
}
