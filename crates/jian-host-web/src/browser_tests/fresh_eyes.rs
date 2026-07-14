use crate::clock::HostClock;
use crate::event::EventBridge;
use crate::runtime_slot::RuntimeSlot;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test;

fn element<T: JsCast>(tag: &str) -> T {
    web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element(tag)
        .unwrap()
        .dyn_into()
        .unwrap()
}

#[wasm_bindgen_test]
fn keyboard_bridge_only_handles_events_owned_by_its_canvas_and_restores_tabindex() {
    let mut runtime = jian_core::Runtime::new();
    runtime
        .load_str(
            r#"{"version":"1.2","state":{"keys":{"type":"int","default":0}},"children":[{"type":"text_input","id":"field","width":100,"height":30,"value":"","events":{"onKey":[{"set":{"$app.keys":"$app.keys + 1"}}]}}]}"#,
        )
        .unwrap();
    runtime.build_layout((200.0, 100.0)).unwrap();
    runtime.rebuild_spatial();
    let field = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    runtime.focus_request(field).unwrap();
    let runtime = Rc::new(RuntimeSlot::new(runtime));

    let document = web_sys::window().unwrap().document().unwrap();
    let canvas: web_sys::HtmlCanvasElement = element("canvas");
    canvas.set_attribute("tabindex", "7").unwrap();
    let input: web_sys::HtmlInputElement = element("input");
    let owned_input: web_sys::HtmlInputElement = element("input");
    document.body().unwrap().append_child(&canvas).unwrap();
    document.body().unwrap().append_child(&input).unwrap();
    document.body().unwrap().append_child(&owned_input).unwrap();

    let bridge = EventBridge::attach_with_clock_and_keyboard_target(
        canvas.clone(),
        runtime.clone(),
        HostClock::new().unwrap(),
        Rc::new(|| {}),
        Some(owned_input.clone().into()),
    )
    .unwrap();
    assert_eq!(canvas.tab_index(), 0);

    input.focus().unwrap();
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("Enter");
    init.set_bubbles(true);
    init.set_cancelable(true);
    let unrelated =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    input.dispatch_event(&unrelated).unwrap();
    assert!(!unrelated.default_prevented());
    assert_eq!(
        runtime.borrow().state.app_get("keys").unwrap().as_i64(),
        Some(0)
    );

    canvas.focus().unwrap();
    let owned =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    canvas.dispatch_event(&owned).unwrap();
    assert!(owned.default_prevented());
    assert_eq!(
        runtime.borrow().state.app_get("keys").unwrap().as_i64(),
        Some(1)
    );

    owned_input.focus().unwrap();
    let owned_ime =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    owned_input.dispatch_event(&owned_ime).unwrap();
    assert!(owned_ime.default_prevented());
    assert_eq!(
        runtime.borrow().state.app_get("keys").unwrap().as_i64(),
        Some(2)
    );

    drop(bridge);
    assert_eq!(canvas.get_attribute("tabindex").as_deref(), Some("7"));
    canvas.remove();
    input.remove();
    owned_input.remove();
}
