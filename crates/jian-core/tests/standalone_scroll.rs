use jian_core::geometry::point;
use jian_core::gesture::{PointerEvent, PointerPhase, ScrollMode, WheelEvent};
use jian_core::render::ScenePaintCommand;
use jian_core::Runtime;

const DOC: &str = r##"{"version":"1.2","responsive":true,"state":{"tapped":{"type":"bool","default":false}},"children":[{
    "type":"frame","id":"screen","width":120,"height":100,"children":[
        {"type":"rectangle","id":"target","x":8,"y":170,"width":36,"height":20,
         "fill":[{"type":"solid","color":"#ff0000"}],
         "events":{"onTap":[{"set":{"$app.tapped":"true"}}]},
         "animations":[{"trigger":"inView","durationMs":160,"easing":"linear",
           "keyframes":{"from":{"translateX":0},"to":{"translateX":40}}}]},
        {"type":"frame","id":"cropped","x":0,"y":0,"width":120,"height":20,
         "clipContent":true,"children":[
            {"type":"rectangle","id":"hidden_overflow","x":0,"y":999,"width":20,"height":10}
         ]}
    ]
}]}"##;

fn runtime() -> Runtime {
    let mut runtime = Runtime::new();
    runtime.enable_standalone_scroll();
    runtime.set_viewport_size_without_relayout((120.0, 100.0));
    runtime.load_str(DOC).unwrap();
    runtime.build_layout((120.0, 100.0)).unwrap();
    runtime.pump(0);
    runtime.frame_presented();
    runtime
}

fn wheel(y: f32, mode: ScrollMode, now: u64) -> WheelEvent {
    WheelEvent {
        mode,
        t_ms: now,
        ..WheelEvent::simple(point(20., 30.), point(0., y))
    }
}

#[test]
fn real_wheel_uses_visible_overflow_and_starts_motion_at_the_event_clock() {
    let mut runtime = runtime();
    assert_eq!(runtime.next_wake_ms(), None);
    runtime.dispatch_standalone_wheel(wheel(-5.0, ScrollMode::Line, 100));
    assert_eq!(runtime.standalone_scroll_y(), 80.0);
    assert_eq!(runtime.standalone_motion_deadline_ms(), Some(116));
    assert!(runtime.pump(180).needs_paint);
    assert!(runtime
        .standalone_motion_overrides()
        .values()
        .any(|value| value.as_f64() == Some(20.0)));
    assert!(matches!(
        runtime.standalone_paint_commands().first(),
        Some(ScenePaintCommand::PushTransform(t)) if t.m32 == -80.0
    ));
    assert!(runtime.pump(260).needs_paint);
    runtime.dispatch_standalone_wheel(wheel(-1.0, ScrollMode::Page, 270));
    assert_eq!(
        runtime.standalone_scroll_y(),
        90.0,
        "clipped overflow contributes no extent"
    );

    for phase in [PointerPhase::Down, PointerPhase::Up] {
        runtime.dispatch_standalone_pointer(PointerEvent::simple_at(
            1,
            phase,
            point(22., 87.),
            300,
        ));
    }
    assert_eq!(
        runtime
            .state
            .app_get("tapped")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn viewport_resize_clamps_and_document_reload_resets_scroll() {
    let mut runtime = runtime();
    runtime.dispatch_standalone_wheel(wheel(-1000.0, ScrollMode::Pixel, 20));
    assert_eq!(runtime.standalone_scroll_y(), 90.0);
    runtime.set_viewport_size((120.0, 152.0));
    assert_eq!(runtime.standalone_scroll_y(), 38.0);
    runtime.load_str_and_relayout(DOC).unwrap();
    assert_eq!(runtime.standalone_scroll_y(), 0.0);
}
