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
      "fills": [{ "type": "solid", "color": "#1e88e5" }],
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
