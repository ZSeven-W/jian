//! End-to-end: winit-style MouseInput events → translator → Runtime
//! → tap on the demo's nested rectangle → state increments.
//!
//! Replicates the running `jian player` pipeline as closely as a
//! headless test can: same translator instance, same scale-factor
//! division, same `dispatch_pointer` call. If this test passes but
//! the on-screen demo doesn't increment, the gap is in winit event
//! delivery on macOS, not in the runtime/translator code.

use jian_core::Runtime;
use jian_host_desktop::pointer::PointerTranslator;
use winit::dpi::PhysicalPosition;
use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};

const COUNTER_OP: &str = r##"{
  "formatVersion": "1.0",
  "version": "1.0.0",
  "state": { "count": { "type": "int", "default": 0 } },
  "children": [
    {
      "type": "frame", "id": "root", "width": 480, "height": 320, "x": 0, "y": 0,
      "children": [
        {
          "type": "rectangle", "id": "btn",
          "x": 140, "y": 180, "width": 200, "height": 64,
          "fill": [{ "type": "solid", "color": "#1e88e5" }],
          "events": { "onTap": [ { "set": { "$app.count": "$app.count + 1" } } ] },
          "children": [
            { "type": "text", "id": "btn-label",
              "x": 0, "y": 20, "width": 200, "height": 24,
              "content": "Click me  +1", "fontSize": 18,
              "fill": [{ "type": "solid", "color": "#ffffff" }] }
          ]
        }
      ]
    }
  ]
}"##;

#[test]
fn winit_click_increments_count_through_full_pipeline() {
    let mut rt =
        Runtime::new_from_document(jian_ops_schema::load_str(COUNTER_OP).unwrap().value).unwrap();
    rt.build_layout((480.0, 320.0)).unwrap();
    rt.rebuild_spatial();

    let mut translator = PointerTranslator::new();
    let device = DeviceId::dummy();

    // Simulate a 1.0x display (no scaling) so logical == physical.
    let scale = 1.0_f32;

    // Center of the button: x = 140 + 100 = 240, y = 180 + 32 = 212.
    let click_phys = PhysicalPosition::new(240.0, 212.0);

    // 1) cursor moves to button (winit fires CursorMoved before press).
    if let Some(mut pe) = translator.translate(&WindowEvent::CursorMoved {
        device_id: device,
        position: click_phys,
    }) {
        pe.position = jian_core::geometry::point(pe.position.x / scale, pe.position.y / scale);
        rt.dispatch_pointer(pe);
    }

    // 2) mouse button pressed.
    let press = WindowEvent::MouseInput {
        device_id: device,
        state: ElementState::Pressed,
        button: MouseButton::Left,
    };
    if let Some(mut pe) = translator.translate(&press) {
        pe.position = jian_core::geometry::point(pe.position.x / scale, pe.position.y / scale);
        rt.dispatch_pointer(pe);
    }

    // 3) mouse button released.
    let release = WindowEvent::MouseInput {
        device_id: device,
        state: ElementState::Released,
        button: MouseButton::Left,
    };
    if let Some(mut pe) = translator.translate(&release) {
        pe.position = jian_core::geometry::point(pe.position.x / scale, pe.position.y / scale);
        rt.dispatch_pointer(pe);
    }

    assert_eq!(
        rt.state.app_get("count").and_then(|v| v.as_i64()),
        Some(1),
        "winit MouseInput pipeline must increment $app.count exactly once"
    );
}
