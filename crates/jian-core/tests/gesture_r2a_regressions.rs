//! R2A repair-round regressions (round 1), grouped by review finding:
//! 1. Pending-Tap state machine (flush-before-process, order-independent
//!    deadline, stale-buffer elimination).
//! 2. Dynamic `gestures.disabled` participates before arbitration and
//!    delivery (DoubleTap owner, widget activation, owner scope for
//!    `$self` / `$event.local`).
//! 3. ContextMenu facts (exactly-RIGHT only) and slider side effects
//!    (right-button no mutation/drag, Cancel disarms).
//! 4. PressCancel facts during multi-pointer claim (current-Move witness).
//! 5. Handler/owner-aware thresholds (one recognizer per kind per path,
//!    ancestor authored thresholds win; empty/disabledEvents ignored).
//! 6. Public API bridge (plain `emit`, legacy `reject`, custom
//!    recognizers still compile).

use jian_core::document::NodeKey;
use jian_core::geometry::{point, Point};
use jian_core::gesture::{
    Arena, ArenaHandle, MouseButtons, PointerEvent, PointerFacts, PointerId, PointerKind,
    PointerPhase, Recognizer, RecognizerId, RecognizerState, SemanticEvent, SemanticEventEnvelope,
};
use jian_core::Runtime;

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

fn names(evs: &[SemanticEvent]) -> Vec<&'static str> {
    evs.iter().map(|e| e.handler_key()).collect()
}

fn envelope_names(evs: &[SemanticEventEnvelope]) -> Vec<&'static str> {
    evs.iter().map(|e| e.event.handler_key()).collect()
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
// 1. Pending-Tap state machine
// ---------------------------------------------------------------------

/// `a` owns Tap + DoubleTap; `b` is Tap-only; both are siblings in a
/// plain frame so the owner chains stay disjoint.
const TWO_TARGETS_OP: &str = r##"{"version":"0.8.0",
  "state":{"taps":{"type":"int","default":0},"doubles":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","width":400,"height":200,"children":[
    {"type":"rectangle","id":"a","x":0,"y":0,"width":100,"height":100,
     "events":{"onTap":[{"set":{"$app.taps":"$app.taps + 1"}}],
               "onDoubleTap":[{"set":{"$app.doubles":"$app.doubles + 1"}}]}},
    {"type":"rectangle","id":"b","x":200,"y":0,"width":100,"height":100,
     "events":{"onTap":[{"set":{"$app.taps":"$app.taps + 1"}}]}}]}]}"##;

const DOUBLE_TAP_OP: &str = r##"{"version":"0.8.0",
  "state":{"taps":{"type":"int","default":0},"doubles":{"type":"int","default":0}},
  "children":[{"type":"rectangle","id":"btn","width":200,"height":100,
    "events":{"onTap":[{"set":{"$app.taps":"$app.taps + 1"}}],
              "onDoubleTap":[{"set":{"$app.doubles":"$app.doubles + 1"}}]}}]}"##;

/// A completed Tap on a Tap-only target must flush an older pending
/// DoubleTap window immediately (delivering BOTH taps), and a later Tap
/// on the DoubleTap owner must not pair with the stale buffer.
#[test]
fn tap_only_target_flushes_pending_and_later_tap_does_not_pair() {
    let mut rt = runtime_with(TWO_TARGETS_OP);
    let a = node_center(&rt, "a");
    let b = node_center(&rt, "b");

    // First Tap on `a` (DoubleTap-owning chain): buffered, no delivery.
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, a, 0));
    assert!(rt
        .dispatch_pointer(mouse(1, PointerPhase::Up, a, 50))
        .is_empty());

    // Completed Tap on Tap-only `b`: `a`'s pending flushes NOW and `b`'s
    // Tap delivers immediately — chronological order [a, b].
    let _ = rt.dispatch_pointer(mouse(2, PointerPhase::Down, b, 150));
    let up_b = rt.dispatch_pointer(mouse(2, PointerPhase::Up, b, 200));
    assert_eq!(names(&up_b), ["onTap", "onTap"], "got {:?}", names(&up_b));
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(2));
    assert_eq!(rt.state.app_get("doubles").unwrap().as_i64(), Some(0));
    // Nothing remains buffered.
    assert!(rt.tick(10_000).is_empty());

    // A later Tap on `a` must NOT pair with the stale `a` buffer.
    let _ = rt.dispatch_pointer(mouse(3, PointerPhase::Down, a, 300));
    let later_a = rt.dispatch_pointer(mouse(3, PointerPhase::Up, a, 350));
    assert!(
        later_a.is_empty(),
        "fresh double-tap window on a, got {later_a:?}"
    );
    let flush_a = rt.tick(650);
    assert_eq!(names(&flush_a), ["onTap"]);
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(3));
    assert_eq!(rt.state.app_get("doubles").unwrap().as_i64(), Some(0));
}

/// Exact-deadline behavior is order-independent: whether the host calls
/// `tick(deadline)` first or dispatches the second input at the deadline
/// first, the deferred Tap surfaces once as a single Tap and the second
/// input never pairs into a DoubleTap.
#[test]
fn exact_deadline_is_order_independent_and_never_double_taps() {
    // Path A: host ticks at the deadline BEFORE the second input arrives.
    let mut rt_a = runtime_with(DOUBLE_TAP_OP);
    let c = node_center(&rt_a, "btn");
    let _ = rt_a.dispatch_pointer(mouse(1, PointerPhase::Down, c, 1_000));
    assert!(rt_a
        .dispatch_pointer(mouse(1, PointerPhase::Up, c, 1_050))
        .is_empty());
    let tick_first = rt_a.tick(1_350);
    assert_eq!(names(&tick_first), ["onTap"], "deadline flush");
    // Second input AT the deadline, after the tick.
    let _ = rt_a.dispatch_pointer(mouse(2, PointerPhase::Down, c, 1_350));
    let up = rt_a.dispatch_pointer(mouse(2, PointerPhase::Up, c, 1_400));
    assert!(
        up.is_empty(),
        "second tap re-opens a fresh window, got {up:?}"
    );
    let flush2 = rt_a.tick(1_700);
    assert_eq!(names(&flush2), ["onTap"]);
    assert_eq!(rt_a.state.app_get("taps").unwrap().as_i64(), Some(2));
    assert_eq!(rt_a.state.app_get("doubles").unwrap().as_i64(), Some(0));

    // Path B: the second input (Down at the deadline) arrives FIRST.
    let mut rt_b = runtime_with(DOUBLE_TAP_OP);
    let c = node_center(&rt_b, "btn");
    let _ = rt_b.dispatch_pointer(mouse(1, PointerPhase::Down, c, 1_000));
    assert!(rt_b
        .dispatch_pointer(mouse(1, PointerPhase::Up, c, 1_050))
        .is_empty());
    // At the Down dispatch (t == deadline) the due pending flushes BEFORE
    // the new input is processed.
    let due_down = rt_b.dispatch_pointer(mouse(2, PointerPhase::Down, c, 1_350));
    assert_eq!(names(&due_down), ["onTap"], "due flush precedes the input");
    let up = rt_b.dispatch_pointer(mouse(2, PointerPhase::Up, c, 1_400));
    assert!(up.is_empty(), "fresh window, got {up:?}");
    let flush2 = rt_b.tick(1_700);
    assert_eq!(names(&flush2), ["onTap"]);
    assert_eq!(rt_b.state.app_get("taps").unwrap().as_i64(), Some(2));
    assert_eq!(rt_b.state.app_get("doubles").unwrap().as_i64(), Some(0));

    // Path C: the second tap's Up lands exactly at the deadline; the due
    // flush at the beginning of that dispatch wins over pairing.
    let mut rt_c = runtime_with(DOUBLE_TAP_OP);
    let c = node_center(&rt_c, "btn");
    let _ = rt_c.dispatch_pointer(mouse(1, PointerPhase::Down, c, 1_000));
    assert!(rt_c
        .dispatch_pointer(mouse(1, PointerPhase::Up, c, 1_050))
        .is_empty());
    let _ = rt_c.dispatch_pointer(mouse(2, PointerPhase::Down, c, 1_300));
    let up_exact = rt_c.dispatch_pointer(mouse(2, PointerPhase::Up, c, 1_350));
    assert_eq!(
        names(&up_exact),
        ["onTap"],
        "at the exact deadline the pending flushes, never a DoubleTap"
    );
    let flush2 = rt_c.tick(1_650);
    assert_eq!(names(&flush2), ["onTap"]);
    assert_eq!(rt_c.state.app_get("taps").unwrap().as_i64(), Some(2));
    assert_eq!(rt_c.state.app_get("doubles").unwrap().as_i64(), Some(0));
}

// ---------------------------------------------------------------------
// 2. Dynamic gestures.disabled participates before arbitration/delivery
// ---------------------------------------------------------------------

/// Two children with dynamically-disabled `onDoubleTap` under a parent
/// that owns `onDoubleTap`: the two taps must pair at the parent.
const DISABLED_CHILD_DOUBLE_TAPS_OP: &str = r##"{"version":"0.8.0",
  "state":{"off":{"type":"bool","default":true},"doubles":{"type":"int","default":0},
           "a_doubles":{"type":"int","default":0},"b_doubles":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","width":400,"height":200,
    "gestures":{"doubleTapSlop":250,"doubleTapTimeout":300},
    "events":{"onDoubleTap":[{"set":{"$app.doubles":"$app.doubles + 1"}}]},
    "children":[
      {"type":"rectangle","id":"a","x":0,"y":0,"width":100,"height":100,
       "gestures":{"disabled":"$app.off"},
       "events":{"onDoubleTap":[{"set":{"$app.a_doubles":"$app.a_doubles + 1"}}]}},
      {"type":"rectangle","id":"b","x":200,"y":0,"width":100,"height":100,
       "gestures":{"disabled":"$app.off"},
       "events":{"onDoubleTap":[{"set":{"$app.b_doubles":"$app.b_doubles + 1"}}]}}]}]}"##;

/// A dynamically-disabled child `onDoubleTap` ABOVE a parent `onTap` must
/// not delay or swallow the parent's two Taps.
const DISABLED_CHILD_DOUBLE_TAP_OVER_TAP_OP: &str = r##"{"version":"0.8.0",
  "state":{"off":{"type":"bool","default":true},"taps":{"type":"int","default":0},
           "doubles":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","width":400,"height":200,
    "events":{"onTap":[{"set":{"$app.taps":"$app.taps + 1"}}]},
    "children":[{"type":"rectangle","id":"child","x":10,"y":10,"width":100,"height":100,
      "gestures":{"disabled":"$app.off"},
      "events":{"onDoubleTap":[{"set":{"$app.doubles":"$app.doubles + 1"}}]}}]}]}"##;

/// A dynamically-disabled Switch must not perform built-in Tap activation.
const DISABLED_SWITCH_OP: &str = r##"{"version":"0.8.0",
  "state":{"off":{"type":"bool","default":false},"on":{"type":"bool","default":false},
           "taps":{"type":"int","default":0}},
  "children":[{"type":"switch","id":"sw","x":10,"y":10,"width":44,"height":24,
    "bindings":{"bind:value":"$state.on"},
    "gestures":{"disabled":"$app.off"},
    "events":{"onTap":[{"set":{"$app.taps":"$app.taps + 1"}}]}}]}"##;

/// Hit a child, the PARENT handler writes `$self` from `$event.local`:
/// the Tap semantic targets the HIT child while `$self` scope and `local`
/// resolve against the parent owner.
const OWNER_SCOPE_OP: &str = r##"{"version":"0.8.0","state":{},"children":[
  {"type":"frame","id":"root","x":100,"y":100,"width":400,"height":400,
   "events":{"onTap":[{"set":{"$self.localX":"$event.local.x"}}]},
   "children":[{"type":"rectangle","id":"child","x":10,"y":10,"width":100,"height":100}]}]}"##;

/// Malformed/non-bool `gestures.disabled` stays fail-open.
const MALFORMED_DISABLED_OP: &str = r##"{"version":"0.8.0",
  "state":{"off":{"type":"string","default":"yes"}},
  "children":[{"type":"frame","id":"root","width":400,"height":200,
    "events":{"onTap":[{"set":{"$app.parent":"$app.parent + 1"}}]},
    "children":[{"type":"rectangle","id":"child","x":10,"y":10,"width":100,"height":100,
      "gestures":{"disabled":"$app.off"},
      "events":{"onTap":[{"set":{"$app.child":"$app.child + 1"}}]}}]}]}"##;

#[test]
fn disabled_child_double_taps_pair_at_enabled_parent_owner() {
    let mut rt = runtime_with(DISABLED_CHILD_DOUBLE_TAPS_OP);
    let a = node_center(&rt, "a");
    let b = node_center(&rt, "b");

    // Both children are dynamically disabled: their onDoubleTap is
    // threaded around; the FIRST tap defers against the parent owner.
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, a, 0));
    assert!(rt
        .dispatch_pointer(mouse(1, PointerPhase::Up, a, 50))
        .is_empty());
    let _ = rt.dispatch_pointer(mouse(2, PointerPhase::Down, b, 150));
    let up_b = rt.dispatch_pointer(mouse(2, PointerPhase::Up, b, 200));
    assert_eq!(names(&up_b), ["onDoubleTap"], "got {:?}", names(&up_b));
    assert_eq!(rt.state.app_get("doubles").unwrap().as_i64(), Some(1));
    // The disabled children's handlers never ran.
    assert_eq!(rt.state.app_get("a_doubles").unwrap().as_i64(), Some(0));
    assert_eq!(rt.state.app_get("b_doubles").unwrap().as_i64(), Some(0));
}

#[test]
fn disabled_child_double_tap_does_not_delay_or_swallow_parent_taps() {
    let mut rt = runtime_with(DISABLED_CHILD_DOUBLE_TAP_OVER_TAP_OP);
    let c = node_center(&rt, "child");

    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    let up1 = rt.dispatch_pointer(mouse(1, PointerPhase::Up, c, 50));
    assert_eq!(
        names(&up1),
        ["onTap"],
        "first Tap must be immediate — no enabled onDoubleTap on the chain"
    );
    let _ = rt.dispatch_pointer(mouse(2, PointerPhase::Down, c, 150));
    let up2 = rt.dispatch_pointer(mouse(2, PointerPhase::Up, c, 200));
    assert_eq!(names(&up2), ["onTap"], "second Tap immediate, got {up2:?}");
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(2));
    assert_eq!(rt.state.app_get("doubles").unwrap().as_i64(), Some(0));
    assert!(rt.tick(10_000).is_empty(), "nothing was buffered");
}

#[test]
fn dynamically_disabled_target_widget_skips_builtin_activation() {
    let mut rt = runtime_with(DISABLED_SWITCH_OP);
    let c = node_center(&rt, "sw");

    // Disabled: the Tap semantic reaches delivery but the built-in
    // activation and the authored handler must not run.
    rt.state.app_set("off", serde_json::json!(true));
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    let up = rt.dispatch_pointer(mouse(1, PointerPhase::Up, c, 50));
    assert_eq!(names(&up), ["onTap"], "Tap is still emitted for the host");
    assert_eq!(
        rt.state.app_get("on").and_then(|v| v.as_bool()),
        Some(false),
        "disabled target must not toggle"
    );
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(0));

    // Re-enabled: the same tap toggles the switch and runs the handler.
    rt.state.app_set("off", serde_json::json!(false));
    let _ = rt.dispatch_pointer(mouse(2, PointerPhase::Down, c, 200));
    let up = rt.dispatch_pointer(mouse(2, PointerPhase::Up, c, 250));
    assert_eq!(names(&up), ["onTap"]);
    assert_eq!(
        rt.state.app_get("on").and_then(|v| v.as_bool()),
        Some(true),
        "enabled target toggles"
    );
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(1));
}

#[test]
fn parent_handler_scope_and_local_coordinate_use_the_owner() {
    let mut rt = runtime_with(OWNER_SCOPE_OP);
    let child_key = rt.document.as_ref().unwrap().tree.get("child").unwrap();
    // Child is hit (its rect: x 110..210, y 110..210 — the child's 10/10
    // offsets are relative to the parent rect origin at (100,100)); the
    // parent owns onTap. Global hit point (160,160); the parent rect
    // origin is (100,100) → local (60,60).
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, point(160.0, 160.0), 0));
    let up = rt.dispatch_pointer(mouse(1, PointerPhase::Up, point(160.0, 160.0), 50));
    assert_eq!(names(&up), ["onTap"]);
    assert_eq!(
        up[0].node(),
        child_key,
        "the Tap semantic targets the HIT child; the handler owner (bubbling target) is the parent"
    );

    // The $self write landed on the PARENT's node scope...
    let parent_x = rt
        .state
        .self_get("", "root", "localX")
        .expect("parent $self write")
        .as_f64();
    assert_eq!(parent_x, Some(60.0), "local is parent-relative");
    // ...and NOT on the hit child's scope.
    assert!(
        rt.state.self_get("", "child", "localX").is_none(),
        "$self must scope to the resolved handler owner, not the hit child"
    );
}

#[test]
fn malformed_non_bool_disabled_fails_open() {
    let mut rt = runtime_with(MALFORMED_DISABLED_OP);
    let c = node_center(&rt, "child");

    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    let up = rt.dispatch_pointer(mouse(1, PointerPhase::Up, c, 50));
    assert_eq!(names(&up), ["onTap"]);
    // "yes" is a string: the expression is not a bool → disabled is false
    // (fail-open) → the child's own handler runs.
    assert_eq!(rt.state.app_get("child").unwrap().as_i64(), Some(1));
}

// ---------------------------------------------------------------------
// 3. ContextMenu facts + slider side effects
// ---------------------------------------------------------------------

const SLIDER_CONTEXT_OP: &str = r##"{"version":"0.8.0",
  "state":{"vol":{"type":"float","default":0},"menu":{"type":"int","default":0}},
  "children":[{"type":"slider","id":"sl","x":10,"y":30,"width":200,"height":20,
    "min":0,"max":100,"step":1,"bindings":{"bind:value":"$state.vol"},
    "events":{"onContextMenu":[{"set":{"$app.menu":"$app.menu + 1"}}],
              "onTap":[{"set":{"$app.menu":"$app.menu + 100"}}]}}]}"##;

const SLIDER_NO_CONTEXT_OP: &str = r##"{"version":"0.8.0",
  "state":{"vol":{"type":"float","default":0}},
  "children":[{"type":"slider","id":"sl","x":10,"y":30,"width":200,"height":20,
    "min":0,"max":100,"step":1,"bindings":{"bind:value":"$state.vol"},
    "events":{"onTap":[{"set":{"$state.vol":"$state.vol + 1"}}]}}]}"##;

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

/// A factual right-button Down over a slider must never focus/arm/mutate
/// it before Router returns ContextMenu, and no later Tap/drag side
/// effects may be synthesized by the right-only sequence.
#[test]
fn right_click_slider_emits_only_context_menu_and_never_mutates() {
    let mut rt = runtime_with(SLIDER_CONTEXT_OP);
    // Hit mid-track: x = 10 + 100 = 110 → would be 50% if it scrubbed.
    let at = point(110.0, 40.0);

    let mut down = mouse(1, PointerPhase::Down, at, 0);
    down.buttons = MouseButtons::RIGHT;
    let down_ev = rt.dispatch_pointer(down);
    assert_eq!(names(&down_ev), ["onContextMenu"], "got {down_ev:?}");
    let mut move_ev = mouse(1, PointerPhase::Move, point(210.0, 40.0), 10);
    move_ev.buttons = MouseButtons::RIGHT;
    assert!(rt.dispatch_pointer(move_ev).is_empty(), "closed sequence");
    let mut up = mouse(1, PointerPhase::Up, point(210.0, 40.0), 20);
    up.buttons = MouseButtons::empty();
    assert!(
        rt.dispatch_pointer(up).is_empty(),
        "no Tap from right press"
    );

    assert_eq!(rt.state.app_get("vol").and_then(|v| v.as_f64()), Some(0.0));
    assert_eq!(
        slider_state(&rt),
        (0.0, false),
        "never armed, never mutated"
    );
    assert_eq!(rt.state.app_get("menu").unwrap().as_i64(), Some(1));
}

/// Right-only pointer sequence over a slider WITHOUT any context-menu
/// handler is swallowed: no drag, no Tap, no value change.
#[test]
fn right_only_sequence_on_slider_is_swallowed_without_side_effects() {
    let mut rt = runtime_with(SLIDER_NO_CONTEXT_OP);
    let at = point(110.0, 40.0);

    let mut down = mouse(1, PointerPhase::Down, at, 0);
    down.buttons = MouseButtons::RIGHT;
    assert!(
        rt.dispatch_pointer(down).is_empty(),
        "right-only Down must not produce Tap/Press semantic"
    );
    let mut move_ev = mouse(1, PointerPhase::Move, point(210.0, 40.0), 10);
    move_ev.buttons = MouseButtons::RIGHT;
    assert!(rt.dispatch_pointer(move_ev).is_empty());
    let mut up = mouse(1, PointerPhase::Up, point(210.0, 40.0), 20);
    up.buttons = MouseButtons::empty();
    assert!(rt.dispatch_pointer(up).is_empty());

    assert_eq!(rt.state.app_get("vol").and_then(|v| v.as_f64()), Some(0.0));
    assert_eq!(slider_state(&rt), (0.0, false));
}

/// PointerPhase::Cancel must disarm Slider.dragging exactly like Up; a
/// later Move must not scrub the canceled pointer's drag.
#[test]
fn cancel_disarms_slider_drag_and_later_move_does_not_scrub() {
    let mut rt = runtime_with(SLIDER_NO_CONTEXT_OP);
    let left = point(12.0, 40.0);
    let far_right = point(260.0, 40.0);

    // Touch Down arms the drag (provable primary contact).
    let _ = rt.dispatch_pointer(touch(1, PointerPhase::Down, left, 0));
    assert_eq!(slider_state(&rt).1, true, "armed");
    // Move scrubs to max.
    let _ = rt.dispatch_pointer(touch(1, PointerPhase::Move, far_right, 10));
    assert_eq!(
        rt.state.app_get("vol").and_then(|v| v.as_f64()),
        Some(100.0)
    );
    assert_eq!(slider_state(&rt).0, 100.0);

    // Cancel disarms exactly like Up.
    let _ = rt.dispatch_pointer(touch(1, PointerPhase::Cancel, far_right, 20));
    assert_eq!(slider_state(&rt).1, false, "Cancel must disarm");

    // A later Move scrubs nothing (no armed slider).
    let _ = rt.dispatch_pointer(touch(2, PointerPhase::Move, point(20.0, 40.0), 30));
    assert_eq!(
        rt.state.app_get("vol").and_then(|v| v.as_f64()),
        Some(100.0)
    );
    assert_eq!(slider_state(&rt).0, 100.0);
}

// ---------------------------------------------------------------------
// 4. PressCancel facts during multi-pointer claim
// ---------------------------------------------------------------------

const PRESS_SCALE_COUNT_OP: &str = r##"{"version":"0.8.0",
  "state":{"pressed":{"type":"int","default":0},"cancelled":{"type":"int","default":0},
           "scales":{"type":"int","default":0}},
  "children":[{"type":"rectangle","id":"canvas","width":800,"height":600,
    "events":{"onPressStart":[{"set":{"$app.pressed":"$app.pressed + 1"}}],
              "onPressCancel":[{"set":{"$app.cancelled":"$app.cancelled + 1"}}],
              "onScaleStart":[{"set":{"$app.scales":"$app.scales + 1"}}]}}]}"##;

/// The triggering pointer's PressCancel must carry the current Move
/// phase/position/timestamp/pointer id; the stationary participant uses
/// its latest factual event; both cancel actions EXECUTE (count, not an
/// idempotent set).
#[test]
fn multi_claim_press_cancels_carry_current_move_facts_and_both_fire() {
    let mut rt = runtime_with(PRESS_SCALE_COUNT_OP);
    let c = node_center(&rt, "canvas");

    let _ = rt.dispatch_pointer_events(mouse(0, PointerPhase::Down, point(c.x - 100.0, c.y), 0));
    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, point(c.x + 100.0, c.y), 10));
    let pinch =
        rt.dispatch_pointer_events(mouse(0, PointerPhase::Move, point(c.x - 200.0, c.y), 20));
    assert_eq!(
        envelope_names(&pinch),
        ["onPressCancel", "onPressCancel", "onScaleStart"],
        "got {:?}",
        envelope_names(&pinch)
    );

    // Inspect the factual envelopes (not just the semantics).
    let cancels: Vec<&SemanticEventEnvelope> = pinch
        .iter()
        .filter(|e| matches!(e.event, SemanticEvent::PressCancel { .. }))
        .collect();
    assert_eq!(cancels.len(), 2, "one cancellation per pressed pointer");

    let (id0, id1) = (
        cancels[0].pointer_facts.as_ref().map(|f| f.id),
        cancels[1].pointer_facts.as_ref().map(|f| f.id),
    );
    assert_eq!(id0, Some(PointerId(0)));
    assert_eq!(id1, Some(PointerId(1)));
    assert_ne!(id0, id1, "pointer ids must be distinct");

    // The moving pointer (0) carries the CURRENT Move facts...
    let m0 = cancels[0].pointer_facts.as_ref().expect("facts");
    assert_eq!(m0.phase, PointerPhase::Move);
    assert_eq!(m0.position, point(c.x - 200.0, c.y));
    assert_eq!(m0.t_ms, 20);
    // ...and the stationary pointer (1) keeps its latest factual event.
    let m1 = cancels[1].pointer_facts.as_ref().expect("facts");
    assert_eq!(m1.id, PointerId(1));
    assert_eq!(m1.phase, PointerPhase::Down);
    assert_eq!(m1.position, point(c.x + 100.0, c.y));
    assert_eq!(m1.t_ms, 10);

    // Both cancel actions executed: a count (not set=1) proves it.
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(2));
    assert_eq!(rt.state.app_get("pressed").unwrap().as_i64(), Some(2));
    assert_eq!(rt.state.app_get("scales").unwrap().as_i64(), Some(1));
}

// ---------------------------------------------------------------------
// 5. Handler/owner-aware thresholds
// ---------------------------------------------------------------------

/// The parent owns Pan/LongPress (authored thresholds); the hit child
/// declares no handlers. A child recognizer with default thresholds must
/// not shadow the ancestor's authored configuration.
const ANCESTOR_THRESHOLDS_OP: &str = r##"{"version":"0.8.0",
  "state":{"pans":{"type":"int","default":0},"longs":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","x":0,"y":0,"width":400,"height":400,
    "gestures":{"dragThreshold":30,"longPressDuration":100},
    "events":{"onPanStart":[{"set":{"$app.pans":"$app.pans + 1"}}],
              "onLongPress":[{"set":{"$app.longs":"$app.longs + 1"}}]},
    "children":[{"type":"rectangle","id":"child","x":10,"y":10,"width":100,"height":100}]}]}"##;

#[test]
fn ancestor_authored_pan_threshold_and_long_press_win_over_hit_child() {
    let mut rt = runtime_with(ANCESTOR_THRESHOLDS_OP);
    let c = node_center(&rt, "child");
    let root_key = rt.document.as_ref().unwrap().tree.get("root").unwrap();

    // 15px move: under the parent's authored 30px threshold → no Pan claim.
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    let small = rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(c.x + 15.0, c.y), 100));
    assert!(small.is_empty(), "got {small:?}");
    // 35px: crosses the authored threshold; semantic targets the OWNER.
    let big = rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(c.x + 35.0, c.y), 200));
    assert_eq!(names(&big), ["onPanStart"], "got {big:?}");
    assert_eq!(
        big[0].node(),
        root_key,
        "Pan semantic target is the handler owner, not the hit child"
    );
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Up, point(c.x + 35.0, c.y), 300));
    assert_eq!(rt.state.app_get("pans").unwrap().as_i64(), Some(1));

    // Authored longPressDuration 100 on the owner, hit child again.
    let _ = rt.dispatch_pointer(mouse(2, PointerPhase::Down, c, 400));
    let early = rt.tick(480);
    assert!(early.is_empty(), "80ms < authored 100ms");
    let fires = rt.tick(520);
    assert_eq!(names(&fires), ["onLongPress"], "got {fires:?}");
    assert_eq!(rt.state.app_get("longs").unwrap().as_i64(), Some(1));
}

/// Empty ActionLists do not count as enabled handlers: a child with
/// `onPanStart: []` must not become the owner (or shadow the parent's
/// authored threshold).
const EMPTY_CHILD_PAN_OP: &str = r##"{"version":"0.8.0",
  "state":{"pans":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","width":400,"height":400,
    "gestures":{"dragThreshold":30},
    "events":{"onPanStart":[{"set":{"$app.pans":"$app.pans + 1"}}]},
    "children":[{"type":"rectangle","id":"child","x":10,"y":10,"width":100,"height":100,
      "events":{"onPanStart":[]}}]}]}"##;

#[test]
fn empty_action_lists_do_not_count_as_handlers() {
    let mut rt = runtime_with(EMPTY_CHILD_PAN_OP);
    let c = node_center(&rt, "child");

    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    // 15px under the parent's 30px threshold → no claim; the child's
    // empty declaration must not have installed a default-8px recognizer.
    let small = rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(c.x + 15.0, c.y), 100));
    assert!(small.is_empty(), "got {small:?}");
    let big = rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(c.x + 35.0, c.y), 200));
    assert_eq!(names(&big), ["onPanStart"]);
    assert_eq!(rt.state.app_get("pans").unwrap().as_i64(), Some(1));
}

/// Scale/Rotate handler detection ignores empty lists and disabledEvents
/// declarations (arbitration itself is untouched).
const DISABLED_SCALE_OP: &str = r##"{"version":"0.7.0",
  "state":{"scales":{"type":"int","default":0}},
  "children":[{"type":"rectangle","id":"canvas","width":800,"height":600,
    "gestures":{"disabledEvents":["onScaleStart","onScaleUpdate","onScaleEnd"]},
    "events":{"onScaleStart":[{"set":{"$app.scales":"$app.scales + 1"}}]}}]}"##;

const EMPTY_SCALE_OP: &str = r##"{"version":"0.7.0",
  "state":{"scales":{"type":"int","default":0}},
  "children":[{"type":"rectangle","id":"canvas","width":800,"height":600,
    "events":{"onScaleStart":[]}}]}"##;

#[test]
fn scale_handler_detection_ignores_disabled_events_and_empty_lists() {
    // disabledEvents declarations + empty lists: no multi recognizer is
    // registered → a pinch never emits ScaleStart.
    let (mut rt, mut second) = (
        runtime_with(DISABLED_SCALE_OP),
        runtime_with(EMPTY_SCALE_OP),
    );
    for rt in [&mut rt, &mut second] {
        let _ = rt.dispatch_pointer(mouse(0, PointerPhase::Down, point(200.0, 300.0), 0));
        let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, point(400.0, 300.0), 10));
        let pinch = rt.dispatch_pointer(mouse(0, PointerPhase::Move, point(100.0, 300.0), 20));
        assert!(
            !pinch.iter().any(|e| matches!(
                e,
                SemanticEvent::ScaleStart { .. } | SemanticEvent::ScaleUpdate { .. }
            )),
            "disabled/empty scale handlers must not register a multi recognizer, got {pinch:?}"
        );
        assert_eq!(rt.state.app_get("scales").unwrap().as_i64(), Some(0));
    }
}

// ---------------------------------------------------------------------
// 6. Public API bridge — plain emit + legacy reject compose
// ---------------------------------------------------------------------

/// Claims on the first Down and emits its Tap via the one-argument
/// (plain-envelope) `ArenaHandle::emit`.
struct ClaimEarlyProber {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
}

impl Recognizer for ClaimEarlyProber {
    fn id(&self) -> RecognizerId {
        self.id
    }
    fn kind(&self) -> &'static str {
        "Prober"
    }
    fn node(&self) -> NodeKey {
        self.node
    }
    fn state(&self) -> RecognizerState {
        self.state
    }
    fn handle_pointer(
        &mut self,
        event: &PointerEvent,
        _arena: &mut ArenaHandle<'_>,
    ) -> RecognizerState {
        if matches!(event.phase, PointerPhase::Down) && self.state != RecognizerState::Rejected {
            self.state = RecognizerState::Claimed;
        }
        self.state
    }
    fn accept(&mut self, arena: &mut ArenaHandle<'_>) {
        self.state = RecognizerState::Claimed;
        // Legacy one-argument emit: creates a plain envelope (no facts).
        arena.emit(SemanticEvent::Tap {
            node: self.node,
            position: jian_core::geometry::point(0.0, 0.0),
        });
    }
    /// Legacy rejection hook only — the arena's handle-aware default
    /// delegates to this. No `reject_with_handle` override by design.
    fn reject(&mut self) {
        self.state = RecognizerState::Rejected;
    }
}

impl ClaimEarlyProber {
    fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
        }
    }
}

const BTN_DOC: &str = r##"{"version":"0.8.0","children":[{"type":"rectangle","id":"btn","width":100,"height":100}]}"##;

/// A custom recognizer (implementing only the legacy `reject`) composes
/// with the arena via the default handle-aware bridge; the one-argument
/// plain `emit` carries no facts; the factual builders stay available.
#[test]
fn custom_recognizer_with_plain_emit_and_legacy_reject_composes() {
    let rt = runtime_with(BTN_DOC);
    let doc = rt.document.as_ref().unwrap();
    let node = doc.tree.get("btn").unwrap();
    let mut arena = Arena::new(vec![
        Box::new(ClaimEarlyProber::new(10, node)),
        Box::new(ClaimEarlyProber::new(11, node)),
    ]);

    // The first prober claims on Down; the arena resolves and rejects the
    // second via the default `reject_with_handle` → `reject`.
    arena.dispatch(
        &PointerEvent::simple(1, PointerPhase::Down, point(5.0, 5.0)),
        doc,
    );
    let envelopes = arena.drain_envelopes();
    assert_eq!(envelopes.len(), 1);
    assert!(
        envelopes[0].pointer_facts.is_none(),
        "one-argument emit creates a plain envelope (facts absent)"
    );
    assert!(matches!(envelopes[0].event, SemanticEvent::Tap { .. }));

    // The loser's legacy reject ran: its state is Rejected.
    let mut rejected = false;
    for r in arena.members_mut() {
        if r.id() == 11 && matches!(r.state(), RecognizerState::Rejected) {
            rejected = true;
        }
    }
    assert!(rejected, "legacy reject must be called via the bridge");

    // The factual builders remain callable from a custom recognizer.
    let mut pending: Option<SemanticEventEnvelope> = None;
    let mut handle = ArenaHandle {
        pending_semantic: &mut pending,
    };
    let ev = PointerEvent::simple(1, PointerPhase::Down, point(3.0, 4.0));
    handle.emit_with_facts(
        SemanticEvent::PressStart {
            node,
            position: ev.position,
        },
        PointerFacts::from_event(&ev),
    );
    let env = pending.take().expect("built");
    assert!(matches!(env.event, SemanticEvent::PressStart { .. }));
    assert_eq!(
        env.pointer_facts.map(|f| f.id),
        Some(PointerId(1)),
        "emit_with_facts attaches the factual pointer metadata"
    );
}
