//! Plan 5 T16 — end-to-end tap counter integration test.
//!
//! A pointer Down + Up over a rectangle with `events.onTap` that increments
//! `$app.count` must run the handler exactly once and leave the state at 1.

use jian_core::gesture::{PointerEvent, PointerPhase};
use jian_core::Runtime;

const COUNTER_OP: &str = r##"{
  "formatVersion": "1.0",
  "version": "1.0.0",
  "state": { "count": { "type": "int", "default": 0 } },
  "children": [
    {
      "type": "rectangle",
      "id": "btn",
      "width": 200,
      "height": 100,
      "fill": [{ "type": "solid", "color": "#1e88e5" }],
      "events": {
        "onTap": [ { "set": { "$app.count": "$app.count + 1" } } ]
      }
    }
  ]
}"##;

fn make_runtime() -> Runtime {
    let mut rt = Runtime::new();
    rt.load_str(COUNTER_OP).unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    rt
}

#[test]
fn tap_increments_app_count() {
    let mut rt = make_runtime();

    // Button lives at origin 0,0 with size 200x100; center is (100, 50).
    let btn = rt
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("btn")
        .expect("btn exists");
    let rect = rt.layout.node_rect(btn).unwrap();
    let cx = rect.min_x() + rect.size.width / 2.0;
    let cy = rect.min_y() + rect.size.height / 2.0;

    assert_eq!(rt.state.app_get("count").unwrap().as_i64(), Some(0));

    let down = PointerEvent::simple(1, PointerPhase::Down, jian_core::geometry::point(cx, cy));
    let up = PointerEvent::simple(1, PointerPhase::Up, jian_core::geometry::point(cx, cy));

    let emitted_down = rt.dispatch_pointer(down);
    assert!(emitted_down.is_empty(), "no tap on Down alone");

    let emitted_up = rt.dispatch_pointer(up);
    assert_eq!(
        emitted_up.len(),
        1,
        "expected one Tap semantic event, got {:?}",
        emitted_up
    );

    assert_eq!(
        rt.state.app_get("count").unwrap().as_i64(),
        Some(1),
        "onTap handler should have incremented $app.count to 1"
    );
}

#[test]
fn drag_past_slop_rejects_tap() {
    // Moving the pointer past 8px slop should cancel the Tap recognizer and
    // leave $app.count at 0 — even though an Up eventually lands on the node.
    let mut rt = make_runtime();
    let btn = rt.document.as_ref().unwrap().tree.get("btn").unwrap();
    let rect = rt.layout.node_rect(btn).unwrap();
    let cx = rect.min_x() + rect.size.width / 2.0;
    let cy = rect.min_y() + rect.size.height / 2.0;

    let _ = rt.dispatch_pointer(PointerEvent::simple(
        2,
        PointerPhase::Down,
        jian_core::geometry::point(cx, cy),
    ));
    let _ = rt.dispatch_pointer(PointerEvent::simple(
        2,
        PointerPhase::Move,
        jian_core::geometry::point(cx + 40.0, cy),
    ));
    let _ = rt.dispatch_pointer(PointerEvent::simple(
        2,
        PointerPhase::Up,
        jian_core::geometry::point(cx + 40.0, cy),
    ));
    assert_eq!(rt.state.app_get("count").unwrap().as_i64(), Some(0));
}

#[test]
fn double_tap_on_same_spot_fires_double_tap_event() {
    let mut rt = make_runtime();
    let btn = rt.document.as_ref().unwrap().tree.get("btn").unwrap();
    let rect = rt.layout.node_rect(btn).unwrap();
    let cx = rect.min_x() + rect.size.width / 2.0;
    let cy = rect.min_y() + rect.size.height / 2.0;

    // First tap.
    let _ = rt.dispatch_pointer(PointerEvent::simple(
        3,
        PointerPhase::Down,
        jian_core::geometry::point(cx, cy),
    ));
    let first_up = rt.dispatch_pointer(PointerEvent::simple(
        3,
        PointerPhase::Up,
        jian_core::geometry::point(cx, cy),
    ));
    assert_eq!(first_up.len(), 1);
    assert!(matches!(
        first_up[0],
        jian_core::gesture::SemanticEvent::Tap { .. }
    ));

    // Second tap immediately — router tracks Tap history across arenas.
    let _ = rt.dispatch_pointer(PointerEvent::simple(
        4,
        PointerPhase::Down,
        jian_core::geometry::point(cx + 1.0, cy + 1.0),
    ));
    let second_up = rt.dispatch_pointer(PointerEvent::simple(
        4,
        PointerPhase::Up,
        jian_core::geometry::point(cx + 1.0, cy + 1.0),
    ));
    assert!(second_up
        .iter()
        .any(|e| matches!(e, jian_core::gesture::SemanticEvent::Tap { .. })));
    assert!(second_up
        .iter()
        .any(|e| matches!(e, jian_core::gesture::SemanticEvent::DoubleTap { .. })));
}

#[test]
fn long_press_claim_via_tick_suppresses_subsequent_tap() {
    // Regression: before the arena.tick fix, a LongPress claim via tick
    // didn't resolve the arena, so a subsequent Up still let Tap claim.
    let mut rt = make_runtime();
    let btn = rt.document.as_ref().unwrap().tree.get("btn").unwrap();
    let rect = rt.layout.node_rect(btn).unwrap();
    let cx = rect.min_x() + rect.size.width / 2.0;
    let cy = rect.min_y() + rect.size.height / 2.0;

    let _ = rt.dispatch_pointer(PointerEvent::simple(
        5,
        PointerPhase::Down,
        jian_core::geometry::point(cx, cy),
    ));
    // Advance time past LongPress duration (500ms default).
    let future = std::time::Instant::now() + std::time::Duration::from_millis(800);
    let tick_emitted = rt.tick(future);
    assert!(tick_emitted
        .iter()
        .any(|e| matches!(e, jian_core::gesture::SemanticEvent::LongPress { .. })));

    // Now Up — Tap must NOT fire; the arena already resolved LongPress.
    let up_emitted = rt.dispatch_pointer(PointerEvent::simple(
        5,
        PointerPhase::Up,
        jian_core::geometry::point(cx, cy),
    ));
    assert!(!up_emitted
        .iter()
        .any(|e| matches!(e, jian_core::gesture::SemanticEvent::Tap { .. })));
    assert_eq!(rt.state.app_get("count").unwrap().as_i64(), Some(0));
}

#[test]
fn dispatch_pointer_flushes_bindings() {
    // Regression (Codex review): Runtime::dispatch_pointer used to run
    // actions without flushing the signal scheduler, leaving bindings
    // stale. A binding observer captures effect invocations — after a
    // tap, it must see the new value without any manual flush.
    use jian_core::binding::BindingEffect;
    use jian_core::expression::Expression;
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut rt = make_runtime();
    let btn = rt.document.as_ref().unwrap().tree.get("btn").unwrap();
    let rect = rt.layout.node_rect(btn).unwrap();
    let cx = rect.min_x() + rect.size.width / 2.0;
    let cy = rect.min_y() + rect.size.height / 2.0;

    let observed: Rc<RefCell<Vec<i64>>> = Rc::new(RefCell::new(Vec::new()));
    let obs = observed.clone();
    let expr = Expression::compile("$app.count").unwrap();
    let _binding = BindingEffect::new(
        &rt.effects,
        expr,
        rt.state.clone(),
        None,
        None,
        move |v, _| {
            if let Some(n) = v.as_i64() {
                obs.borrow_mut().push(n);
            }
        },
    );

    // Initial eval fires once with count=0.
    assert_eq!(observed.borrow().as_slice(), &[0]);

    let _ = rt.dispatch_pointer(PointerEvent::simple(
        7,
        PointerPhase::Down,
        jian_core::geometry::point(cx, cy),
    ));
    let _ = rt.dispatch_pointer(PointerEvent::simple(
        7,
        PointerPhase::Up,
        jian_core::geometry::point(cx, cy),
    ));

    // No explicit scheduler.flush() call — the runtime should have
    // flushed internally after the tap fired its handler.
    assert_eq!(
        observed.borrow().as_slice(),
        &[0, 1],
        "binding observer should have seen both 0 (initial) and 1 (after tap)"
    );
}

#[test]
fn miss_outside_button_does_not_fire_tap() {
    let mut rt = make_runtime();
    let down = PointerEvent::simple(
        1,
        PointerPhase::Down,
        jian_core::geometry::point(500.0, 500.0),
    );
    let up = PointerEvent::simple(
        1,
        PointerPhase::Up,
        jian_core::geometry::point(500.0, 500.0),
    );
    let _ = rt.dispatch_pointer(down);
    let _ = rt.dispatch_pointer(up);
    assert_eq!(rt.state.app_get("count").unwrap().as_i64(), Some(0));
}
