//! R2A `$event` payload snapshots: exact JSON for pointer facts (global +
//! node-local coordinates, pointer id/type, phase, provable button,
//! pressure, modifiers, tilt, timestamp) and gesture facts (Pan
//! start/current/delta/translation/velocity, Scale/Rotate absolute,
//! delta, and focal), including initiating-button continuity (Tap /
//! Press / Pan keep the Down's provable button) and Mouse/Pen hover
//! payloads. Payloads are built by the ONE path used by
//! `runtime/async_runtime.rs` — `SemanticEventEnvelope::payload` — driven
//! here through `Runtime::dispatch_pointer_events` so the snapshots cover
//! the real pipe.

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

/// Compute the payload exactly like the runtime does: the handler node's
/// layout-rect origin feeds `local`.
fn payload_json(rt: &mut Runtime, env: &jian_core::gesture::SemanticEventEnvelope) -> String {
    let handler_node = env.event.node();
    let rect = rt.layout.node_rect(handler_node).unwrap();
    let origin = point(rect.min_x(), rect.min_y());
    serde_json::to_string(&env.payload(Some(origin)).expect("payload")).expect("serialize payload")
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
fn tap_payload_exact_json() {
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "taps": { "type": "int", "default": 0 } },
      "children": [{
        "type": "rectangle", "id": "btn", "width": 200, "height": 100,
        "events": { "onTap": [ { "set": { "$app.taps": "$app.taps + 1" } } ] }
      }]
    }"##,
    );
    let mut rt = rt;
    let c = point(100.0, 50.0);
    let mut down = mouse(1, PointerPhase::Down, c, 100);
    down.buttons = MouseButtons::LEFT;
    down.modifiers = jian_core::gesture::Modifiers::CTRL;
    let _ = rt.dispatch_pointer_events(down);
    let mut up = mouse(1, PointerPhase::Up, c, 150);
    up.modifiers = jian_core::gesture::Modifiers::CTRL;
    let envelopes = rt.dispatch_pointer_events(up);
    assert_eq!(envelopes.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &envelopes[0]),
        r#"{"pointerId":1,"pointerType":"mouse","phase":"up","position":{"x":100.0,"y":50.0},"local":{"x":100.0,"y":50.0},"button":"left","modifiers":["ctrl"],"timestamp":150}"#
    );
}

#[test]
fn press_start_payload_carries_provable_left_button() {
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "pressed": { "type": "int", "default": 0 } },
      "children": [{
        "type": "rectangle", "id": "btn", "width": 200, "height": 100,
        "events": { "onPressStart": [ { "set": { "$app.pressed": "1" } } ],
                    "onPressEnd": [ { "set": { "$app.pressed": "0" } } ] }
      }]
    }"##,
    );
    let mut rt = rt;
    let c = point(100.0, 50.0);
    let mut down = mouse(1, PointerPhase::Down, c, 100);
    down.buttons = MouseButtons::LEFT;
    let envelopes = rt.dispatch_pointer_events(down);
    assert_eq!(envelopes.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &envelopes[0]),
        r#"{"pointerId":1,"pointerType":"mouse","phase":"down","position":{"x":100.0,"y":50.0},"local":{"x":100.0,"y":50.0},"button":"left","buttons":["left"],"modifiers":[],"timestamp":100}"#
    );
}

#[test]
fn local_coordinates_derive_from_handler_rect() {
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "taps": { "type": "int", "default": 0 } },
      "children": [{
        "type": "rectangle", "id": "btn", "x": 40, "y": 60, "width": 200, "height": 100,
        "events": { "onTap": [ { "set": { "$app.taps": "$app.taps + 1" } } ] }
      }]
    }"##,
    );
    let mut rt = rt;
    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, point(140.0, 110.0), 100));
    let envelopes =
        rt.dispatch_pointer_events(mouse(1, PointerPhase::Up, point(140.0, 110.0), 150));
    assert_eq!(envelopes.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &envelopes[0]),
        r#"{"pointerId":1,"pointerType":"mouse","phase":"up","position":{"x":140.0,"y":110.0},"local":{"x":100.0,"y":50.0},"modifiers":[],"timestamp":150}"#
    );
}

/// The initiating Down's provable single button is retained on
/// PressEnd and PressCancel while phase/position/timestamp/buttons come
/// from the triggering event.
#[test]
fn press_end_and_cancel_retain_initiating_left_button() {
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "pressed": { "type": "int", "default": 0 } },
      "children": [{
        "type": "rectangle", "id": "btn", "width": 200, "height": 100,
        "events": { "onPressStart": [ { "set": { "$app.pressed": "1" } } ],
                    "onPressEnd": [ { "set": { "$app.pressed": "0" } } ],
                    "onPressCancel": [ { "set": { "$app.pressed": "0" } } ] }
      }]
    }"##,
    );
    let mut rt = rt;
    let c = point(100.0, 50.0);

    // Normal end: Down (LEFT) then Up with NO buttons held (release).
    let mut down = mouse(1, PointerPhase::Down, c, 100);
    down.buttons = MouseButtons::LEFT;
    let _ = rt.dispatch_pointer_events(down);
    let end = rt.dispatch_pointer_events(mouse(1, PointerPhase::Up, c, 200));
    let press_end = end
        .iter()
        .find(|e| matches!(e.event, jian_core::gesture::SemanticEvent::PressEnd { .. }))
        .expect("PressEnd envelope");
    assert_eq!(
        payload_json(&mut rt, press_end),
        r#"{"pointerId":1,"pointerType":"mouse","phase":"up","position":{"x":100.0,"y":50.0},"local":{"x":100.0,"y":50.0},"button":"left","modifiers":[],"timestamp":200}"#
    );

    // Cancel: Down (LEFT) then a host Cancel.
    let mut down2 = mouse(2, PointerPhase::Down, c, 300);
    down2.buttons = MouseButtons::LEFT;
    let _ = rt.dispatch_pointer_events(down2);
    let cancel = rt.dispatch_pointer_events(mouse(2, PointerPhase::Cancel, c, 400));
    let press_cancel = cancel
        .iter()
        .find(|e| {
            matches!(
                e.event,
                jian_core::gesture::SemanticEvent::PressCancel { .. }
            )
        })
        .expect("PressCancel envelope");
    assert_eq!(
        payload_json(&mut rt, press_cancel),
        r#"{"pointerId":2,"pointerType":"mouse","phase":"cancel","position":{"x":100.0,"y":50.0},"local":{"x":100.0,"y":50.0},"button":"left","modifiers":[],"timestamp":400}"#
    );
}

/// An ambiguous multi-button Down (LEFT|RIGHT) proves no single changed
/// button: the Tap that follows keeps `button` absent.
#[test]
fn ambiguous_multi_button_down_keeps_button_absent_on_tap() {
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "taps": { "type": "int", "default": 0 } },
      "children": [{
        "type": "rectangle", "id": "btn", "width": 200, "height": 100,
        "events": { "onTap": [ { "set": { "$app.taps": "$app.taps + 1" } } ] }
      }]
    }"##,
    );
    let mut rt = rt;
    let c = point(100.0, 50.0);
    let mut down = mouse(1, PointerPhase::Down, c, 100);
    down.buttons = MouseButtons::LEFT | MouseButtons::RIGHT;
    let _ = rt.dispatch_pointer_events(down);
    let up = rt.dispatch_pointer_events(mouse(1, PointerPhase::Up, c, 150));
    assert_eq!(up.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &up[0]),
        r#"{"pointerId":1,"pointerType":"mouse","phase":"up","position":{"x":100.0,"y":50.0},"local":{"x":100.0,"y":50.0},"modifiers":[],"timestamp":150}"#,
        "no guessed button from an ambiguous Down"
    );
}

#[test]
fn absent_facts_are_absent_touch_payload() {
    // Touch facts: pressure is provable, buttons/button are not (empty
    // bitmask) — they must be absent from the JSON, never guessed.
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "taps": { "type": "int", "default": 0 } },
      "children": [{
        "type": "rectangle", "id": "btn", "width": 200, "height": 100,
        "events": { "onTap": [ { "set": { "$app.taps": "$app.taps + 1" } } ] }
      }]
    }"##,
    );
    let mut rt = rt;
    let touch = |phase, t| PointerEvent {
        id: PointerId(7),
        kind: PointerKind::Touch,
        phase,
        position: point(50.0, 25.0),
        pressure: 0.5,
        buttons: Default::default(),
        modifiers: Default::default(),
        tilt: None,
        t_ms: t,
    };
    let _ = rt.dispatch_pointer_events(touch(PointerPhase::Down, 100));
    let envelopes = rt.dispatch_pointer_events(touch(PointerPhase::Up, 150));
    assert_eq!(envelopes.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &envelopes[0]),
        r#"{"pointerId":7,"pointerType":"touch","phase":"up","position":{"x":50.0,"y":25.0},"local":{"x":50.0,"y":25.0},"pressure":0.5,"modifiers":[],"timestamp":150}"#
    );
}

#[test]
fn context_menu_payload_exact_json() {
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "menu": { "type": "int", "default": 0 } },
      "children": [{
        "type": "rectangle", "id": "btn", "width": 200, "height": 100,
        "events": { "onContextMenu": [ { "set": { "$app.menu": "$app.menu + 1" } } ] }
      }]
    }"##,
    );
    let mut rt = rt;
    let mut down = mouse(1, PointerPhase::Down, point(100.0, 50.0), 100);
    down.buttons = MouseButtons::RIGHT;
    down.modifiers = jian_core::gesture::Modifiers::SHIFT;
    let envelopes = rt.dispatch_pointer_events(down);
    assert_eq!(envelopes.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &envelopes[0]),
        r#"{"pointerId":1,"pointerType":"mouse","phase":"down","position":{"x":100.0,"y":50.0},"local":{"x":100.0,"y":50.0},"button":"right","buttons":["right"],"modifiers":["shift"],"timestamp":100}"#
    );
}

#[test]
fn multi_button_down_is_ambiguous_and_emits_no_context_menu() {
    // RIGHT held IS factual, but with LEFT also held the press is
    // ambiguous: a changed button cannot be proven (→ `button` absent)
    // AND no ContextMenu may be emitted — only a Down whose bitmask is
    // EXACTLY RIGHT is a factual right-button ContextMenu. The
    // multi-button Down is neither a context press nor a closed
    // sequence; no guessed changed button is serialized.
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "menu": { "type": "int", "default": 0 } },
      "children": [{
        "type": "rectangle", "id": "btn", "width": 200, "height": 100,
        "events": { "onContextMenu": [ { "set": { "$app.menu": "$app.menu + 1" } } ] }
      }]
    }"##,
    );
    let mut rt = rt;
    let mut down = mouse(1, PointerPhase::Down, point(100.0, 50.0), 100);
    down.buttons = MouseButtons::LEFT | MouseButtons::RIGHT;
    let envelopes = rt.dispatch_pointer_events(down);
    assert!(
        !envelopes.iter().any(|e| matches!(
            e.event,
            jian_core::gesture::SemanticEvent::ContextMenu { .. }
        )),
        "LEFT|RIGHT is ambiguous: must not emit ContextMenu, got {envelopes:?}"
    );
    assert_eq!(
        envelopes.len(),
        0,
        "no press/context event may be guessed from an ambiguous Down"
    );
    assert_eq!(
        rt.state.app_get("menu").unwrap().as_i64(),
        Some(0),
        "the context-menu handler must not have run"
    );
}

#[test]
fn long_press_payload_carries_duration() {
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "longs": { "type": "int", "default": 0 } },
      "children": [{
        "type": "rectangle", "id": "btn", "x": 40, "y": 60, "width": 200, "height": 100,
        "gestures": { "longPressDuration": 250 },
        "events": { "onLongPress": [ { "set": { "$app.longs": "$app.longs + 1" } } ] }
      }]
    }"##,
    );
    let mut rt = rt;
    let down = PointerEvent {
        id: PointerId(4),
        kind: PointerKind::Touch,
        phase: PointerPhase::Down,
        position: point(140.0, 110.0),
        pressure: 0.5,
        buttons: Default::default(),
        modifiers: Default::default(),
        tilt: None,
        t_ms: 1000,
    };
    let _ = rt.dispatch_pointer_events(down);
    // The long-press envelope arrives from the router tick; the payload
    // reflects the press-down facts (no new pointer event at the deadline).
    let envelopes = rt.gestures.tick_enveloped(1250);
    assert_eq!(envelopes.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &envelopes[0]),
        r#"{"pointerId":4,"pointerType":"touch","phase":"down","position":{"x":140.0,"y":110.0},"local":{"x":100.0,"y":50.0},"pressure":0.5,"modifiers":[],"timestamp":1000,"durationMs":250}"#
    );
}

#[test]
fn pan_payloads_exact_json() {
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "pans": { "type": "int", "default": 0 } },
      "children": [{
        "type": "rectangle", "id": "btn", "x": 40, "y": 60, "width": 200, "height": 100,
        "events": {
          "onPanStart": [ { "set": { "$app.pans": "$app.pans + 1" } } ],
          "onPanUpdate": [ { "set": { "$app.pans": "$app.pans + 1" } } ],
          "onPanEnd": [ { "set": { "$app.pans": "$app.pans + 1" } } ]
        }
      }]
    }"##,
    );
    let mut rt = rt;
    let mut down = mouse(2, PointerPhase::Down, point(140.0, 110.0), 0);
    down.buttons = MouseButtons::LEFT;
    let _ = rt.dispatch_pointer_events(down);
    let mut move1 = mouse(2, PointerPhase::Move, point(160.0, 110.0), 125);
    move1.buttons = MouseButtons::LEFT;
    let start = rt.dispatch_pointer_events(move1);
    let mut move2 = mouse(2, PointerPhase::Move, point(200.0, 110.0), 250);
    move2.buttons = MouseButtons::LEFT;
    let update = rt.dispatch_pointer_events(move2);
    let end = rt.dispatch_pointer_events(mouse(2, PointerPhase::Up, point(200.0, 110.0), 375));
    assert_eq!(start.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &start[0]),
        r#"{"pointerId":2,"pointerType":"mouse","phase":"move","position":{"x":160.0,"y":110.0},"local":{"x":120.0,"y":50.0},"button":"left","buttons":["left"],"modifiers":[],"timestamp":125,"start":{"x":140.0,"y":110.0},"current":{"x":160.0,"y":110.0},"delta":{"x":20.0,"y":0.0},"translation":{"x":20.0,"y":0.0},"velocity":{"x":160.0,"y":0.0}}"#
    );
    assert_eq!(update.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &update[0]),
        r#"{"pointerId":2,"pointerType":"mouse","phase":"move","position":{"x":200.0,"y":110.0},"local":{"x":160.0,"y":50.0},"button":"left","buttons":["left"],"modifiers":[],"timestamp":250,"start":{"x":140.0,"y":110.0},"current":{"x":200.0,"y":110.0},"delta":{"x":40.0,"y":0.0},"translation":{"x":60.0,"y":0.0},"velocity":{"x":320.0,"y":0.0}}"#
    );
    assert_eq!(end.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &end[0]),
        r#"{"pointerId":2,"pointerType":"mouse","phase":"up","position":{"x":200.0,"y":110.0},"local":{"x":160.0,"y":50.0},"button":"left","modifiers":[],"timestamp":375,"start":{"x":140.0,"y":110.0},"current":{"x":200.0,"y":110.0},"delta":{"x":0.0,"y":0.0},"translation":{"x":60.0,"y":0.0},"velocity":{"x":0.0,"y":0.0}}"#
    );
}

#[test]
fn scale_payloads_carry_absolute_delta_and_focal() {
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "zoom": { "type": "float", "default": 1.0 } },
      "children": [{
        "type": "rectangle", "id": "canvas", "width": 800, "height": 600,
        "events": {
          "onScaleStart": [ { "set": { "$app.zoom": "$event.scale" } } ],
          "onScaleUpdate": [ { "set": { "$app.zoom": "$event.scale" } } ],
          "onScaleEnd": [ { "set": { "$app.zoom": "$event.scale" } } ]
        }
      }]
    }"##,
    );
    let mut rt = rt;
    let _ = rt.dispatch_pointer_events(mouse(0, PointerPhase::Down, point(200.0, 300.0), 0));
    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, point(400.0, 300.0), 10));
    // dist 200 -> 250: scale 1.25 exactly (deltaScale = scale − 1).
    let started = rt.dispatch_pointer_events(mouse(0, PointerPhase::Move, point(150.0, 300.0), 20));
    assert_eq!(started.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &started[0]),
        r#"{"pointerId":0,"pointerType":"mouse","phase":"move","position":{"x":150.0,"y":300.0},"local":{"x":150.0,"y":300.0},"modifiers":[],"timestamp":20,"scale":1.25,"deltaScale":0.25,"focal":{"x":275.0,"y":300.0}}"#
    );
    // dist 200 -> 300: scale 1.5, per-frame delta 0.25 exactly.
    let updated = rt.dispatch_pointer_events(mouse(0, PointerPhase::Move, point(100.0, 300.0), 30));
    assert_eq!(updated.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &updated[0]),
        r#"{"pointerId":0,"pointerType":"mouse","phase":"move","position":{"x":100.0,"y":300.0},"local":{"x":100.0,"y":300.0},"modifiers":[],"timestamp":30,"scale":1.5,"deltaScale":0.25,"focal":{"x":250.0,"y":300.0}}"#
    );
    let ended = rt.dispatch_pointer_events(mouse(0, PointerPhase::Up, point(100.0, 300.0), 40));
    assert_eq!(ended.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &ended[0]),
        r#"{"pointerId":0,"pointerType":"mouse","phase":"up","position":{"x":100.0,"y":300.0},"local":{"x":100.0,"y":300.0},"modifiers":[],"timestamp":40,"scale":1.5,"focal":{"x":250.0,"y":300.0}}"#
    );
}

#[test]
fn rotate_payloads_carry_radians_rotation_delta_and_focal() {
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "rotation": { "type": "float", "default": 0.0 } },
      "children": [{
        "type": "rectangle", "id": "canvas", "width": 800, "height": 600,
        "events": {
          "onRotateStart": [ { "set": { "$app.rotation": "$event.radians" } } ],
          "onRotateUpdate": [ { "set": { "$app.rotation": "$event.radians" } } ],
          "onRotateEnd": [ { "set": { "$app.rotation": "$event.radians" } } ]
        }
      }]
    }"##,
    );
    let mut rt = rt;
    let _ = rt.dispatch_pointer_events(mouse(0, PointerPhase::Down, point(300.0, 300.0), 0));
    let _ = rt.dispatch_pointer_events(mouse(1, PointerPhase::Down, point(600.0, 300.0), 10));
    let started = rt.dispatch_pointer_events(mouse(1, PointerPhase::Move, point(600.0, 600.0), 20));
    assert_eq!(started.len(), 1);
    // Exact snapshot: the crossing rotation is 45° (π/4) — not a zero
    // placeholder, and deltaRotation = rotation at the start.
    assert_eq!(
        payload_json(&mut rt, &started[0]),
        r#"{"pointerId":1,"pointerType":"mouse","phase":"move","position":{"x":600.0,"y":600.0},"local":{"x":600.0,"y":600.0},"modifiers":[],"timestamp":20,"rotation":0.7853981852531433,"deltaRotation":0.7853981852531433,"focal":{"x":450.0,"y":450.0}}"#
    );
    let value: serde_json::Value =
        serde_json::from_str(&payload_json(&mut rt, &started[0])).unwrap();
    assert_eq!(
        value["rotation"],
        serde_json::json!(std::f32::consts::FRAC_PI_4)
    );
    assert_eq!(
        value["deltaRotation"],
        serde_json::json!(std::f32::consts::FRAC_PI_4)
    );

    let updated = rt.dispatch_pointer_events(mouse(1, PointerPhase::Move, point(600.0, 900.0), 30));
    assert_eq!(updated.len(), 1);
    let payload = payload_json(&mut rt, &updated[0]);
    let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
    let radians = value["radians"].as_f64().unwrap();
    let rotation = value["rotation"].as_f64().unwrap();
    let delta = value["deltaRotation"].as_f64().unwrap();
    // 45° → 90°…600,900 gives atan2(600,300) ≈ 1.107 rad (63.4°).
    assert!((radians - 1.1071).abs() < 1e-3, "got {radians}");
    assert_eq!(rotation, radians, "absolute rotation == radians");
    assert!((delta - (radians - std::f64::consts::FRAC_PI_4)).abs() < 1e-6);
    assert_eq!(value["focal"], serde_json::json!({"x": 450.0, "y": 600.0}));
    // `radians` stays the pre-existing key.
    let payload_text = payload_json(&mut rt, &updated[0]);
    assert!(payload_text.contains("\"radians\":"));

    let ended = rt.dispatch_pointer_events(mouse(1, PointerPhase::Up, point(600.0, 900.0), 40));
    assert_eq!(ended.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &ended[0]),
        r#"{"pointerId":1,"pointerType":"mouse","phase":"up","position":{"x":600.0,"y":900.0},"local":{"x":600.0,"y":900.0},"modifiers":[],"timestamp":40,"rotation":1.1071487665176392,"focal":{"x":450.0,"y":600.0}}"#
    );
}

#[test]
fn pen_payload_carries_tilt_and_pressure() {
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "menu": { "type": "int", "default": 0 } },
      "children": [{
        "type": "rectangle", "id": "btn", "width": 200, "height": 100,
        "events": { "onContextMenu": [ { "set": { "$app.menu": "$app.menu + 1" } } ] }
      }]
    }"##,
    );
    let mut rt = rt;
    let down = PointerEvent {
        id: PointerId(9),
        kind: PointerKind::Pen,
        phase: PointerPhase::Down,
        position: point(100.0, 50.0),
        pressure: 0.5,
        buttons: MouseButtons::RIGHT,
        modifiers: Default::default(),
        tilt: Some((10.0, 20.0)),
        t_ms: 100,
    };
    let envelopes = rt.dispatch_pointer_events(down);
    assert_eq!(envelopes.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &envelopes[0]),
        r#"{"pointerId":9,"pointerType":"pen","phase":"down","position":{"x":100.0,"y":50.0},"local":{"x":100.0,"y":50.0},"pressure":0.5,"button":"right","buttons":["right"],"modifiers":[],"tilt":{"xDegrees":10.0,"yDegrees":20.0},"timestamp":100}"#
    );
}

/// Mouse hover: standard pointer facts (id/type/phase/global/local/
/// modifiers/timestamp) are exposed; pressure and a changed button are
/// not provable for a Hover phase and stay absent.
#[test]
fn mouse_hover_payload_exposes_standard_pointer_facts() {
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "hovers": { "type": "int", "default": 0 }, "leaves": { "type": "int", "default": 0 } },
      "children": [{
        "type": "rectangle", "id": "btn", "x": 40, "y": 60, "width": 200, "height": 100,
        "events": { "onHoverEnter": [ { "set": { "$app.hovers": "$app.hovers + 1" } } ],
                    "onHoverLeave": [ { "set": { "$app.leaves": "$app.leaves + 1" } } ] }
      }]
    }"##,
    );
    let mut rt = rt;
    let mut enter = mouse(5, PointerPhase::Hover, point(140.0, 110.0), 100);
    enter.modifiers = jian_core::gesture::Modifiers::SHIFT;
    let envelopes = rt.dispatch_pointer_events(enter);
    assert_eq!(envelopes.len(), 1);
    assert!(matches!(
        envelopes[0].event,
        jian_core::gesture::SemanticEvent::HoverEnter { .. }
    ));
    assert_eq!(
        payload_json(&mut rt, &envelopes[0]),
        r#"{"pointerId":5,"pointerType":"mouse","phase":"hover","position":{"x":140.0,"y":110.0},"local":{"x":100.0,"y":50.0},"modifiers":["shift"],"timestamp":100}"#
    );

    // Leave to the void: HoverLeave with factual facts.
    let leaves =
        rt.dispatch_pointer_events(mouse(5, PointerPhase::Hover, point(700.0, 500.0), 200));
    assert_eq!(leaves.len(), 1);
    assert!(matches!(
        leaves[0].event,
        jian_core::gesture::SemanticEvent::HoverLeave { .. }
    ));
    assert_eq!(
        payload_json(&mut rt, &leaves[0]),
        r#"{"pointerId":5,"pointerType":"mouse","phase":"hover","position":{"x":700.0,"y":500.0},"local":{"x":660.0,"y":440.0},"modifiers":[],"timestamp":200}"#
    );
}

/// Pen hover additionally proves pressure/tilt and reports held buttons
/// (row `buttons`) — a Hover phase proves no changed `button`.
#[test]
fn pen_hover_payload_carries_tilt_pressure_and_held_buttons() {
    let rt = runtime_with(
        r##"{
      "version": "0.8.0",
      "state": { "hovers": { "type": "int", "default": 0 } },
      "children": [{
        "type": "rectangle", "id": "btn", "x": 40, "y": 60, "width": 200, "height": 100,
        "events": { "onHoverEnter": [ { "set": { "$app.hovers": "$app.hovers + 1" } } ] }
      }]
    }"##,
    );
    let mut rt = rt;
    let enter = PointerEvent {
        id: PointerId(9),
        kind: PointerKind::Pen,
        phase: PointerPhase::Hover,
        position: point(140.0, 110.0),
        pressure: 0.5,
        buttons: MouseButtons::RIGHT,
        modifiers: jian_core::gesture::Modifiers::CTRL,
        tilt: Some((10.0, 20.0)),
        t_ms: 100,
    };
    let envelopes = rt.dispatch_pointer_events(enter);
    assert_eq!(envelopes.len(), 1);
    assert_eq!(
        payload_json(&mut rt, &envelopes[0]),
        r#"{"pointerId":9,"pointerType":"pen","phase":"hover","position":{"x":140.0,"y":110.0},"local":{"x":100.0,"y":50.0},"pressure":0.5,"buttons":["right"],"modifiers":["ctrl"],"tilt":{"xDegrees":10.0,"yDegrees":20.0},"timestamp":100}"#
    );
}
