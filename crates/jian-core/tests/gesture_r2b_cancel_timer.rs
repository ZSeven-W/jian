//! R2B repair-round regressions: a late host-sent Cancel must not claim
//! arena timers.
//!
//! Timer-before-current (R2B) drives EVERY gesture deadline `<= t` on
//! incoming pointer input, so Down at t0 followed by Cancel at t600 used
//! to tick the arena's LongPress / touch ContextMenu timer at the cancel
//! and yield `[PressCancel, LongPress]` (or `ContextMenu`). A system
//! cancel is the TERMINAL authority for its pointer: the contract now
//! dispatches the current Cancel immediately, so ownership ends with
//! PressCancel exactly once and no timer claim off the CANCELED pointer
//! — in BOTH the runtime pointer path and the public `PointerRouter`
//! entry points. Every other phase keeps timer-before-current.
//!
//! The isolation is PER-POINTER, not global: a Cancel ticks every OTHER
//! active arena (an overdue LongPress on a different pointer still fires
//! before the cancel), skips only the canceling pointer's OWN arena, and
//! a due pending Tap from an unrelated completed pointer still flushes.
//! The two-pointer tests below assert the exact stream and exactly-once
//! handler counts for BOTH directions: the other pointer's timer fires,
//! the canceled pointer's does not.
//!
//! Regression assertions are on the exact envelope stream plus the
//! handler side effects in `$state`, so a claimed LongPress, a duplicate
//! PressCancel or a stale arena wake can never slip through.

use jian_core::geometry::{point, Point};
use jian_core::gesture::{
    MouseButtons, PointerEvent, PointerId, PointerKind, PointerPhase, SemanticEvent,
    SemanticEventEnvelope,
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

/// Explicit `onLongPress` (authored 100ms duration) + press witness.
/// The long-press deadline crosses at t=100, so a Cancel at t=600 is
/// well past it.
const LONG_PRESS_OP: &str = r##"{"version":"0.8.0",
  "state":{"longs":{"type":"int","default":0},"pressed":{"type":"int","default":0},
           "cancelled":{"type":"int","default":0}},
  "children":[{"type":"rectangle","id":"btn","width":200,"height":100,
    "gestures":{"longPressDuration":100},
    "events":{"onLongPress":[{"set":{"$app.longs":"$app.longs + 1"}}],
              "onPressStart":[{"set":{"$app.pressed":"$app.pressed + 1"}}],
              "onPressCancel":[{"set":{"$app.cancelled":"$app.cancelled + 1"}}]}}]}"##;

/// Touch chain with an enabled `onContextMenu` but NO `onLongPress`: its
/// long-press deadline is the touch ContextMenu fallback.
const CONTEXT_MENU_OP: &str = r##"{"version":"0.8.0",
  "state":{"menu":{"type":"int","default":0},"pressed":{"type":"int","default":0},
           "cancelled":{"type":"int","default":0}},
  "children":[{"type":"rectangle","id":"btn","width":200,"height":100,
    "events":{"onContextMenu":[{"set":{"$app.menu":"$app.menu + 1"}}],
              "onPressStart":[{"set":{"$app.pressed":"$app.pressed + 1"}}],
              "onPressCancel":[{"set":{"$app.cancelled":"$app.cancelled + 1"}}]}}]}"##;

/// `onDoubleTap` chain (default 300ms window) + press witness: the first
/// completed Tap buffers as a pending Tap whose deadline is
/// `up_ms + 300`.
const DOUBLE_TAP_OP: &str = r##"{"version":"0.8.0",
  "state":{"taps":{"type":"int","default":0},"pressed":{"type":"int","default":0},
           "cancelled":{"type":"int","default":0}},
  "children":[{"type":"rectangle","id":"btn","width":200,"height":100,
    "events":{"onTap":[{"set":{"$app.taps":"$app.taps + 1"}}],
              "onDoubleTap":[{"set":{"$app.taps":"$app.taps + 100"}}],
              "onPressStart":[{"set":{"$app.pressed":"$app.pressed + 1"}}],
              "onPressCancel":[{"set":{"$app.cancelled":"$app.cancelled + 1"}}]}}]}"##;

/// Two-pointer isolation setup: `btn1` owns a 100ms `onLongPress` and NO
/// press handlers (a timer claim there surfaces cleanly — no loser
/// PressCancel), `btn2` owns an even earlier 80ms `onLongPress` plus
/// press handlers. At t=600 BOTH deadlines are overdue, so the cancel of
/// pid2 must deliver pid1's LongPress while suppressing pid2's own.
const TWO_LONG_PRESS_OP: &str = r##"{"version":"0.8.0",
  "state":{"longs":{"type":"int","default":0},"pressed":{"type":"int","default":0},
           "cancelled":{"type":"int","default":0}},
  "children":[
    {"type":"rectangle","id":"btn1","width":200,"height":100,
     "gestures":{"longPressDuration":100},
     "events":{"onLongPress":[{"set":{"$app.longs":"$app.longs + 1"}}]}},
    {"type":"rectangle","id":"btn2","x":300,"width":200,"height":100,
     "gestures":{"longPressDuration":80},
     "events":{"onLongPress":[{"set":{"$app.longs":"$app.longs + 1000"}}],
               "onPressStart":[{"set":{"$app.pressed":"$app.pressed + 1"}}],
               "onPressCancel":[{"set":{"$app.cancelled":"$app.cancelled + 1"}}]}}]}"##;

/// Two-pointer touch setup: `btn1` (0,0) owns a 100ms `onLongPress`;
/// `btn3` (300,0) is a touch ContextMenu chain — enabled `onContextMenu`,
/// no `onLongPress` — whose long-press deadline is the touch ContextMenu
/// fallback (default 500ms). Cancel of pid2 at t=600 must keep pid1's
/// LongPress and suppress pid2's ContextMenu.
const TWO_CONTEXT_MENU_OP: &str = r##"{"version":"0.8.0",
  "state":{"longs":{"type":"int","default":0},"menu":{"type":"int","default":0},
           "pressed":{"type":"int","default":0},"cancelled":{"type":"int","default":0}},
  "children":[
    {"type":"rectangle","id":"btn1","width":200,"height":100,
     "gestures":{"longPressDuration":100},
     "events":{"onLongPress":[{"set":{"$app.longs":"$app.longs + 1"}}]}},
    {"type":"rectangle","id":"btn3","x":300,"width":200,"height":100,
     "events":{"onContextMenu":[{"set":{"$app.menu":"$app.menu + 1"}}],
               "onPressStart":[{"set":{"$app.pressed":"$app.pressed + 1"}}],
               "onPressCancel":[{"set":{"$app.cancelled":"$app.cancelled + 1"}}]}}]}"##;

/// Three-pointer ordering setup: `btnDT` (0,0) is a DoubleTap chain whose
/// completed Tap buffers (deadline `up + 300`), `btn1` (0,150) owns a
/// 100ms `onLongPress`, `btn2` (300,0) is press-only. One Cancel at t=600
/// must surface, in order: pid1's overdue LongPress (deadline 100), the
/// due pending Tap (deadline 350), then pid2's PressCancel.
const THREE_POINTER_ORDER_OP: &str = r##"{"version":"0.8.0",
  "state":{"longs":{"type":"int","default":0},"taps":{"type":"int","default":0},
           "pressed":{"type":"int","default":0},"cancelled":{"type":"int","default":0}},
  "children":[
    {"type":"rectangle","id":"btnDT","width":200,"height":100,
     "events":{"onTap":[{"set":{"$app.taps":"$app.taps + 1"}}],
               "onDoubleTap":[{"set":{"$app.taps":"$app.taps + 100"}}],
               "onPressStart":[{"set":{"$app.pressed":"$app.pressed + 1"}}],
               "onPressCancel":[{"set":{"$app.cancelled":"$app.cancelled + 1"}}]}},
    {"type":"rectangle","id":"btn1","y":150,"width":200,"height":100,
     "gestures":{"longPressDuration":100},
     "events":{"onLongPress":[{"set":{"$app.longs":"$app.longs + 1"}}]}},
    {"type":"rectangle","id":"btn2","x":300,"width":200,"height":100,
     "events":{"onPressStart":[{"set":{"$app.pressed":"$app.pressed + 1"}}],
               "onPressCancel":[{"set":{"$app.cancelled":"$app.cancelled + 1"}}]}}]}"##;

// ---------------------------------------------------------------------
// Runtime pointer path
// ---------------------------------------------------------------------

/// Down at t0, then a system Cancel 600ms later — past the 100ms
/// LongPress deadline. The cancel must NOT tick the arena timer: the
/// stream is exactly one PressCancel (with the factual Cancel facts),
/// no LongPress, and nothing lingers for a later tick.
#[test]
fn runtime_cancel_past_long_press_deadline_claims_no_long_press() {
    let mut rt = runtime_with(LONG_PRESS_OP);
    let c = node_center(&rt, "btn");

    let down = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, c, 0));
    assert_eq!(envelope_names(&down), ["onPressStart"]);

    let cancel = rt.dispatch_pointer_events(mouse(1, PointerPhase::Cancel, c, 600));
    assert_eq!(
        envelope_names(&cancel),
        ["onPressCancel"],
        "a late Cancel must yield exactly one PressCancel, got {:?}",
        envelope_names(&cancel)
    );
    // The PressCancel envelope carries the factual Cancel event.
    let facts = cancel[0].pointer_facts.as_ref().expect("PressCancel facts");
    assert_eq!(facts.phase, PointerPhase::Cancel);
    assert_eq!(facts.t_ms, 600);
    assert_eq!(facts.position, c);

    assert_eq!(rt.state.app_get("longs").unwrap().as_i64(), Some(0));
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(1));
    // No stale arena / timer wake survives the cancel teardown.
    assert!(rt.tick(10_000).is_empty());
}

/// Touch Down at t0, then a system Cancel at t600 — past the touch
/// ContextMenu fallback deadline. No ContextMenu may be claimed, exactly
/// one PressCancel, and the fallback never fires.
#[test]
fn runtime_cancel_past_touch_context_menu_deadline_claims_no_context_menu() {
    let mut rt = runtime_with(CONTEXT_MENU_OP);
    let c = node_center(&rt, "btn");

    let down = rt.dispatch_pointer_events(touch(1, PointerPhase::Down, c, 0));
    assert_eq!(envelope_names(&down), ["onPressStart"]);

    let cancel = rt.dispatch_pointer_events(touch(1, PointerPhase::Cancel, c, 600));
    assert_eq!(
        envelope_names(&cancel),
        ["onPressCancel"],
        "the touch ContextMenu fallback must not claim off a late Cancel, got {:?}",
        envelope_names(&cancel)
    );
    assert_eq!(rt.state.app_get("menu").unwrap().as_i64(), Some(0));
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(1));
    assert!(rt.tick(10_000).is_empty());
}

/// The Cancel isolation is per-pointer, NOT global: pid1 Down at t=0 on
/// the 100ms LongPress node, pid2 Down at t=0 on another press-capable
/// node (its OWN 80ms LongPress deadline is overdue at t=600 too), pid2
/// Cancel at t=600. The stream must be pid1's overdue LongPress FIRST,
/// then pid2's PressCancel — pid2's own LongPress (and Tap/Pan) never
/// claims. Exactly-once counts on both handlers.
#[test]
fn runtime_cancel_delivers_other_pointers_overdue_long_press_but_not_own() {
    let mut rt = runtime_with(TWO_LONG_PRESS_OP);
    let c1 = node_center(&rt, "btn1");
    let c2 = node_center(&rt, "btn2");
    let btn1 = rt.document.as_ref().unwrap().tree.get("btn1").unwrap();
    let btn2 = rt.document.as_ref().unwrap().tree.get("btn2").unwrap();

    let down1 = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, c1, 0));
    assert!(
        down1.is_empty(),
        "no press handlers on btn1, got {:?}",
        envelope_names(&down1)
    );
    let down2 = rt.dispatch_pointer_events(mouse(2, PointerPhase::Down, c2, 0));
    assert_eq!(envelope_names(&down2), ["onPressStart"]);

    let cancel = rt.dispatch_pointer_events(mouse(2, PointerPhase::Cancel, c2, 600));
    assert_eq!(
        envelope_names(&cancel),
        ["onLongPress", "onPressCancel"],
        "pid1's overdue LongPress must fire before pid2's cancel; pid2's own timer must not, got {:?}",
        envelope_names(&cancel)
    );
    // pid1's LongPress envelope carries pid1's Down facts and routes to btn1.
    let lp = &cancel[0];
    assert_eq!(lp.event.node(), btn1);
    let lp_facts = lp.pointer_facts.as_ref().expect("LongPress facts");
    assert_eq!(lp_facts.id, PointerId(1));
    assert_eq!(lp_facts.phase, PointerPhase::Down);
    assert_eq!(lp_facts.t_ms, 0);
    assert_eq!(lp_facts.position, c1);
    // pid2's PressCancel carries the factual Cancel event of pid2.
    let pc = &cancel[1];
    assert_eq!(pc.event.node(), btn2);
    let pc_facts = pc.pointer_facts.as_ref().expect("PressCancel facts");
    assert_eq!(pc_facts.id, PointerId(2));
    assert_eq!(pc_facts.phase, PointerPhase::Cancel);
    assert_eq!(pc_facts.t_ms, 600);
    assert_eq!(pc_facts.position, c2);

    // Exactly-once: only pid1's LongPress ran (+1), pid2's (+1000) never
    // did; pid2's press started and was canceled exactly once.
    assert_eq!(rt.state.app_get("longs").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("pressed").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(1));
    assert!(rt.tick(10_000).is_empty());
}

/// Touch variant of the same contract: pid2 is TOUCH on a ContextMenu
/// chain (its long-press deadline is the touch ContextMenu fallback,
/// overdue at t=600). Canceling pid2 must keep pid1's LongPress and
/// produce NO ContextMenu for pid2 — exactly one PressCancel.
#[test]
fn runtime_cancel_of_touch_menu_pointer_keeps_other_long_press_and_no_menu() {
    let mut rt = runtime_with(TWO_CONTEXT_MENU_OP);
    let c1 = node_center(&rt, "btn1");
    let c3 = node_center(&rt, "btn3");

    let down1 = rt.dispatch_pointer_events(touch(1, PointerPhase::Down, c1, 0));
    assert!(
        down1.is_empty(),
        "no press handlers on btn1, got {:?}",
        envelope_names(&down1)
    );
    let down2 = rt.dispatch_pointer_events(touch(2, PointerPhase::Down, c3, 0));
    assert_eq!(envelope_names(&down2), ["onPressStart"]);

    let cancel = rt.dispatch_pointer_events(touch(2, PointerPhase::Cancel, c3, 600));
    assert_eq!(
        envelope_names(&cancel),
        ["onLongPress", "onPressCancel"],
        "pid1's LongPress fires; the canceled touch pointer claims no ContextMenu, got {:?}",
        envelope_names(&cancel)
    );
    assert_eq!(cancel[0].pointer_facts.as_ref().unwrap().id, PointerId(1));
    assert_eq!(cancel[1].pointer_facts.as_ref().unwrap().id, PointerId(2));
    assert_eq!(rt.state.app_get("longs").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("menu").unwrap().as_i64(), Some(0));
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(1));
    assert!(rt.tick(10_000).is_empty());
}

// ---------------------------------------------------------------------
// Public PointerRouter dispatch path
// ---------------------------------------------------------------------

/// The same Cancel contract through the public `dispatch_enveloped`
/// entry point (static disabled predicate): a late Cancel claims no
/// LongPress, ends with exactly one PressCancel, and leaves no wake.
#[test]
fn public_router_cancel_past_deadline_claims_no_long_press() {
    let mut rt = runtime_with(LONG_PRESS_OP);
    let c = node_center(&rt, "btn");
    let doc = rt.document.as_ref().unwrap();
    let spatial = &rt.spatial;
    let router = &mut rt.gestures;

    let down = router.dispatch_enveloped(mouse(1, PointerPhase::Down, c, 0), doc, spatial);
    assert_eq!(envelope_names(&down), ["onPressStart"]);

    let cancel = router.dispatch_enveloped(mouse(1, PointerPhase::Cancel, c, 600), doc, spatial);
    assert_eq!(
        envelope_names(&cancel),
        ["onPressCancel"],
        "the public dispatch path must honor the same Cancel contract, got {:?}",
        envelope_names(&cancel)
    );
    // The public router returns envelopes only (delivery is the runtime's
    // job), so the long-press absence is asserted on the envelope stream.
    assert!(
        !cancel
            .iter()
            .any(|e| matches!(e.event, SemanticEvent::LongPress { .. })),
        "no LongPress envelope may be produced off a late Cancel"
    );
    assert!(router.tick_enveloped(10_000).is_empty());
}

/// The same two-pointer per-pointer isolation through the public
/// `dispatch_enveloped` entry point: pid1's overdue LongPress envelope
/// comes FIRST (with pid1's Down facts), then pid2's PressCancel
/// envelope (with the factual Cancel event) — exactly one LongPress and
/// no timer envelope for the canceled pointer.
#[test]
fn public_router_cancel_delivers_other_pointers_overdue_long_press_but_not_own() {
    let mut rt = runtime_with(TWO_LONG_PRESS_OP);
    let c1 = node_center(&rt, "btn1");
    let c2 = node_center(&rt, "btn2");
    let btn1 = rt.document.as_ref().unwrap().tree.get("btn1").unwrap();
    let btn2 = rt.document.as_ref().unwrap().tree.get("btn2").unwrap();
    let doc = rt.document.as_ref().unwrap();
    let spatial = &rt.spatial;
    let router = &mut rt.gestures;

    let _ = router.dispatch_enveloped(mouse(1, PointerPhase::Down, c1, 0), doc, spatial);
    let _ = router.dispatch_enveloped(mouse(2, PointerPhase::Down, c2, 0), doc, spatial);

    let cancel = router.dispatch_enveloped(mouse(2, PointerPhase::Cancel, c2, 600), doc, spatial);
    assert_eq!(
        envelope_names(&cancel),
        ["onLongPress", "onPressCancel"],
        "the public dispatch path must isolate per-pointer timers identically, got {:?}",
        envelope_names(&cancel)
    );

    // Exactly ONE LongPress envelope, for pid1, with pid1's Down facts.
    let long_presses: Vec<&SemanticEventEnvelope> = cancel
        .iter()
        .filter(|e| matches!(e.event, SemanticEvent::LongPress { .. }))
        .collect();
    assert_eq!(long_presses.len(), 1, "pid2's LongPress must never claim");
    let lp = long_presses[0];
    assert_eq!(lp.event.node(), btn1);
    let lp_facts = lp.pointer_facts.as_ref().expect("LongPress facts");
    assert_eq!(lp_facts.id, PointerId(1));
    assert_eq!(lp_facts.phase, PointerPhase::Down);
    assert_eq!(lp_facts.t_ms, 0);

    // No ContextMenu envelope may exist for the canceled pointer either.
    assert!(
        !cancel
            .iter()
            .any(|e| matches!(e.event, SemanticEvent::ContextMenu { .. })),
        "no ContextMenu envelope may be produced off a late Cancel"
    );

    // pid2's PressCancel carries the factual Cancel event of pid2.
    let pc = &cancel[1];
    assert_eq!(pc.event.node(), btn2);
    let pc_facts = pc.pointer_facts.as_ref().expect("PressCancel facts");
    assert_eq!(pc_facts.id, PointerId(2));
    assert_eq!(pc_facts.phase, PointerPhase::Cancel);
    assert_eq!(pc_facts.t_ms, 600);
    assert_eq!(pc_facts.position, c2);

    // No stale arena wake survives the cancel teardown.
    assert!(router.tick_enveloped(10_000).is_empty());
}

// ---------------------------------------------------------------------
// Non-Cancel phases keep timer-before-current
// ---------------------------------------------------------------------

/// A Move at t=600 (past the 100ms deadline) still crosses the arena
/// timer BEFORE the current event: `[PressCancel, LongPress]` — only
/// Cancel is exempt from timer-before-current.
#[test]
fn move_past_deadline_still_claims_long_press_before_current() {
    let mut rt = runtime_with(LONG_PRESS_OP);
    let c = node_center(&rt, "btn");

    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, c, 0));
    let move_ev = rt.dispatch_pointer_events(mouse(1, PointerPhase::Move, c, 600));
    assert_eq!(
        envelope_names(&move_ev),
        ["onPressCancel", "onLongPress"],
        "non-Cancel phases keep timer-before-current, got {:?}",
        envelope_names(&move_ev)
    );
    assert_eq!(rt.state.app_get("longs").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(1));
}

// ---------------------------------------------------------------------
// Due pending Tap on an unrelated Cancel
// ---------------------------------------------------------------------

/// A completed tap on a DoubleTap chain buffers; an UNRELATED pointer's
/// late Cancel (t=600, past BOTH the pending tap's 350ms deadline and the
/// cancelling pointer's own 500ms LongPress deadline) must flush the due
/// Tap while claiming nothing from the cancelling pointer's arena: the
/// stream is exactly `[onTap, onPressCancel]`.
#[test]
fn unrelated_cancel_flushes_due_pending_tap_and_claims_nothing() {
    let mut rt = runtime_with(DOUBLE_TAP_OP);
    let c = node_center(&rt, "btn");

    // Pointer 1 completes a Tap on the DoubleTap chain: the Tap buffers
    // (nothing delivered as Tap; deadline = 50 + 300 = 350). The press
    // witness ends NORMALLY (PressEnd) — only the Tap defers.
    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, c, 0));
    let up = rt.dispatch_pointer_events(mouse(1, PointerPhase::Up, c, 50));
    assert_eq!(
        envelope_names(&up),
        ["onPressEnd"],
        "the first Tap must buffer (only PressEnd surfaces), got {:?}",
        envelope_names(&up)
    );

    // Pointer 2 crosses the same chain, then receives a system Cancel at
    // t=600 — past the pending tap's deadline AND its own LongPress
    // deadline (0 + 500 = 500, legacy unconditional long-press).
    let _ = rt.dispatch_pointer_events(mouse(2, PointerPhase::Down, c, 0));
    let cancel = rt.dispatch_pointer_events(mouse(2, PointerPhase::Cancel, c, 600));
    assert_eq!(
        envelope_names(&cancel),
        ["onTap", "onPressCancel"],
        "due pending Tap flushes first; the Cancel claims nothing, got {:?}",
        envelope_names(&cancel)
    );
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("pressed").unwrap().as_i64(), Some(2));
    assert!(rt.tick(10_000).is_empty());
}

/// Combined ordering: pid3 completed a Tap on a DoubleTap chain (buffered
/// until t=350), pid1 holds an overdue 100ms LongPress, and pid2 is
/// canceled at t=600. The stream must be exactly
/// `[LongPress(pid1), Tap, PressCancel(pid2)]` — other-arena timer
/// first, then the due pending Tap (exactly once, as before), then the
/// current cancel — each handler exactly once.
#[test]
fn runtime_cancel_orders_other_timer_then_pending_tap_then_cancel() {
    let mut rt = runtime_with(THREE_POINTER_ORDER_OP);
    let cdt = node_center(&rt, "btnDT");
    let c1 = node_center(&rt, "btn1");
    let c2 = node_center(&rt, "btn2");

    // pid3 completes a Tap on the DoubleTap chain at t=50: the Tap
    // buffers (deadline 350), only PressEnd surfaces.
    let _ = rt.dispatch_pointer_events(mouse(3, PointerPhase::Down, cdt, 0));
    let up = rt.dispatch_pointer_events(mouse(3, PointerPhase::Up, cdt, 50));
    assert_eq!(envelope_names(&up), ["onPressEnd"]);

    // pid1 Down at t=0 on the 100ms LongPress node, pid2 Down at t=0 on
    // the press-only node.
    let down1 = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, c1, 0));
    assert!(
        down1.is_empty(),
        "no press handlers on btn1, got {:?}",
        envelope_names(&down1)
    );
    let down2 = rt.dispatch_pointer_events(mouse(2, PointerPhase::Down, c2, 0));
    assert_eq!(envelope_names(&down2), ["onPressStart"]);

    // pid2 Cancel at t=600: pid1's overdue LongPress (deadline 100) is
    // delivered FIRST, then the due pending Tap (deadline 350) — flushes
    // exactly once, as before — then pid2's cancel; pid2's own legacy
    // LongPress (500ms) never claims.
    let cancel = rt.dispatch_pointer_events(mouse(2, PointerPhase::Cancel, c2, 600));
    assert_eq!(
        envelope_names(&cancel),
        ["onLongPress", "onTap", "onPressCancel"],
        "timer-before-current for the other arena, then the due pending Tap, got {:?}",
        envelope_names(&cancel)
    );
    assert_eq!(cancel[0].pointer_facts.as_ref().unwrap().id, PointerId(1));
    assert_eq!(cancel[1].pointer_facts.as_ref().unwrap().id, PointerId(3));
    assert_eq!(cancel[2].pointer_facts.as_ref().unwrap().id, PointerId(2));

    // Exactly-once counts: one LongPress, one flushed Tap, one cancel;
    // press starts: pid3 (btnDT) + pid2 (btn2).
    assert_eq!(rt.state.app_get("longs").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("pressed").unwrap().as_i64(), Some(2));
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(1));
    // A follow-up tick must not re-flush the Tap or claim anything.
    assert!(rt.tick(10_000).is_empty());
}
