//! R2A repair-round regressions (round 2): true due-Tap ordering + hover.
//!
//! The runtime must deliver a due deferred (double-tap-window) Tap's
//! actions BEFORE any current event processing — slider side effects,
//! `gestures.disabled` predicate evaluation, hover semantics and arena
//! routing — so the due action observes pre-current state and the current
//! arbitration observes the post-due state, identically in both the
//! tick-first and the event-first interleavings. The internal
//! current-event path must also prevent the router from flushing the same
//! pending Tap twice, and the public `PointerRouter` entry points must
//! still flush due before Hover/current semantics for static users.

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

// ---------------------------------------------------------------------
// 1. Exact-deadline Slider Down: due action observes pre-slider state and
//    the final state/order is identical in both interleavings.
// ---------------------------------------------------------------------

/// `src` owns Tap + DoubleTap (its first Tap buffers; the flushed onTap
/// snapshot reads the slider's bound value). `sl` is a slider bound to
/// `$state.vol`; its Down at the deadline arms a drag and scrubs the
/// value as the CURRENT event side effect.
const SLIDER_DUE_OP: &str = r##"{"version":"0.8.0",
  "state":{"vol":{"type":"float","default":0},"seen":{"type":"float","default":-1},
           "taps":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","width":600,"height":400,"children":[
    {"type":"rectangle","id":"src","x":0,"y":0,"width":100,"height":100,
     "events":{"onTap":[{"set":{"$app.seen":"$state.vol"}}],
               "onDoubleTap":[{"set":{"$app.taps":"$app.taps + 100"}}]}},
    {"type":"slider","id":"sl","x":300,"y":30,"width":200,"height":20,
     "min":0,"max":100,"step":1,"bindings":{"bind:value":"$state.vol"},
     "events":{"onTap":[{"set":{"$app.taps":"$app.taps + 1"}}]}}]}]}"##;

fn buffered_src_tap(rt: &mut Runtime, src: Point) {
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, src, 1_000));
    assert!(rt
        .dispatch_pointer(mouse(1, PointerPhase::Up, src, 1_050))
        .is_empty());
}

#[test]
fn exact_deadline_slider_down_due_action_observes_pre_slider_state() {
    // Path A: the host ticks at the deadline BEFORE the slider Down.
    let mut rt_a = runtime_with(SLIDER_DUE_OP);
    let src = node_center(&rt_a, "src");
    let at_slider = point(400.0, 40.0); // slider track midpoint -> 50.0
    buffered_src_tap(&mut rt_a, src);
    // deadline = 1050 + 300 = 1350.
    let tick_first = rt_a.tick(1_350);
    assert_eq!(names(&tick_first), ["onTap"], "deadline flush");
    // The due action already observed the PRE-scrub value.
    assert_eq!(
        rt_a.state.app_get("seen").and_then(|v| v.as_f64()),
        Some(0.0)
    );
    // The current Slider Down at the deadline scrubs only after the tick.
    let down = rt_a.dispatch_pointer(mouse(2, PointerPhase::Down, at_slider, 1_350));
    assert!(
        down.is_empty(),
        "current Down has no semantic, got {down:?}"
    );
    assert_eq!(
        rt_a.state.app_get("vol").and_then(|v| v.as_f64()),
        Some(50.0)
    );
    let up = rt_a.dispatch_pointer(mouse(2, PointerPhase::Up, at_slider, 1_400));
    assert_eq!(names(&up), ["onTap"], "clean Down+Up lands as a Tap");

    // Path B: the slider Down arrives AT the deadline, event-first.
    let mut rt_b = runtime_with(SLIDER_DUE_OP);
    let src = node_center(&rt_b, "src");
    buffered_src_tap(&mut rt_b, src);
    let down = rt_b.dispatch_pointer(mouse(2, PointerPhase::Down, at_slider, 1_350));
    assert_eq!(
        names(&down),
        ["onTap"],
        "due flush must precede the current Slider Down, got {:?}",
        names(&down)
    );
    assert_eq!(
        rt_b.state.app_get("seen").and_then(|v| v.as_f64()),
        Some(0.0),
        "due action must observe the PRE-slider value, not a scrubbed one"
    );
    assert_eq!(
        rt_b.state.app_get("vol").and_then(|v| v.as_f64()),
        Some(50.0)
    );
    let up = rt_b.dispatch_pointer(mouse(2, PointerPhase::Up, at_slider, 1_400));
    assert_eq!(names(&up), ["onTap"]);

    // Final state/order is identical: the due action read 0, the slider
    // settled at 50, the slider's clean tap fired once, and no pairing /
    // source-side effect occurred in either interleaving.
    for (label, rt) in [("tick-first", &mut rt_a), ("event-first", &mut rt_b)] {
        assert_eq!(
            rt.state.app_get("seen").and_then(|v| v.as_f64()),
            Some(0.0),
            "{label}"
        );
        assert_eq!(
            rt.state.app_get("vol").and_then(|v| v.as_f64()),
            Some(50.0),
            "{label}"
        );
        assert_eq!(
            rt.state.app_get("taps").unwrap().as_i64(),
            Some(1),
            "{label}"
        );
        assert!(rt.tick(10_000).is_empty(), "{label}: nothing left buffered");
    }
}

// ---------------------------------------------------------------------
// 2. Due action flips a state flag used by the current target's
//    `gestures.disabled`: the current arbitration observes the updated
//    flag in both interleavings.
// ---------------------------------------------------------------------

/// `src`'s flushed onTap flips `gate` false. `mid` declares an
/// `onDoubleTap` gated by `gate` and a disabled-agnostic `onTap` bubbles
/// to `root`. When the current tap's ARBITRATION sees the post-due flag
/// (gate == false), the tap defers at `mid` and the second tap PAIRS into
/// `onDoubleTap`; when arbitration sees the stale pre-due flag the tap is
/// immediate and the second tap never pairs.
const GATED_DOUBLE_TAP_OP: &str = r##"{"version":"0.8.0",
  "state":{"gate":{"type":"bool","default":true},
           "taps":{"type":"int","default":0},
           "src_d":{"type":"int","default":0},
           "mid_d":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","width":600,"height":400,
    "events":{"onTap":[{"set":{"$app.taps":"$app.taps + 1"}}]},
    "children":[
      {"type":"rectangle","id":"src","x":0,"y":0,"width":100,"height":100,
       "events":{"onTap":[{"set":{"$app.gate":"false"}}],
                 "onDoubleTap":[{"set":{"$app.src_d":"$app.src_d + 1"}}]}},
      {"type":"rectangle","id":"mid","x":300,"y":0,"width":150,"height":150,
       "gestures":{"disabled":"$app.gate"},
       "events":{"onDoubleTap":[{"set":{"$app.mid_d":"$app.mid_d + 1"}}]}}]}]}"##;

fn assert_order_independent_gated_arbitration(rt: &mut Runtime) {
    let src = node_center(rt, "src");
    let mid = node_center(rt, "mid");

    // First Tap on `src`: buffered (src owns onDoubleTap); deadline 350.
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, src, 0));
    assert!(rt
        .dispatch_pointer(mouse(1, PointerPhase::Up, src, 50))
        .is_empty());

    // Second tap: Down before the deadline, Up AT the deadline — the due
    // flush and the current arbitration race here. The dispatch returns
    // exactly the due Tap (delivered first); the current tap must be
    // DEFERRED (post-due gate=false re-enables mid's onDoubleTap), so NO
    // second onTap follows the due one.
    let _ = rt.dispatch_pointer(mouse(2, PointerPhase::Down, mid, 345));
    let up_mid = rt.dispatch_pointer(mouse(2, PointerPhase::Up, mid, 400));
    assert_eq!(
        names(&up_mid),
        ["onTap"],
        "must return exactly the due Tap, with the current tap deferred, got {:?}",
        names(&up_mid)
    );
    // The due action ran before the current tap was classified.
    assert_eq!(
        rt.state.app_get("gate").and_then(|v| v.as_bool()),
        Some(false)
    );

    // Third tap within the fresh window: pairs into a single DoubleTap at
    // the now-enabled `mid` owner.
    let _ = rt.dispatch_pointer(mouse(3, PointerPhase::Down, mid, 430));
    let second_up = rt.dispatch_pointer(mouse(3, PointerPhase::Up, mid, 450));
    assert_eq!(
        names(&second_up),
        ["onDoubleTap"],
        "got {:?}",
        names(&second_up)
    );

    // The deferred current tap never surfaced as a single Tap, the pair
    // fired exactly once at mid, and the src double-tap never fired.
    let final_tick = rt.tick(10_000);
    assert!(
        final_tick.is_empty(),
        "nothing left buffered, got {final_tick:?}"
    );
    assert_eq!(
        rt.state.app_get("gate").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(0));
    assert_eq!(rt.state.app_get("src_d").unwrap().as_i64(), Some(0));
    assert_eq!(rt.state.app_get("mid_d").unwrap().as_i64(), Some(1));
}

#[test]
fn due_action_flag_update_is_seen_by_current_arbitration_in_both_orderings() {
    // Event-first: the due flush happens at the current Up (deadline).
    let mut rt_event_first = runtime_with(GATED_DOUBLE_TAP_OP);
    assert_order_independent_gated_arbitration(&mut rt_event_first);

    // Tick-first: the due flush happens at the deadline BEFORE the current
    // Up arrives — final state must be identical.
    let mut rt_tick_first = runtime_with(GATED_DOUBLE_TAP_OP);
    let src = node_center(&rt_tick_first, "src");
    let mid = node_center(&rt_tick_first, "mid");
    let _ = rt_tick_first.dispatch_pointer(mouse(1, PointerPhase::Down, src, 0));
    assert!(rt_tick_first
        .dispatch_pointer(mouse(1, PointerPhase::Up, src, 50))
        .is_empty());
    assert_eq!(names(&rt_tick_first.tick(350)), ["onTap"]);
    assert_eq!(
        rt_tick_first
            .state
            .app_get("gate")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    let _ = rt_tick_first.dispatch_pointer(mouse(2, PointerPhase::Down, mid, 345));
    assert!(rt_tick_first
        .dispatch_pointer(mouse(2, PointerPhase::Up, mid, 400))
        .is_empty());
    let _ = rt_tick_first.dispatch_pointer(mouse(3, PointerPhase::Down, mid, 430));
    assert_eq!(
        names(&rt_tick_first.dispatch_pointer(mouse(3, PointerPhase::Up, mid, 450))),
        ["onDoubleTap"]
    );
    assert!(rt_tick_first.tick(10_000).is_empty());
    assert_eq!(
        rt_tick_first.state.app_get("taps").unwrap().as_i64(),
        Some(0)
    );
    assert_eq!(
        rt_tick_first.state.app_get("src_d").unwrap().as_i64(),
        Some(0)
    );
    assert_eq!(
        rt_tick_first.state.app_get("mid_d").unwrap().as_i64(),
        Some(1)
    );
}

// ---------------------------------------------------------------------
// 3. Pending Tap + Hover at the deadline: the Tap action executes before
//    the Hover action in BOTH interleavings.
// ---------------------------------------------------------------------

/// Both actions append to one shared `seq` (Tap appends bit 1, Hover
/// appends bit 2), so `seq == 4` proves Tap-then-Hover while `seq == 5`
/// would prove Hover-then-Tap.
const TAP_HOVER_OP: &str = r##"{"version":"0.8.0",
  "state":{"seq":{"type":"int","default":0},"taps":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","width":600,"height":400,"children":[
    {"type":"rectangle","id":"src","x":0,"y":0,"width":100,"height":100,
     "events":{"onTap":[{"set":{"$app.seq":"$app.seq * 2 + 1"}}],
               "onDoubleTap":[{"set":{"$app.taps":"$app.taps + 1"}}]}},
    {"type":"rectangle","id":"dst","x":300,"y":0,"width":150,"height":150,
     "events":{"onHoverEnter":[{"set":{"$app.seq":"$app.seq * 2 + 2"}}]}}]}]}"##;

#[test]
fn pending_tap_flushes_before_hover_at_the_deadline_in_both_orderings() {
    // Path A: tick at the deadline, then the hover.
    let mut rt_a = runtime_with(TAP_HOVER_OP);
    let src = node_center(&rt_a, "src");
    let dst = node_center(&rt_a, "dst");
    let _ = rt_a.dispatch_pointer(mouse(1, PointerPhase::Down, src, 0));
    assert!(rt_a
        .dispatch_pointer(mouse(1, PointerPhase::Up, src, 50))
        .is_empty());
    assert_eq!(names(&rt_a.tick(350)), ["onTap"]);
    let hover = rt_a.dispatch_pointer(mouse(2, PointerPhase::Hover, dst, 350));
    assert_eq!(names(&hover), ["onHoverEnter"]);
    assert_eq!(rt_a.state.app_get("seq").unwrap().as_i64(), Some(4));

    // Path B: the hover arrives AT the deadline, event-first — the due
    // Tap is flushed and delivered before the hover semantics.
    let mut rt_b = runtime_with(TAP_HOVER_OP);
    let src = node_center(&rt_b, "src");
    let dst = node_center(&rt_b, "dst");
    let _ = rt_b.dispatch_pointer(mouse(1, PointerPhase::Down, src, 0));
    assert!(rt_b
        .dispatch_pointer(mouse(1, PointerPhase::Up, src, 50))
        .is_empty());
    let hover = rt_b.dispatch_pointer(mouse(2, PointerPhase::Hover, dst, 350));
    assert_eq!(
        names(&hover),
        ["onTap", "onHoverEnter"],
        "due Tap envelope must precede the current Hover envelope, got {:?}",
        names(&hover)
    );
    assert_eq!(
        rt_b.state.app_get("seq").unwrap().as_i64(),
        Some(4),
        "Tap action must run before the Hover action"
    );
    assert_eq!(rt_b.state.app_get("taps").unwrap().as_i64(), Some(0));
}

/// The public (static-users) router entry point must also flush a due
/// pending Tap before Hover semantics — a static consumer that feeds
/// Hover straight into `PointerRouter::dispatch_enveloped` must see the
/// Tap before the Enter.
#[test]
fn public_router_flushes_due_tap_before_hover() {
    let mut rt = runtime_with(TAP_HOVER_OP);
    let src = node_center(&rt, "src");
    let dst = node_center(&rt, "dst");
    let doc = rt.document.as_ref().unwrap();
    let spatial = &rt.spatial;
    let router = &mut rt.gestures;

    // Buffer the first Tap through the public path (static predicate).
    assert!(router
        .dispatch_enveloped(mouse(1, PointerPhase::Down, src, 0), doc, spatial)
        .is_empty());
    let tap = router.dispatch_enveloped(mouse(1, PointerPhase::Up, src, 50), doc, spatial);
    assert!(tap.is_empty(), "first Tap is buffered, got {tap:?}");

    // The hover at the deadline: due Tap first, then HoverEnter.
    let due_hover =
        router.dispatch_enveloped(mouse(2, PointerPhase::Hover, dst, 350), doc, spatial);
    assert_eq!(
        due_hover
            .iter()
            .map(|e| e.event.handler_key())
            .collect::<Vec<_>>(),
        ["onTap", "onHoverEnter"],
        "public dispatch must flush due before hover, got {due_hover:?}"
    );
    // Nothing was flushed twice: a follow-up hover at the same target
    // emits nothing (a Changed target would emit Leave/Enter) and the
    // later tick re-emits nothing.
    assert!(router
        .dispatch_enveloped(mouse(2, PointerPhase::Hover, dst, 400), doc, spatial)
        .is_empty());
    assert!(router.tick_enveloped(10_000).is_empty());
}
