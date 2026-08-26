//! R2B repair-round regressions for Swipe.
//!
//! Three blocked repairs, each with the exact failing sequence plus
//! sign/boundary/diagonal coverage:
//!
//! - **Shared judged axis (A)**: direction, the min-distance gate and the
//!   min-velocity gate all judge the SAME axis. A fast perpendicular
//!   final segment must never satisfy a horizontal swipe; the distance
//!   gate is the PROJECTED travel on the judged axis (and
//!   `$event.distance` reports that projected value); the velocity gate
//!   is the same-axis component with the same SIGN as the direction.
//! - **Dynamic disabled mid-gesture (B)**: a Swipe owner that becomes
//!   `gestures.disabled` after Down but before claim cancels the captured
//!   session — delivery must never skip the child and execute the parent
//!   with the child's lower thresholds. A fresh Down re-resolves the
//!   parent normally.
//! - **Timer-before-current (C)**: pointer input at `t` drives all
//!   gesture deadlines `<= t` before the current arena dispatch, so a
//!   Move that crosses a LongPress deadline loses to the LongPress
//!   identically in the event-first and the tick-first interleavings.

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
// Part A — direction / distance / velocity share one judged axis
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

fn feed(r: &mut SwipeRecognizer, ev: PointerEvent) -> Option<SemanticEventEnvelope> {
    let mut pending = None;
    let mut h = ArenaHandle {
        pending_semantic: &mut pending,
    };
    let _ = r.handle_pointer(&ev, &mut h);
    pending
}

/// Claim-time envelopes are emitted from `accept` (so the arena can order
/// loser cancellations first) — call it once the recognizer is Claimed.
fn claim_env(r: &mut SwipeRecognizer) -> Option<SemanticEventEnvelope> {
    let mut pending = None;
    let mut h = ArenaHandle {
        pending_semantic: &mut pending,
    };
    r.accept(&mut h);
    pending
}

/// The exact A regression: total displacement is horizontal/Right, the
/// first segment is slow, and the fast FINAL segment is pure-vertical.
/// The vector-magnitude velocity gate must NOT accept it — the velocity
/// gate is the same-axis component, which is zero here.
#[test]
fn fast_perpendicular_final_segment_cannot_claim_horizontal_swipe() {
    let node = make_key();
    let mut r = SwipeRecognizer::new(1, node).with_axis_lock(AxisLock::Horizontal);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    // Slow horizontal arrival: 60px over 1000ms = 60 px/s < 320.
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 0.0, 1000));
    assert_eq!(
        r.state(),
        RecognizerState::Possible,
        "slow horizontal segment must not claim"
    );
    // Fast PURE-VERTICAL final segment: vector magnitude 4000 px/s, but
    // the judged (horizontal) axis component is zero.
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 40.0, 1010));
    assert_eq!(
        r.state(),
        RecognizerState::Possible,
        "horizontal lock keeps the dominance: this is not wrong-axis rejection"
    );
    // A decisive cross-axis total then rejects the sequence permanently.
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 90.0, 1020));
    assert_eq!(r.state(), RecognizerState::Rejected);
    assert!(
        claim_env(&mut r).is_none(),
        "rejected recognizer emits nothing"
    );
}

/// Same sequence under `Auto`: the axis RE-JUDGES from the total
/// displacement on every Move, so the same-axis velocity gate is always
/// the one the direction came from — perpendicular segments never claim,
/// and a genuinely vertical stroke claims as `Down`.
#[test]
fn auto_rejudges_the_axis_from_total_displacement() {
    let node = make_key();
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 0.0, 1000));
    assert_eq!(r.state(), RecognizerState::Possible);
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 40.0, 1010));
    assert_eq!(r.state(), RecognizerState::Possible);
    // The total is now vertical-dominant: primary axis = vertical,
    // projected |dy| = 90, same-axis velocity 5000 px/s, sign Down.
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 90.0, 1020));
    assert_eq!(r.state(), RecognizerState::Claimed);
    let env = claim_env(&mut r).expect("claim envelope");
    assert!(matches!(
        env.event,
        SemanticEvent::Swipe {
            direction: SwipeDirection::Down,
            distance,
            ..
        } if distance == 90.0
    ));
}

/// A fast segment whose axis component has the OPPOSITE sign of the
/// total-displacement direction must not claim (vector magnitude would
/// have passed the old gate).
#[test]
fn opposite_sign_segment_cannot_claim_against_total_direction() {
    let node = make_key();
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    // Slow rightward arrival: 100px over 1000ms = 100 px/s < 320.
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 100.0, 0.0, 1000));
    assert_eq!(r.state(), RecognizerState::Possible);
    // Fast LEFTWARD segment: total displacement is still Right (70px),
    // but the segment velocity x is -3000 px/s — opposite sign to Right.
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 70.0, 0.0, 1010));
    assert_eq!(
        r.state(),
        RecognizerState::Possible,
        "sign-reversal segment must not claim"
    );
    // A same-sign fast segment completes the swipe at the projected distance.
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 90.0, 0.0, 1020));
    assert!(matches!(
        claim_env(&mut r).map(|e| e.event),
        Some(SemanticEvent::Swipe {
            direction: SwipeDirection::Right,
            ..
        })
    ));
}

/// The distance gate is the PROJECTED travel on the judged axis, not the
/// Euclidean vector length — and `$event.distance` reports the projected
/// value.
#[test]
fn distance_gate_and_payload_are_projected_not_euclidean() {
    // (40,40): Euclidean 56.6 >= 48 (old gate would claim) but projected
    // |dx| = 40 < 48 — no claim until the stroke extends past 48 on x.
    let node = make_key();
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 40.0, 40.0, 100));
    assert_eq!(
        r.state(),
        RecognizerState::Possible,
        "Euclidean 56.6px must not pass the projected 48px gate"
    );
    // Extend to (60,40): projected 60 >= 48, vx = (20,0)/0.01 = 2000 px/s.
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 40.0, 110));
    let env = claim_env(&mut r).expect("claim");
    assert!(matches!(
        env.event,
        SemanticEvent::Swipe {
            direction: SwipeDirection::Right,
            distance,
            ..
        } if distance == 60.0
    ));
    assert_eq!(
        env.gesture.swipe_distance,
        Some(60.0),
        "payload distance = projected 60, NOT the Euclidean 72.1"
    );
    assert_eq!(env.gesture.swipe_velocity, Some(point(2000.0, 0.0)));
}

#[test]
fn diagonal_swipes_report_projected_distance_and_judged_direction() {
    // Horizontal-dominant diagonal: projected |dx| = 60, Euclidean 78.1.
    let node = make_key();
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 60.0, 50.0, 100));
    let env = claim_env(&mut r).expect("claim");
    assert!(
        matches!(
            env.event,
            SemanticEvent::Swipe {
                direction: SwipeDirection::Right,
                distance,
                ..
            } if distance == 60.0
        ),
        "got {:?}",
        env.event
    );
    assert_eq!(env.gesture.swipe_velocity, Some(point(600.0, 500.0)));

    // Vertical-dominant diagonal: projected |dy| = 60, direction Down.
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 50.0, 60.0, 100));
    let env = claim_env(&mut r).expect("claim");
    assert!(
        matches!(
            env.event,
            SemanticEvent::Swipe {
                direction: SwipeDirection::Down,
                distance,
                ..
            } if distance == 60.0
        ),
        "got {:?}",
        env.event
    );
}

/// The runtime path with `axisLock: "horizontal"` must apply the same
/// shared-axis gates: the perpendicular fast final segment never claims.
#[test]
fn runtime_perpendicular_final_segment_never_emits_swipe() {
    let op = r##"{"version":"0.8.0",
      "state":{"swipes":{"type":"int","default":0}},
      "children":[{"type":"frame","id":"btn","width":400,"height":400,
        "gestures":{"axisLock":"horizontal"},
        "events":{"onSwipe":[{"set":{"$app.swipes":"$app.swipes + 1"}}]}}]}"##;
    let mut rt = runtime_with(op);
    let c = node_center(&rt, "btn");

    // Keep every timestamp below the 500ms LongPress deadline so this
    // test isolates the Swipe gates (timer arbitration is part C).
    let _ = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
    let slow = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 60.0, c.y),
        400,
    ));
    assert!(
        slow.is_empty(),
        "slow horizontal segment must not claim, got {:?}",
        names(&slow)
    );
    let perpendicular = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 60.0, c.y + 40.0),
        410,
    ));
    assert!(
        perpendicular.is_empty(),
        "fast pure-vertical segment must not SwipeRight, got {:?}",
        names(&perpendicular)
    );
    let up = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Up,
        point(c.x + 60.0, c.y + 40.0),
        450,
    ));
    assert!(up.is_empty(), "got {up:?}");
    assert_eq!(
        rt.state.app_get("swipes").unwrap().as_i64(),
        Some(0),
        "no Swipe may fire for the whole sequence"
    );
}

#[test]
fn projected_48px_boundary_is_inclusive_and_47_9_is_not() {
    // (48,30): projected 48 == min_distance, vx = 480 px/s -> claim.
    let node = make_key();
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 48.0, 30.0, 100));
    assert_eq!(r.state(), RecognizerState::Claimed);
    let env = claim_env(&mut r).expect("claim");
    assert!(
        matches!(
            env.event,
            SemanticEvent::Swipe {
                direction: SwipeDirection::Right,
                distance,
                ..
            } if distance == 48.0
        ),
        "projected-48 boundary is inclusive"
    );

    // (47.9,30): projected 47.9 < 48 even though Euclidean ~= 56.6.
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 47.9, 30.0, 100));
    assert_eq!(r.state(), RecognizerState::Possible);
}

#[test]
fn velocity_boundary_is_the_same_axis_component() {
    // (80,60) over 250ms: axis component vx = 320 px/s (inclusive) even
    // though the vector magnitude is 400 px/s; (79,60) is vx = 316.
    let node = make_key();
    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 80.0, 60.0, 250));
    assert_eq!(r.state(), RecognizerState::Claimed);
    let env = claim_env(&mut r).expect("claim");
    assert!(
        matches!(
            env.event,
            SemanticEvent::Swipe {
                direction: SwipeDirection::Right,
                velocity,
                ..
            } if velocity == point(320.0, 240.0)
        ),
        "same-axis component 320 px/s must pass; full vector stays factual"
    );

    let mut r = SwipeRecognizer::new(1, node);
    let _ = feed(&mut r, mouse(1, PointerPhase::Down, 0.0, 0.0, 0));
    let _ = feed(&mut r, mouse(1, PointerPhase::Move, 79.0, 60.0, 250));
    assert_eq!(
        r.state(),
        RecognizerState::Possible,
        "316 px/s on the axis must hold"
    );
}

// ---------------------------------------------------------------------
// Part B — dynamic disabled invalidates the captured Swipe session
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

/// Child owns onSwipe at authored 30px/100px/s (action increments by 2),
/// parent owns onSwipe at defaults 48px/320px/s (increments by 1). The
/// child is gated by the dynamic `gestures.disabled` expression.
const CHILD_PARENT_SWIPE_OP: &str = r##"{"version":"0.8.0",
  "state":{"off":{"type":"bool","default":false},
           "swipes":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","x":0,"y":0,"width":400,"height":400,
    "events":{"onSwipe":[{"set":{"$app.swipes":"$app.swipes + 1"}}]},
    "children":[{"type":"rectangle","id":"child","x":10,"y":10,"width":100,"height":100,
      "gestures":{"disabled":"$app.off","swipeMinDistance":30,"swipeMinVelocity":100},
      "events":{"onSwipe":[{"set":{"$app.swipes":"$app.swipes + 2"}}]}}]}]}"##;

#[test]
fn disabled_mid_gesture_cancels_captured_swipe_session() {
    let mut rt = runtime_with(CHILD_PARENT_SWIPE_OP);
    let c = node_center(&rt, "child");
    let parent_key = rt.document.as_ref().unwrap().tree.get("root").unwrap();

    // Down while the CHILD is enabled: the Swipe recognizer is installed
    // at the child with ITS authored 30px/100px/s thresholds.
    let _ = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
    // The state flips mid-gesture: the child's `gestures.disabled` is now
    // truthy, BEFORE the sequence reaches the claim.
    rt.state.app_set("off", serde_json::json!(true));

    // 40px over 200ms = 200 px/s: qualifies the child (30/100) but not the
    // parent (48/320). The captured session must be CANCELLED — neither
    // the child action (+2) nor a parent-skipped delivery (+1) may run.
    let move_ev = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 40.0, c.y),
        200,
    ));
    assert!(
        move_ev.is_empty(),
        "disabled owner must cancel the captured Swipe, got {:?}",
        names(&move_ev)
    );
    // Even a decisively fast far segment stays silent: the session is
    // closed, not merely holding a stricter gate.
    let fast = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 60.0, c.y),
        300,
    ));
    assert!(
        fast.is_empty(),
        "captured session is cancelled outright, got {:?}",
        names(&fast)
    );
    let up = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Up,
        point(c.x + 60.0, c.y),
        400,
    ));
    assert!(up.is_empty(), "got {up:?}");
    assert_eq!(
        rt.state.app_get("swipes").unwrap().as_i64(),
        Some(0),
        "neither the child nor the parent Swipe action may run"
    );

    // A FRESH Down after the flip re-resolves the nearest ENABLED owner:
    // the parent governs with 48/320. 40px/200ms still fails the parent...
    let _ = rt.dispatch_pointer_events(mouse_event(2, PointerPhase::Down, c, 500));
    let too_small = rt.dispatch_pointer_events(mouse_event(
        2,
        PointerPhase::Move,
        point(c.x + 40.0, c.y),
        700,
    ));
    assert!(too_small.is_empty(), "parent 48px gate holds");
    // ...and exactly 80px over 250ms (= 320 px/s) claims at the PARENT
    // (increment 1, not the child's 2), with the parent as the node.
    let parent_swipe = rt.dispatch_pointer_events(mouse_event(
        2,
        PointerPhase::Move,
        point(c.x + 80.0, c.y),
        750,
    ));
    assert_eq!(names(&parent_swipe), ["onSwipe"], "got {parent_swipe:?}");
    assert_eq!(
        parent_swipe[0].event.node(),
        parent_key,
        "fresh Down resolves the parent owner"
    );
    assert_eq!(
        rt.state.app_get("swipes").unwrap().as_i64(),
        Some(1),
        "exactly the parent action ran exactly once"
    );
    let up = rt.dispatch_pointer_events(mouse_event(
        2,
        PointerPhase::Up,
        point(c.x + 80.0, c.y),
        900,
    ));
    assert!(up.is_empty(), "one-shot Swipe, got {up:?}");
}

// ---------------------------------------------------------------------
// Part C — timer-before-current arena dispatch
// ---------------------------------------------------------------------

/// onLongPress + onSwipe on the same node: a Move that arrives after the
/// LongPress deadline (500ms) must lose to the LongPress in BOTH the
/// event-first and the tick-first interleavings.
const LONG_SWIPE_OP: &str = r##"{"version":"0.8.0",
  "state":{"long":{"type":"int","default":0},"swipes":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"btn","width":400,"height":400,
    "events":{
      "onLongPress":[{"set":{"$app.long":"$app.long + 1"}}],
      "onSwipe":[{"set":{"$app.swipes":"$app.swipes + 1"}}]}}]}"##;

/// Event-first (no explicit tick): the pointer input itself must drive
/// the LongPress deadline before the current Move reaches the arena.
#[test]
fn longpress_wins_over_swipe_when_input_crosses_deadline_event_first() {
    let mut rt = runtime_with(LONG_SWIPE_OP);
    let c = node_center(&rt, "btn");
    let _ = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
    // Sub-slop nudge keeps the LongPress live (2px < 8px slop).
    let _ = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 2.0, c.y),
        100,
    ));
    // Second nudge BEFORE the deadline so the final segment's own clock
    // is short: 60px over 10ms = 6000 px/s, projected 62px — a Swipe that
    // would otherwise claim — but the LongPress deadline (500) is crossed
    // by this event's timestamp (510) and must win FIRST.
    let _ = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 2.0, c.y),
        490,
    ));
    let fast = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 62.0, c.y),
        510,
    ));
    assert_eq!(
        names(&fast),
        ["onLongPress"],
        "event-first: due LongPress precedes the current Move, got {:?}",
        names(&fast)
    );
    // The current fast Move was fed to the ALREADY-RESOLVED arena: no
    // Swipe ever fires, and the LongPress action ran exactly once.
    assert_eq!(rt.state.app_get("long").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("swipes").unwrap().as_i64(), Some(0));
    let up = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Up,
        point(c.x + 62.0, c.y),
        700,
    ));
    assert!(up.is_empty(), "long press sequence ends silent, got {up:?}");
    assert_eq!(rt.state.app_get("swipes").unwrap().as_i64(), Some(0));
}

#[test]
fn longpress_wins_over_swipe_when_input_crosses_deadline_tick_first() {
    let mut rt = runtime_with(LONG_SWIPE_OP);
    let c = node_center(&rt, "btn");
    let _ = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
    let _ = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 2.0, c.y),
        100,
    ));
    let _ = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 2.0, c.y),
        490,
    ));
    // Tick-first: the host's tick at the deadline claims the LongPress.
    let ticked = rt.tick(510);
    assert_eq!(
        ticked.iter().map(|e| e.handler_key()).collect::<Vec<_>>(),
        ["onLongPress"],
        "tick-first claim"
    );
    // The same fast Move now finds an already-resolved arena: silent.
    let fast = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 62.0, c.y),
        510,
    ));
    assert!(fast.is_empty(), "got {fast:?}");
    assert_eq!(rt.state.app_get("long").unwrap().as_i64(), Some(1));
    assert_eq!(
        rt.state.app_get("swipes").unwrap().as_i64(),
        Some(0),
        "Swipe never fires in either interleaving"
    );
}

/// The PUBLIC PointerRouter dispatch path must also return due timer
/// semantics before the current event's arena routing.
#[test]
fn public_router_delivers_due_longpress_before_current() {
    let mut rt = runtime_with(LONG_SWIPE_OP);
    let c = node_center(&rt, "btn");
    let doc = rt.document.as_ref().unwrap();
    let spatial = &rt.spatial;
    let router = &mut rt.gestures;

    assert!(router
        .dispatch_enveloped(mouse_event(1, PointerPhase::Down, c, 0), doc, spatial)
        .is_empty());
    assert!(router
        .dispatch_enveloped(
            mouse_event(1, PointerPhase::Move, point(c.x + 2.0, c.y), 100),
            doc,
            spatial
        )
        .is_empty());
    assert!(router
        .dispatch_enveloped(
            mouse_event(1, PointerPhase::Move, point(c.x + 2.0, c.y), 490),
            doc,
            spatial
        )
        .is_empty());
    // The fast Move at t=510: the router ticks the arena at the event's
    // timestamp BEFORE feeding the current event, so the due LongPress is
    // returned first and the Swipe recognizer never sees the Move.
    let due = router.dispatch_enveloped(
        mouse_event(1, PointerPhase::Move, point(c.x + 62.0, c.y), 510),
        doc,
        spatial,
    );
    assert_eq!(
        due.iter()
            .map(|e| e.event.handler_key())
            .collect::<Vec<_>>(),
        ["onLongPress"],
        "public dispatch must return due timer first, got {due:?}"
    );
    assert!(
        due.iter().all(|e| e.event.handler_key() != "onSwipe"),
        "Swipe must never fire, got {due:?}"
    );
    // Nothing left buffered, nothing pending in the arena.
    assert!(router.tick_enveloped(10_000).is_empty());
}
