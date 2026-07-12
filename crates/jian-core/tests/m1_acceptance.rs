use jian_core::expression::Expression;
use jian_core::geometry::{point, Rect};
use jian_core::gesture::{
    Modifiers, MouseButtons, PointerEvent, PointerId, PointerKind, PointerPhase, SemanticEvent,
};
use jian_core::render::{collect_draws_with_state, DrawOp, ImageSource};
use jian_core::widget_state::WidgetState;
use jian_core::Runtime;
use jian_ops_schema::PenDocument;

const HEIGHT: f32 = 720.0;

fn source() -> PenDocument {
    serde_json::from_str(include_str!("fixtures/m1_acceptance.json")).unwrap()
}

fn resize(runtime: &mut Runtime, width: f32, now_ms: u64) {
    if let Some(target) = runtime.needs_variant_swap(width) {
        assert!(runtime.switch_variant(&target).unwrap());
    }
    runtime.build_layout((width, HEIGHT)).unwrap();
    runtime.rebuild_spatial();
    runtime.set_viewport_size((width, HEIGHT));
    runtime.pump(now_ms);
}

fn rect(runtime: &Runtime, id: &str) -> Rect {
    let document = runtime.document.as_ref().unwrap();
    runtime
        .layout
        .node_rect(document.tree.get(id).unwrap())
        .unwrap()
}

fn rendered_texts(runtime: &Runtime) -> Vec<String> {
    collect_draws_with_state(
        runtime.document.as_ref().unwrap(),
        &runtime.layout,
        &runtime.state,
    )
    .into_iter()
    .filter_map(|draw| match draw {
        DrawOp::Text(run) => Some(run.content),
        _ => None,
    })
    .collect()
}

fn assert_variant(
    runtime: &Runtime,
    page_id: &str,
    viewport_width: f32,
    expected_button_x: f32,
    expected_button_width: f32,
) -> f32 {
    assert_eq!(runtime.active_page_key(), page_id);
    let button = rect(runtime, "button");
    assert_eq!(
        (
            button.origin.x,
            button.origin.y,
            button.size.width,
            button.size.height
        ),
        (expected_button_x, 650.0, expected_button_width, 50.0)
    );
    assert_eq!(
        runtime.state.app_get("sharedLabel").unwrap().as_str(),
        Some("continuity")
    );
    assert_eq!(
        Expression::compile("$viewport.width")
            .unwrap()
            .eval(&runtime.state, None, None)
            .0
            .as_f64(),
        Some(viewport_width as f64)
    );
    let texts = rendered_texts(runtime);
    assert!(texts.iter().any(|text| text == "continuity"));
    assert!(texts
        .iter()
        .any(|text| text.parse::<f32>().ok() == Some(viewport_width)));
    assert!(collect_draws_with_state(
        runtime.document.as_ref().unwrap(),
        &runtime.layout,
        &runtime.state,
    )
    .iter()
    .any(
        |draw| matches!(draw, DrawOp::Image { source: ImageSource::Url(url), .. }
        if url == "https://example.invalid/acceptance.png")
    ));
    rect(runtime, "wrapping-text").size.height
}

fn current_input_text(runtime: &mut Runtime) -> String {
    let document = runtime.document.as_ref().unwrap();
    let key = document.tree.get("field").unwrap();
    let node = document.tree.nodes[key].schema.clone();
    match runtime.widget_states.get_or_init(&node, &runtime.state) {
        Some(WidgetState::TextInput(input)) => input.text().to_owned(),
        other => panic!("expected text input state, got {other:?}"),
    }
}

fn focus_and_type(runtime: &mut Runtime, text: &str) {
    let key = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    runtime.focus_request(key).unwrap();
    assert!(runtime.dispatch_text_input(text).unwrap());
}

fn mouse_hover(id: u32, position: jian_core::geometry::Point) -> PointerEvent {
    PointerEvent {
        id: PointerId(id),
        kind: PointerKind::Mouse,
        phase: PointerPhase::Hover,
        position,
        pressure: 0.0,
        buttons: MouseButtons::empty(),
        modifiers: Modifiers::empty(),
        tilt: None,
        t_ms: 0,
    }
}

#[test]
fn desktop_drag_resize_closes_m1_responsive_acceptance() {
    let schema = source();
    let mut runtime = Runtime::new_from_document(schema.clone()).unwrap();
    runtime
        .state
        .app_set("sharedLabel", serde_json::json!("continuity"));

    resize(&mut runtime, 320.0, 1);
    let mobile_text_height = assert_variant(&runtime, "home-mobile@0-480", 320.0, 240.0, 60.0);
    focus_and_type(&mut runtime, "-typed");
    assert_eq!(current_input_text(&mut runtime), "mobile-typed");

    let button = rect(&runtime, "button");
    let hover = runtime.dispatch_pointer(mouse_hover(
        7,
        point(button.min_x() + 1.0, button.min_y() + 1.0),
    ));
    assert!(hover
        .iter()
        .any(|event| matches!(event, SemanticEvent::HoverEnter { .. })));

    resize(&mut runtime, 400.0, 2);
    assert_eq!(runtime.active_page_key(), "home-mobile@0-480");
    assert_eq!(rect(&runtime, "button").origin.x, 320.0);
    assert!(rect(&runtime, "wrapping-text").size.height <= mobile_text_height);
    resize(&mut runtime, 320.0, 3);

    resize(&mut runtime, 768.0, 4);
    let tablet_text_height = assert_variant(&runtime, "home-tablet@480.5-1024", 768.0, 668.0, 80.0);
    assert_eq!(current_input_text(&mut runtime), "tablet");
    assert!(runtime.focus.current().is_none());
    assert_eq!(
        runtime.state.app_get("blurEvents").unwrap().as_i64(),
        Some(0)
    );
    assert_eq!(
        runtime.state.app_get("hoverLeaveEvents").unwrap().as_i64(),
        Some(0)
    );
    let after_reset = runtime.dispatch_pointer(mouse_hover(7, point(-10.0, -10.0)));
    assert!(!after_reset
        .iter()
        .any(|event| matches!(event, SemanticEvent::HoverLeave { .. })));

    resize(&mut runtime, 1280.0, 5);
    let desktop_text_height = assert_variant(&runtime, "home-desktop", 1280.0, 1160.0, 100.0);
    assert_eq!(current_input_text(&mut runtime), "desktop");
    assert!(mobile_text_height > tablet_text_height);
    assert!(tablet_text_height >= desktop_text_height);

    resize(&mut runtime, 320.0, 6);
    assert_variant(&runtime, "home-mobile@0-480", 320.0, 240.0, 60.0);
    assert_eq!(current_input_text(&mut runtime), "mobile-typed");

    runtime.replace_document(schema).unwrap();
    runtime.build_layout((320.0, HEIGHT)).unwrap();
    runtime.rebuild_spatial();
    runtime.set_viewport_size((320.0, HEIGHT));
    assert_eq!(runtime.active_page_key(), "home-mobile@0-480");
    assert_eq!(current_input_text(&mut runtime), "mobile-typed");

    let live_page = runtime.active_page_key().to_owned();
    runtime.inject_staged_variant_build_failure();
    let failure = runtime
        .switch_variant("home-tablet@480.5-1024")
        .unwrap_err();
    assert!(failure
        .to_string()
        .contains("injected staged variant build failure"));
    assert_eq!(runtime.active_page_key(), live_page);
    let button = rect(&runtime, "button");
    let hit = point(button.min_x() + 2.0, button.min_y() + 2.0);
    runtime.dispatch_pointer(PointerEvent::simple(9, PointerPhase::Down, hit));
    runtime.dispatch_pointer(PointerEvent::simple(9, PointerPhase::Up, hit));
    assert_eq!(runtime.state.app_get("clicks").unwrap().as_i64(), Some(1));
}
