use crate::tests::ensure_canvaskit;
use jian_core::action::services::RouteState;
use js_sys::{Function, Object, Promise, Reflect};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test;

fn canvas() -> web_sys::HtmlCanvasElement {
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(240);
    canvas.set_height(120);
    canvas.style().set_property("width", "240px").unwrap();
    canvas.style().set_property("height", "120px").unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();
    canvas
}

pub(crate) async fn wait(ms: i32) {
    let promise = Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    JsFuture::from(promise).await.unwrap();
}

fn tap(canvas: &web_sys::HtmlCanvasElement, x: f64, y: f64) {
    let bounds = canvas.get_bounding_client_rect();
    for kind in ["pointerdown", "pointerup"] {
        let init = web_sys::PointerEventInit::new();
        init.set_pointer_id(1);
        init.set_pointer_type("mouse");
        init.set_client_x((bounds.left() + x) as i32);
        init.set_client_y((bounds.top() + y) as i32);
        init.set_buttons(if kind == "pointerdown" { 1 } else { 0 });
        canvas
            .dispatch_event(&web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap())
            .unwrap();
    }
}

fn routed_document(label: &str) -> String {
    format!(
        r#"{{
          "version":"1.0",
          "routes":{{"entry":"/","routes":{{"/":{{"pageId":"home"}},"/detail":{{"pageId":"detail"}}}}}},
          "pages":[
            {{"id":"home","name":"Home","children":[
              {{"type":"frame","id":"home-root","width":240,"height":120,"screen":"/","children":[
                {{"type":"rectangle","id":"go","width":80,"height":40,
                 "events":{{"onTap":[{{"push":"\"/detail\""}}]}}}}
              ]}}]}},
            {{"id":"detail","name":"Detail {label}","children":[
              {{"type":"frame","id":"detail-root","width":240,"height":120,"screen":"/detail"}}
            ]}}
          ]}}"#
    )
}

#[wasm_bindgen_test(async)]
async fn mounted_authored_navigation_uses_router_and_reload_preserves_route_state() {
    ensure_canvaskit();
    let canvas = canvas();
    let handle = crate::mount_jian(
        canvas.clone(),
        JsValue::from_str(&routed_document("before")),
        JsValue::UNDEFINED,
    )
    .await
    .unwrap();
    wait(40).await;

    let runtime = handle.test_runtime().unwrap();
    runtime.borrow().nav.restore(
        RouteState {
            path: "/".to_owned(),
            params: BTreeMap::from([("id".to_owned(), "42".to_owned())]),
            query: BTreeMap::from([("tab".to_owned(), "info".to_owned())]),
            stack: vec!["/".to_owned()],
        },
        &["/".to_owned(), "/detail".to_owned()],
    );

    tap(&canvas, 5.0, 5.0);
    wait(80).await;
    assert_eq!(runtime.borrow().active_screen_path(), Some("/detail"));
    let before = runtime.borrow().nav.current();
    assert_eq!(before.stack, ["/", "/detail"]);
    assert_eq!(before.params.get("id").map(String::as_str), Some("42"));
    assert_eq!(before.query.get("tab").map(String::as_str), Some("info"));

    JsFuture::from(handle.set_document(JsValue::from_str(&routed_document("after"))))
        .await
        .unwrap();
    wait(40).await;
    let after = runtime.borrow().nav.current();
    assert_eq!(after, before);
    {
        let runtime = runtime.borrow();
        let key = runtime
            .document
            .as_ref()
            .unwrap()
            .tree
            .get("detail-root")
            .unwrap();
        assert!(runtime.node_scene_rect(key).is_some());
    }

    handle.dispose();
    canvas.remove();
}

#[wasm_bindgen_test(async)]
async fn mounted_authored_clipboard_rejection_reports_action_and_aborts_chain() {
    ensure_canvaskit();
    let navigator = web_sys::window().unwrap().navigator();
    let original = Reflect::get(navigator.as_ref(), &JsValue::from_str("clipboard")).unwrap();
    let rejected_clipboard = Object::new();
    Reflect::set(
        &rejected_clipboard,
        &JsValue::from_str("writeText"),
        Function::new_no_args(
            "return new Promise((resolve,reject) => setTimeout(() => reject(new DOMException('denied','NotAllowedError')), 10));",
        )
        .as_ref(),
    )
    .unwrap();
    Function::new_with_args(
        "clipboard",
        "Object.defineProperty(navigator, 'clipboard', {configurable:true, value:clipboard});",
    )
    .call1(&JsValue::UNDEFINED, rejected_clipboard.as_ref())
    .unwrap();

    let payload = Rc::new(RefCell::new(None::<JsValue>));
    let on_error = {
        let payload = payload.clone();
        Closure::wrap(Box::new(move |value: JsValue| {
            *payload.borrow_mut() = Some(value);
        }) as Box<dyn FnMut(JsValue)>)
    };
    let options = Object::new();
    Reflect::set(
        &options,
        &JsValue::from_str("onError"),
        on_error.as_ref().unchecked_ref(),
    )
    .unwrap();
    let canvas = canvas();
    let handle = crate::mount_jian(
        canvas.clone(),
        JsValue::from_str(
            r#"{"version":"1.2","app":{"name":"clip","version":"1","id":"clip","capabilities":["clipboard"]},"state":{"continued":{"type":"bool","default":false}},"children":[{"type":"rectangle","id":"copy","width":80,"height":40,"events":{"onTap":[{"copy":"'secret'"},{"set":{"$app.continued":"true"}}]}}]}"#,
        ),
        options.into(),
    )
    .await
    .unwrap();
    wait(40).await;
    tap(&canvas, 5.0, 5.0);
    wait(100).await;

    assert!(!handle
        .test_runtime()
        .unwrap()
        .borrow()
        .state
        .app_get("continued")
        .unwrap()
        .as_bool()
        .unwrap());
    let payload = payload.borrow().clone().expect("clipboard error reported");
    assert_eq!(
        Reflect::get(&payload, &JsValue::from_str("kind"))
            .unwrap()
            .as_string()
            .as_deref(),
        Some("action")
    );
    assert_eq!(
        Reflect::get(&payload, &JsValue::from_str("source"))
            .unwrap()
            .as_string()
            .as_deref(),
        Some("onTap")
    );

    handle.dispose();
    canvas.remove();
    Function::new_with_args(
        "clipboard",
        "Object.defineProperty(navigator, 'clipboard', {configurable:true, value:clipboard});",
    )
    .call1(&JsValue::UNDEFINED, &original)
    .unwrap();
}
