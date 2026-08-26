//! R2A semantic-trace tests: PressStart/End/Cancel, ContextMenu, and
//! handler-aware Tap/DoubleTap deferral, driven through the public
//! `Runtime` API with factual `PointerEvent` inputs.
//!
//! Assertions are on the *order and count* of emit semantic events (via
//! `handler_key`) plus the handler side effects in `$state`, so a
//! cancellation or a duplicate emission can never slip through.

use jian_core::geometry::{point, Point};
use jian_core::gesture::{PointerEvent, PointerId, PointerKind, PointerPhase, SemanticEvent};
use jian_core::Runtime;

const PRESS_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": {
    "pressed": { "type": "int", "default": 0 },
    "taps": { "type": "int", "default": 0 },
    "cancelled": { "type": "int", "default": 0 }
  },
  "children": [{
    "type": "rectangle", "id": "btn", "width": 200, "height": 100,
    "events": {
      "onPressStart": [ { "set": { "$app.pressed": "1" } } ],
      "onPressEnd": [ { "set": { "$app.pressed": "0" } } ],
      "onPressCancel": [ { "set": { "$app.cancelled": "1" } } ],
      "onTap": [ { "set": { "$app.taps": "$app.taps + 1" } } ]
    }
  }]
}"##;

/// Press + Pan fixture (added Pan handlers; Pan claims at 8px default).
const PAN_PRESS_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": {
    "pressed": { "type": "int", "default": 0 },
    "cancelled": { "type": "int", "default": 0 },
    "pans": { "type": "int", "default": 0 },
    "pan_updates": { "type": "int", "default": 0 }
  },
  "children": [{
    "type": "rectangle", "id": "btn", "width": 200, "height": 100,
    "events": {
      "onPressStart": [ { "set": { "$app.pressed": "1" } } ],
      "onPressCancel": [ { "set": { "$app.cancelled": "1" } } ],
      "onPanStart": [ { "set": { "$app.pans": "$app.pans + 1" } } ],
      "onPanUpdate": [ { "set": { "$app.pan_updates": "$app.pan_updates + 1" } } ]
    }
  }]
}"##;

/// Press + LongPress fixture (authored 120ms long-press).
const LONG_PRESS_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": {
    "pressed": { "type": "int", "default": 0 },
    "cancelled": { "type": "int", "default": 0 },
    "longs": { "type": "int", "default": 0 }
  },
  "children": [{
    "type": "rectangle", "id": "btn", "width": 200, "height": 100,
    "gestures": { "longPressDuration": 120 },
    "events": {
      "onPressStart": [ { "set": { "$app.pressed": "1" } } ],
      "onPressCancel": [ { "set": { "$app.cancelled": "1" } } ],
      "onLongPress": [ { "set": { "$app.longs": "$app.longs + 1" } } ]
    }
  }]
}"##;

/// Press + two-finger Scale fixture.
const SCALE_PRESS_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": {
    "pressed": { "type": "int", "default": 0 },
    "cancelled": { "type": "int", "default": 0 },
    "scales": { "type": "int", "default": 0 }
  },
  "children": [{
    "type": "rectangle", "id": "btn", "width": 800, "height": 600,
    "events": {
      "onPressStart": [ { "set": { "$app.pressed": "1" } } ],
      "onPressCancel": [ { "set": { "$app.cancelled": "1" } } ],
      "onScaleStart": [ { "set": { "$app.scales": "$app.scales + 1" } } ]
    }
  }]
}"##;

/// Context-menu-only fixture (desktop right-button + touch fallback).
const CONTEXT_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": {
    "menu": { "type": "int", "default": 0 },
    "longs": { "type": "int", "default": 0 }
  },
  "children": [{
    "type": "rectangle", "id": "btn", "width": 200, "height": 100,
    "events": {
      "onContextMenu": [ { "set": { "$app.menu": "$app.menu + 1" } } ],
      "onPressStart": [ { "set": { "$app.menu": "$app.menu + 100" } } ],
      "onPressEnd": [ { "set": { "$app.menu": "$app.menu + 1000" } } ]
    }
  }]
}"##;

/// LongPress + ContextMenu both declared — explicit onLongPress wins.
const LONG_PRESS_WINS_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": { "longs": { "type": "int", "default": 0 }, "menu": { "type": "int", "default": 0 } },
  "children": [{
    "type": "rectangle", "id": "btn", "width": 200, "height": 100,
    "events": {
      "onLongPress": [ { "set": { "$app.longs": "$app.longs + 1" } } ],
      "onContextMenu": [ { "set": { "$app.menu": "$app.menu + 1" } } ]
    }
  }]
}"##;

/// Hover fixture.
const HOVER_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": { "enters": { "type": "int", "default": 0 }, "leaves": { "type": "int", "default": 0 } },
  "children": [{
    "type": "rectangle", "id": "btn", "width": 200, "height": 100,
    "events": {
      "onHoverEnter": [ { "set": { "$app.enters": "$app.enters + 1" } } ],
      "onHoverLeave": [ { "set": { "$app.leaves": "$app.leaves + 1" } } ]
    }
  }]
}"##;

/// Parent owns onTap; child declares onTap + disabledEvents (skip+bubble).
const DISABLED_EVENTS_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": { "child": { "type": "int", "default": 0 }, "parent": { "type": "int", "default": 0 } },
  "children": [{
    "type": "frame", "id": "root", "width": 400, "height": 400,
    "events": { "onTap": [ { "set": { "$app.parent": "$app.parent + 1" } } ] },
    "children": [{
      "type": "rectangle", "id": "child", "x": 10, "y": 10, "width": 100, "height": 100,
      "gestures": { "disabledEvents": ["onTap"] },
      "events": { "onTap": [ { "set": { "$app.child": "$app.child + 1" } } ] }
    }]
  }]
}"##;

/// Parent owns onTap; child declares onTap + `gestures.disabled`.
const GESTURES_DISABLED_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": {
    "off": { "type": "bool", "default": false },
    "child": { "type": "int", "default": 0 },
    "parent": { "type": "int", "default": 0 }
  },
  "children": [{
    "type": "frame", "id": "root", "width": 400, "height": 400,
    "events": { "onTap": [ { "set": { "$app.parent": "$app.parent + 1" } } ] },
    "children": [{
      "type": "rectangle", "id": "child", "x": 10, "y": 10, "width": 100, "height": 100,
      "gestures": { "disabled": "$app.off" },
      "events": { "onTap": [ { "set": { "$app.child": "$app.child + 1" } } ] }
    }]
  }]
}"##;

/// Authored thresholds: dragThreshold 30, longPressDuration 100.
const THRESHOLDS_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": { "pans": { "type": "int", "default": 0 }, "longs": { "type": "int", "default": 0 } },
  "children": [{
    "type": "rectangle", "id": "btn", "width": 200, "height": 100,
    "gestures": { "dragThreshold": 30, "longPressDuration": 100 },
    "events": {
      "onPanStart": [ { "set": { "$app.pans": "$app.pans + 1" } } ],
      "onLongPress": [ { "set": { "$app.longs": "$app.longs + 1" } } ]
    }
  }]
}"##;

/// Authored double-tap window: timeout 80ms, slop 4px.
const DOUBLE_TAP_AUTH_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": { "taps": { "type": "int", "default": 0 }, "doubles": { "type": "int", "default": 0 } },
  "children": [{
    "type": "rectangle", "id": "btn", "width": 200, "height": 100,
    "gestures": { "doubleTapTimeout": 80, "doubleTapSlop": 4 },
    "events": {
      "onTap": [ { "set": { "$app.taps": "$app.taps + 1" } } ],
      "onDoubleTap": [ { "set": { "$app.doubles": "$app.doubles + 1" } } ]
    }
  }]
}"##;

/// Two children under a parent that owns onDoubleTap (authored wide slop
/// so the two taps are close enough physically while hitting different
/// children).
const SAME_OWNER_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": { "doubles": { "type": "int", "default": 0 } },
  "children": [{
    "type": "frame", "id": "root", "width": 400, "height": 200,
    "gestures": { "doubleTapSlop": 250, "doubleTapTimeout": 300 },
    "events": { "onDoubleTap": [ { "set": { "$app.doubles": "$app.doubles + 1" } } ] },
    "children": [
      { "type": "rectangle", "id": "a", "x": 0, "y": 0, "width": 100, "height": 100 },
      { "type": "rectangle", "id": "b", "x": 200, "y": 0, "width": 100, "height": 100 }
    ]
  }]
}"##;

/// Switch with both onTap and onDoubleTap: the deferred Tap must still
/// perform built-in widget activation when the deadline flushes it.
const SWITCH_DEFER_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": { "on": { "type": "bool", "default": false }, "taps": { "type": "int", "default": 0 } },
  "children": [{
    "type": "switch", "id": "sw", "x": 10, "y": 10, "width": 44, "height": 24,
    "bindings": { "bind:value": "$state.on" },
    "events": {
      "onTap": [ { "set": { "$app.taps": "$app.taps + 1" } } ],
      "onDoubleTap": [ { "set": { "$app.taps": "$app.taps + 0" } } ]
    }
  }]
}"##;

/// interactionOrder is authoring presentation only — a node that declares
/// an order still dispatches its Tap through normal arbitration.
const INTERACTION_ORDER_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": { "taps": { "type": "int", "default": 0 } },
  "children": [{
    "type": "rectangle", "id": "btn", "width": 200, "height": 100,
    "gestures": { "interactionOrder": ["onSwipe", "onTap"] },
    "events": { "onTap": [ { "set": { "$app.taps": "$app.taps + 1" } } ] }
  }]
}"##;

/// Responsive variant doc for the input-freeze test: desktop contains the
/// tappable button + text field; mobile is the parked swap target.
const VARIANT_OP: &str = r##"{
  "version": "1.2", "responsive": true,
  "state": {
    "taps": { "type": "int", "default": 0 },
    "swapped": { "type": "int", "default": 0 }
  },
  "children": [
    { "type": "frame", "id": "desktop", "screen": "/", "width": 300, "height": 200,
      "children": [
        { "type": "text_input", "id": "field", "value": "ab", "width": 100, "height": 30 },
        { "type": "rectangle", "id": "btn", "x": 0, "y": 60, "width": 100, "height": 40,
          "events": {
            "onTap": [ { "set": { "$app.taps": "$app.taps + 1" } } ],
            "onDoubleTap": [ { "set": { "$app.swapped": "$app.swapped" } } ]
          } }
      ] },
    { "type": "frame", "id": "mobile", "screen": "/", "breakpoint": { "maxWidth": 480 },
      "children": [ { "type": "text_input", "id": "field", "value": "m", "width": 100, "height": 30 } ] }
  ]
}"##;

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
        buttons: Default::default(),
        modifiers: Default::default(),
        tilt: None,
        t_ms,
    }
}

#[test]
fn press_start_end_tap_sequence() {
    let mut rt = runtime_with(PRESS_OP);
    let c = node_center(&rt, "btn");

    let down = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    assert_eq!(names(&down), ["onPressStart"]);
    assert_eq!(rt.state.app_get("pressed").unwrap().as_i64(), Some(1));

    let up = rt.dispatch_pointer(mouse(1, PointerPhase::Up, c, 80));
    // Unclaimed Up: PressEnd first, then Tap — exact order.
    assert_eq!(names(&up), ["onPressEnd", "onTap"]);
    assert_eq!(rt.state.app_get("pressed").unwrap().as_i64(), Some(0));
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(0));
}

#[test]
fn host_cancel_emits_press_cancel_exactly_once() {
    let mut rt = runtime_with(PRESS_OP);
    let c = node_center(&rt, "btn");

    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    let cancel = rt.dispatch_pointer(mouse(1, PointerPhase::Cancel, c, 40));
    assert_eq!(names(&cancel), ["onPressCancel"], "cancel emits once");
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(0));

    // A second Cancel for the same (already-closed) pointer is a no-op.
    assert!(rt
        .dispatch_pointer(mouse(1, PointerPhase::Cancel, c, 50))
        .is_empty());
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(1));
}

#[test]
fn pan_claim_cancels_press_before_pan_start() {
    let mut rt = runtime_with(PAN_PRESS_OP);
    let c = node_center(&rt, "btn");

    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    let move_ev = rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(c.x + 40.0, c.y), 100));
    // Cancel BEFORE the winning semantic event, exactly once each.
    assert_eq!(names(&move_ev), ["onPressCancel", "onPanStart"]);
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("pans").unwrap().as_i64(), Some(1));

    let update = rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(c.x + 60.0, c.y), 200));
    assert_eq!(names(&update), ["onPanUpdate"]);

    let end = rt.dispatch_pointer(mouse(1, PointerPhase::Up, point(c.x + 60.0, c.y), 300));
    assert_eq!(names(&end), ["onPanEnd"]);
    // The press was already canceled; the Up must not resurrect it.
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(1));
}

#[test]
fn long_press_claim_cancels_press_before_long_press() {
    let mut rt = runtime_with(LONG_PRESS_OP);
    let c = node_center(&rt, "btn");

    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    let tick_ev = rt.tick(120);
    assert_eq!(names(&tick_ev), ["onPressCancel", "onLongPress"]);
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("longs").unwrap().as_i64(), Some(1));

    // The winning LongPress resolved the arena: the Up emits nothing.
    let up = rt.dispatch_pointer(mouse(1, PointerPhase::Up, c, 300));
    assert!(up.is_empty(), "got {up:?}");
}

#[test]
fn scale_claim_cancels_each_active_press_before_scale_start() {
    let mut rt = runtime_with(SCALE_PRESS_OP);
    let c = node_center(&rt, "btn");

    let _ = rt.dispatch_pointer(mouse(0, PointerPhase::Down, point(c.x - 100.0, c.y), 0));
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, point(c.x + 100.0, c.y), 10));
    let pinch = rt.dispatch_pointer(mouse(0, PointerPhase::Move, point(c.x - 200.0, c.y), 20));
    // Each active press (one per pointer) is canceled exactly once, and
    // all cancellations precede ScaleStart.
    assert_eq!(
        names(&pinch),
        ["onPressCancel", "onPressCancel", "onScaleStart"],
        "got {:?}",
        names(&pinch)
    );
    assert_eq!(rt.state.app_get("scales").unwrap().as_i64(), Some(1));
    // No PressEnd after cancel.
    let up0 = rt.dispatch_pointer(mouse(0, PointerPhase::Up, point(c.x - 200.0, c.y), 30));
    assert!(!up0.is_empty(), "ScaleEnd expected");
    assert_eq!(rt.state.app_get("cancelled").unwrap().as_i64(), Some(1));
}

#[test]
fn press_keeps_captured_target_when_release_moves_outside() {
    let mut rt = runtime_with(PRESS_OP);
    let key = rt.document.as_ref().unwrap().tree.get("btn").unwrap();
    let rect = rt.layout.node_rect(key).unwrap();
    let down_at = point(rect.min_x() + 2.0, rect.min_y() + 2.0);
    // Release just outside the right edge (within recognizer slop 8px).
    let up_at = point(rect.max_x() + 2.0, rect.min_y() + 2.0);

    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, down_at, 0));
    let up = rt.dispatch_pointer(mouse(1, PointerPhase::Up, up_at, 80));
    assert_eq!(names(&up), ["onPressEnd", "onTap"]);
    // Both events keep the captured (Down) target.
    for ev in &up {
        assert_eq!(ev.node(), key, "captured target must be kept");
    }
}

#[test]
fn right_mouse_down_emits_only_context_menu() {
    let mut rt = runtime_with(CONTEXT_OP);
    let c = node_center(&rt, "btn");

    let mut down = mouse(1, PointerPhase::Down, c, 0);
    down.buttons = jian_core::gesture::MouseButtons::RIGHT;
    let down_ev = rt.dispatch_pointer(down);
    assert_eq!(
        names(&down_ev),
        ["onContextMenu"],
        "right-button press yields only ContextMenu (no PressStart)"
    );
    let up = rt.dispatch_pointer(mouse(1, PointerPhase::Up, c, 50));
    assert!(
        up.is_empty(),
        "context-menu press closes the sequence: no Tap/PressEnd, got {up:?}"
    );
    assert_eq!(rt.state.app_get("menu").unwrap().as_i64(), Some(1));
}

#[test]
fn pen_right_button_emits_only_context_menu() {
    let mut rt = runtime_with(CONTEXT_OP);
    let c = node_center(&rt, "btn");

    let down = PointerEvent {
        id: PointerId(1),
        kind: PointerKind::Pen,
        phase: PointerPhase::Down,
        position: c,
        pressure: 0.5,
        buttons: jian_core::gesture::MouseButtons::RIGHT,
        modifiers: Default::default(),
        tilt: Some((10.0, 20.0)),
        t_ms: 0,
    };
    let down_ev = rt.dispatch_pointer(down);
    assert_eq!(names(&down_ev), ["onContextMenu"]);
    let up = rt.dispatch_pointer(mouse(1, PointerPhase::Up, c, 50));
    assert!(up.is_empty());
}

#[test]
fn left_mouse_down_does_not_trigger_context_menu() {
    let mut rt = runtime_with(CONTEXT_OP);
    let c = node_center(&rt, "btn");

    let down = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    assert!(
        !down
            .iter()
            .any(|e| matches!(e, SemanticEvent::ContextMenu { .. })),
        "no RIGHT button fact → no ContextMenu, got {:?}",
        names(&down)
    );
}

#[test]
fn touch_long_press_falls_back_to_context_menu() {
    // Touch long-press on a chain WITHOUT onLongPress but WITH
    // onContextMenu: the same long-press deadline emits ContextMenu.
    let mut rt = runtime_with(CONTEXT_OP);
    let c = node_center(&rt, "btn");

    let down = rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Down, c));
    // CONTEXT_OP declares press handlers, so the touch press starts
    // (menu += 100) and is later canceled by the long-press claim.
    assert_eq!(names(&down), ["onPressStart"]);
    let tick_ev = rt.tick(600);
    assert_eq!(names(&tick_ev), ["onPressCancel", "onContextMenu"]);
    // 100 from PressStart + 1 from ContextMenu; PressEnd never fired.
    assert_eq!(rt.state.app_get("menu").unwrap().as_i64(), Some(101));
}

#[test]
fn explicit_on_long_press_wins_over_context_menu() {
    let mut rt = runtime_with(LONG_PRESS_WINS_OP);
    let c = node_center(&rt, "btn");

    let _ = rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Down, c));
    let tick_ev = rt.tick(600);
    assert_eq!(
        names(&tick_ev),
        ["onLongPress"],
        "explicit onLongPress wins; ContextMenu must not also fire"
    );
    assert_eq!(rt.state.app_get("longs").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("menu").unwrap().as_i64(), Some(0));
}

#[test]
fn touch_never_emits_hover_or_poisons_cache() {
    let mut rt = runtime_with(HOVER_OP);
    let c = node_center(&rt, "btn");

    // Touch "hover" is ignored entirely (no events, no cache mutation).
    let touch_hover = rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Hover, c));
    assert!(touch_hover.is_empty(), "got {touch_hover:?}");

    // The first real mouse hover at the same spot is a fresh Enter — the
    // poisoned touch hover must not have turned it into Leave→Enter.
    let mouse_hover = rt.dispatch_pointer(mouse(2, PointerPhase::Hover, c, 0));
    assert_eq!(names(&mouse_hover), ["onHoverEnter"]);
    assert_eq!(rt.state.app_get("enters").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("leaves").unwrap().as_i64(), Some(0));
}

#[test]
fn disabled_events_skips_owner_and_continues_bubbling() {
    let mut rt = runtime_with(DISABLED_EVENTS_OP);
    let c = node_center(&rt, "child");

    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    let up = rt.dispatch_pointer(mouse(1, PointerPhase::Up, c, 80));
    assert_eq!(names(&up), ["onTap"]);
    // Child's handler was skipped; the parent's handler ran instead.
    assert_eq!(rt.state.app_get("child").unwrap().as_i64(), Some(0));
    assert_eq!(rt.state.app_get("parent").unwrap().as_i64(), Some(1));
}

#[test]
fn gestures_disabled_expression_skips_handler_and_continues_bubbling() {
    let mut rt = runtime_with(GESTURES_DISABLED_OP);
    let c = node_center(&rt, "child");

    let tap = |rt: &mut Runtime| {
        let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
        let up = rt.dispatch_pointer(mouse(1, PointerPhase::Up, c, 80));
        assert_eq!(names(&up), ["onTap"]);
    };
    // disabled = false → the child's own handler runs.
    tap(&mut rt);
    assert_eq!(rt.state.app_get("child").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("parent").unwrap().as_i64(), Some(0));

    rt.state.app_set("off", serde_json::json!(true));
    // disabled = true → child skipped, bubbling reaches the parent.
    tap(&mut rt);
    assert_eq!(rt.state.app_get("child").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("parent").unwrap().as_i64(), Some(1));
}

#[test]
fn authored_thresholds_change_behavior() {
    let mut rt = runtime_with(THRESHOLDS_OP);
    let c = node_center(&rt, "btn");

    // 15px move is under the authored dragThreshold (30) — no Pan claim.
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    let small = rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(c.x + 15.0, c.y), 100));
    assert!(small.is_empty(), "got {small:?}");
    // 35px move crosses the authored threshold.
    let big = rt.dispatch_pointer(mouse(1, PointerPhase::Move, point(c.x + 35.0, c.y), 200));
    assert_eq!(names(&big), ["onPanStart"]);
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Up, point(c.x + 35.0, c.y), 300));

    // Authored longPressDuration = 100: 120ms tick fires, 80ms does not.
    let _ = rt.dispatch_pointer(mouse(2, PointerPhase::Down, c, 400));
    assert!(
        rt.tick(480).is_empty(),
        "80ms must be before the 100ms deadline"
    );
    let fires = rt.tick(520);
    assert_eq!(names(&fires), ["onLongPress"]);
}

#[test]
fn authored_double_tap_timeout_flushes_non_matching_second_tap() {
    let mut rt = runtime_with(DOUBLE_TAP_AUTH_OP);
    let c = node_center(&rt, "btn");

    // First tap: up at t=50 → buffered; deadline = 50 + 80 = 130.
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    assert!(rt
        .dispatch_pointer(mouse(1, PointerPhase::Up, c, 50))
        .is_empty());
    // The second tap's Down lands at t=200 — at/after the deadline. The
    // DUE pending is flushed at the BEGINNING of that dispatch (before
    // the new tap is processed): exact-deadline order independence.
    let second_down = rt.dispatch_pointer(mouse(2, PointerPhase::Down, c, 200));
    assert_eq!(
        names(&second_down),
        ["onTap"],
        "a due pending Tap flushes before the first input at/after the deadline, got {second_down:?}"
    );
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(1));

    // The new Tap defers (the chain still declares onDoubleTap).
    let second_up = rt.dispatch_pointer(mouse(2, PointerPhase::Up, c, 250));
    assert!(
        second_up.is_empty(),
        "the second tap re-opens the window, got {second_up:?}"
    );
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(1));

    // The second Tap flushes at its own deadline (250 + 80 = 330).
    assert!(rt.tick(329).is_empty());
    let flush = rt.tick(330);
    assert_eq!(names(&flush), ["onTap"]);
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(2));
}

#[test]
fn authored_double_tap_slop_rejects_distant_second_tap() {
    let mut rt = runtime_with(DOUBLE_TAP_AUTH_OP);
    let c = node_center(&rt, "btn");

    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    assert!(rt
        .dispatch_pointer(mouse(1, PointerPhase::Up, c, 50))
        .is_empty());
    // Second tap 6px away (within tap-recognizer slop 8, beyond authored
    // doubleTapSlop 4, within the 80ms window) → non-matching.
    let _ = rt.dispatch_pointer(mouse(2, PointerPhase::Down, point(c.x + 6.0, c.y), 100));
    let second_up = rt.dispatch_pointer(mouse(2, PointerPhase::Up, point(c.x + 6.0, c.y), 150));
    assert_eq!(names(&second_up), ["onTap"], "got {second_up:?}");
    assert_eq!(rt.state.app_get("doubles").unwrap().as_i64(), Some(0));
}

#[test]
fn two_children_bubble_to_same_double_tap_owner() {
    let mut rt = runtime_with(SAME_OWNER_OP);
    let a = node_center(&rt, "a");
    let b = node_center(&rt, "b");

    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, a, 0));
    assert!(rt
        .dispatch_pointer(mouse(1, PointerPhase::Up, a, 50))
        .is_empty());
    // Different hit child, same logical (owner) target → DoubleTap.
    let _ = rt.dispatch_pointer(mouse(2, PointerPhase::Down, b, 150));
    let second_up = rt.dispatch_pointer(mouse(2, PointerPhase::Up, b, 200));
    assert_eq!(names(&second_up), ["onDoubleTap"], "got {second_up:?}");
    assert_eq!(rt.state.app_get("doubles").unwrap().as_i64(), Some(1));
}

#[test]
fn deferred_tap_still_activates_switch() {
    let mut rt = runtime_with(SWITCH_DEFER_OP);
    let c = node_center(&rt, "sw");

    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    assert!(rt
        .dispatch_pointer(mouse(1, PointerPhase::Up, c, 50))
        .is_empty());
    // Deadline (50+300) flushes the deferred Tap — built-in activation
    // must still toggle the switch and run the onTap handler.
    rt.tick(350);
    assert_eq!(rt.state.app_get("on").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(1));
}

#[test]
fn next_wake_ms_includes_pending_tap_deadline() {
    let mut rt = runtime_with(DOUBLE_TAP_AUTH_OP);
    let c = node_center(&rt, "btn");

    assert_eq!(rt.gestures.next_wake_ms(), None);
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 1_000));
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Up, c, 1_050));
    // deadline = 1050 + 80 = 1130.
    assert_eq!(rt.gestures.next_wake_ms(), Some(1_130));
    rt.tick(1_130);
    assert_eq!(rt.gestures.next_wake_ms(), None);
}

#[test]
fn replace_document_clears_pending_tap() {
    let mut rt = runtime_with(DOUBLE_TAP_AUTH_OP);
    let c = node_center(&rt, "btn");

    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 1_000));
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Up, c, 1_050));
    assert_eq!(rt.gestures.next_wake_ms(), Some(1_130));

    rt.replace_document(
        serde_json::from_str(
            r##"{"version":"0.8.0","children":[
                 {"type":"rectangle","id":"other","width":50,"height":50}
               ]}"##,
        )
        .unwrap(),
    )
    .unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    // Pending was cleared by the document swap; no stale flush/leak.
    assert_eq!(rt.gestures.next_wake_ms(), None);
    assert!(rt.tick(1_130).is_empty());
    // The replacement document declares no `taps` state (and no handler),
    // so nothing was delivered.
    assert_eq!(rt.state.app_get("taps").and_then(|v| v.as_i64()), None);
}

#[test]
fn interaction_order_is_presentation_only() {
    let mut rt = runtime_with(INTERACTION_ORDER_OP);
    let c = node_center(&rt, "btn");
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 0));
    let up = rt.dispatch_pointer(mouse(1, PointerPhase::Up, c, 80));
    assert_eq!(names(&up), ["onTap"]);
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(1));
}

#[test]
fn frozen_input_does_not_drop_expired_pending_tap() {
    // Replicate the variant-swap freeze: a text-input IME handshake parks
    // the swap and `input_frozen()` becomes true. A deferred Tap whose
    // deadline passes during the freeze must still be emitted by `tick`.
    use jian_ops_schema::document::PenDocument;
    use jian_ops_schema::screen_projection::project_screens;

    let source: PenDocument = serde_json::from_str(VARIANT_OP).unwrap();
    let (projected, _) = project_screens(&source);
    let (normalized, variants) = projected.unwrap();
    let desktop = normalized
        .pages
        .as_ref()
        .unwrap()
        .iter()
        .find(|page| page.id == "desktop")
        .unwrap()
        .clone();
    let mut mounted = normalized.clone();
    mounted.pages = Some(vec![desktop]);
    let mut rt = Runtime::new_from_document(mounted).unwrap();
    rt.configure_variant_source(normalized, "/", variants);
    rt.build_layout((300.0, 200.0)).unwrap();
    rt.rebuild_spatial();

    // Buffer a Tap (deadline = 1050 + 300 = 1350).
    let c = node_center(&rt, "btn");
    let _ = rt.dispatch_pointer(mouse(1, PointerPhase::Down, c, 1_000));
    assert!(rt
        .dispatch_pointer(mouse(1, PointerPhase::Up, c, 1_050))
        .is_empty());

    // Freeze: text field carries an active IME composition, and the swap
    // parks on an IME handshake.
    let field = rt.document.as_ref().unwrap().tree.get("field").unwrap();
    let node = rt.document.as_ref().unwrap().tree.nodes[field]
        .schema
        .clone();
    let state = rt
        .widget_states
        .get_or_init(&node, &rt.state)
        .expect("text input state");
    let jian_core::widget_state::WidgetState::TextInput(text) = state else {
        panic!("expected text input state");
    };
    text.set_composition("pending", 7, 0);
    rt.switch_variant("mobile@0-480").unwrap();
    assert!(rt.input_frozen());

    // The expired pending Tap is flushed (not consumed-and-dropped) even
    // while input is frozen.
    let flush = rt.tick(1_350);
    assert_eq!(
        names(&flush),
        ["onTap"],
        "frozen tick must still emit the expired pending Tap, got {flush:?}"
    );
    assert_eq!(rt.state.app_get("taps").unwrap().as_i64(), Some(1));
}
