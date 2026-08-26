//! R2B same-batch Swipe ownership re-validation.
//!
//! The reviewed P1: a PressCancel derived in the SAME raw-event batch as
//! the Swipe claim must be able to disable the Swipe's captured owner,
//! and the claimed Swipe must then be dropped — never re-resolved to an
//! ancestor handler.
//!
//! Channel: the child owns `onSwipe` at authored 30px/100px/s and its
//! `onPressCancel` action flips `$app.off`, which gates the child's
//! `gestures.disabled`. The parent owns `onSwipe` at the shared defaults
//! 48px/320px/s. A Down on the child then a 40px/200ms Move qualifies the
//! child's captured thresholds but NOT the parent's. The arena derives
//! `[PressCancel, Swipe]` in one batch; the PressCancel action runs
//! first, so by the time the Swipe envelope is considered its captured
//! owner is dynamically disabled. Delivery must drop it: no handler runs
//! (child count AND parent count stay 0) and the returned batch contains
//! only the PressCancel — never a Swipe report, since the parent's
//! thresholds never qualified.
//!
//! `disabledEvents` variant: the same sequence with the middle ancestor's
//! `onSwipe` statically slated. A naive "skip disabled nodes and keep
//! bubbling" repair would still rebind the claimed Swipe to the ENABLED
//! grandparent; the owner-anchored rule drops it instead.

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

fn count(rt: &Runtime, key: &str) -> i64 {
    rt.state.app_get(key).unwrap().as_i64().unwrap()
}

fn flag(rt: &Runtime, key: &str) -> bool {
    rt.state.app_get(key).unwrap().as_bool().unwrap()
}

/// The exact P1 fixture: child owns onSwipe at 30px/100px/s and
/// `onPressCancel` disables it via `$app.off`; parent owns onSwipe at the
/// shared 48px/320px/s defaults.
const SAME_BATCH_OP: &str = r##"{"version":"0.8.0",
  "state":{"off":{"type":"bool","default":false},
           "childSwipe":{"type":"int","default":0},
           "parentSwipe":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","x":0,"y":0,"width":400,"height":400,
    "events":{"onSwipe":[{"set":{"$app.parentSwipe":"$app.parentSwipe + 1"}}]},
    "children":[{"type":"rectangle","id":"child","x":10,"y":10,"width":100,"height":100,
      "gestures":{"disabled":"$app.off","swipeMinDistance":30,"swipeMinVelocity":100},
      "events":{
        "onPressCancel":[{"set":{"$app.off":"true"}}],
        "onSwipe":[{"set":{"$app.childSwipe":"$app.childSwipe + 2"}}]}}]}]}"##;

#[test]
fn same_batch_press_cancel_disables_captured_child_swipe_and_never_rebinds_parent() {
    let mut rt = runtime_with(SAME_BATCH_OP);
    let c = node_center(&rt, "child");
    let parent_key = rt.document.as_ref().unwrap().tree.get("root").unwrap();

    // Down while the child is enabled: the Swipe recognizer captures the
    // CHILD with its authored 30px/100px/s thresholds; the press witness
    // installs because the chain declares an enabled onPressCancel.
    let down = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
    assert_eq!(
        names(&down),
        ["onPressStart"],
        "press witness emits PressStart on the Down, got {:?}",
        names(&down)
    );

    // The exact failing Move: 40px over 200ms = 200 px/s. The arena
    // derives [PressCancel, Swipe] in ONE batch. The PressCancel action
    // flips `$app.off` FIRST; the Swipe's captured owner (the child) is
    // then dynamically disabled and the claim must be dropped.
    let batch = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 40.0, c.y),
        200,
    ));
    assert_eq!(
        names(&batch),
        ["onPressCancel"],
        "same-batch ownership: only the cancellation is reported, got {:?}",
        names(&batch)
    );
    assert!(
        !batch
            .iter()
            .any(|e| matches!(e.event, SemanticEvent::Swipe { .. })),
        "the host-visible batch must not report a claimed Swipe: {batch:?}"
    );
    assert!(
        flag(&rt, "off"),
        "the PressCancel action must have flipped $app.off"
    );
    assert_eq!(count(&rt, "childSwipe"), 0);
    assert_eq!(count(&rt, "parentSwipe"), 0);

    // Close the pointer; nothing further may fire for the sequence.
    let up = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Up,
        point(c.x + 40.0, c.y),
        300,
    ));
    assert!(up.is_empty(), "one-shot Swipe, got {up:?}");
    assert_eq!(count(&rt, "childSwipe"), 0);
    assert_eq!(count(&rt, "parentSwipe"), 0);

    // A FRESH Down re-resolves the nearest ENABLED owner: the parent
    // (child is disabled) governs with 48px/320px/s. The same 40px/200ms
    // still fails the parent gate...
    let _ = rt.dispatch_pointer_events(mouse_event(2, PointerPhase::Down, c, 400));
    let too_small = rt.dispatch_pointer_events(mouse_event(
        2,
        PointerPhase::Move,
        point(c.x + 40.0, c.y),
        700,
    ));
    assert!(
        too_small.is_empty(),
        "parent 48px gate holds: {too_small:?}"
    );
    // ...and 80px over the final 50ms (= 800 px/s) claims at the PARENT.
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
    assert_eq!(count(&rt, "childSwipe"), 0);
    assert_eq!(count(&rt, "parentSwipe"), 1);
    let up = rt.dispatch_pointer_events(mouse_event(
        2,
        PointerPhase::Up,
        point(c.x + 80.0, c.y),
        900,
    ));
    assert!(up.is_empty(), "got {up:?}");
}

/// Same fixture, but the MIDDLE ancestor's `onSwipe` is statically slated
/// via `disabledEvents` while the grandparent's stays enabled — a
/// skip-and-continue repair would rebind to the grandparent.
const STATIC_SLATE_OP: &str = r##"{"version":"0.8.0",
  "state":{"off":{"type":"bool","default":false},
           "childSwipe":{"type":"int","default":0},
           "parentSwipe":{"type":"int","default":0},
           "rootSwipe":{"type":"int","default":0}},
  "children":[{"type":"frame","id":"root","x":0,"y":0,"width":400,"height":400,
    "events":{"onSwipe":[{"set":{"$app.rootSwipe":"$app.rootSwipe + 1"}}]},
    "children":[{"type":"frame","id":"mid","x":0,"y":0,"width":400,"height":400,
      "gestures":{"disabledEvents":["onSwipe"]},
      "events":{"onSwipe":[{"set":{"$app.parentSwipe":"$app.parentSwipe + 1"}}]},
      "children":[{"type":"rectangle","id":"child","x":10,"y":10,"width":100,"height":100,
        "gestures":{"disabled":"$app.off","swipeMinDistance":30,"swipeMinVelocity":100},
        "events":{
          "onPressCancel":[{"set":{"$app.off":"true"}}],
          "onSwipe":[{"set":{"$app.childSwipe":"$app.childSwipe + 2"}}]}}]}]}]}"##;

#[test]
fn same_batch_press_cancel_drops_swipe_even_with_statically_slated_ancestor() {
    let mut rt = runtime_with(STATIC_SLATE_OP);
    let c = node_center(&rt, "child");

    let _ = rt.dispatch_pointer_events(mouse_event(1, PointerPhase::Down, c, 0));
    let batch = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Move,
        point(c.x + 40.0, c.y),
        200,
    ));
    assert_eq!(
        names(&batch),
        ["onPressCancel"],
        "no Swipe may be reported, got {:?}",
        names(&batch)
    );
    assert!(
        !batch
            .iter()
            .any(|e| matches!(e.event, SemanticEvent::Swipe { .. })),
        "nothing may report a Swipe: {batch:?}"
    );
    // Neither the skipped (statically slated) middle ancestor nor the
    // ENABLED root may have run — dropping the claim is owner-anchored,
    // not skip-and-continue bubbling.
    assert_eq!(count(&rt, "childSwipe"), 0);
    assert_eq!(count(&rt, "parentSwipe"), 0);
    assert_eq!(count(&rt, "rootSwipe"), 0);
    let up = rt.dispatch_pointer_events(mouse_event(
        1,
        PointerPhase::Up,
        point(c.x + 40.0, c.y),
        300,
    ));
    assert!(up.is_empty(), "got {up:?}");
    assert_eq!(count(&rt, "rootSwipe"), 0);
}
