use crate::tests::ensure_canvaskit;
use crate::{mount_jian, JianHandle};
use js_sys::{Function, Object, Promise, Reflect};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use wasm_bindgen_test::wasm_bindgen_test;

fn canvas() -> web_sys::HtmlCanvasElement {
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(200);
    canvas.set_height(100);
    canvas.style().set_property("width", "200px").unwrap();
    canvas.style().set_property("height", "100px").unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();
    canvas
}

fn options(callback_name: &str, callback: &Function) -> JsValue {
    let options = Object::new();
    Reflect::set(
        &options,
        &JsValue::from_str("canvasKitUrl"),
        &JsValue::from_str("/assets/canvaskit/"),
    )
    .unwrap();
    Reflect::set(&options, &JsValue::from_str(callback_name), callback).unwrap();
    options.into()
}

async fn wait(ms: i32) {
    let promise = Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

#[wasm_bindgen_test(async)]
async fn warning_and_error_callbacks_can_reenter_handle_without_refcell_panics() {
    ensure_canvaskit();

    let warning_canvas = canvas();
    let warning_handle = Rc::new(RefCell::new(None::<JianHandle>));
    let warning_called = Rc::new(Cell::new(false));
    let warning_source = Rc::new(RefCell::new(None::<String>));
    let replacement = Rc::new(RefCell::new(None::<Promise>));
    let callback = {
        let handle = warning_handle.clone();
        let called = warning_called.clone();
        let replacement = replacement.clone();
        let warning_source = warning_source.clone();
        Closure::wrap(Box::new(move |payload: JsValue| {
            *warning_source.borrow_mut() = Reflect::get(&payload, &JsValue::from_str("source"))
                .ok()
                .and_then(|source| source.as_string());
            if called.replace(true) {
                return;
            }
            let promise = handle.borrow().as_ref().map(|handle| {
                handle.set_document(JsValue::from_str(
                    r#"{"version":"1.2","responsive":true,"children":[{"type":"frame","id":"clean","width":"fill_container","height":"fill_container"}]}"#,
                ))
            });
            *replacement.borrow_mut() = promise;
        }) as Box<dyn FnMut(JsValue)>)
    };
    let handle = mount_jian(
        warning_canvas.clone(),
        JsValue::from_str(
            r#"{"version":"1.2","responsive":true,"children":[{"type":"frame","id":"first"},{"type":"frame","id":"extra"}]}"#,
        ),
        options("onWarning", callback.as_ref().unchecked_ref()),
    )
    .await
    .unwrap();
    *warning_handle.borrow_mut() = Some(handle);
    wait(100).await;
    assert!(warning_called.get());
    assert_eq!(warning_source.borrow().as_deref(), Some("runtime"));
    let promise = replacement
        .borrow_mut()
        .take()
        .expect("warning callback scheduled setDocument");
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    warning_handle.borrow().as_ref().unwrap().dispose();
    warning_canvas.remove();

    let error_canvas = canvas();
    let error_handle = Rc::new(RefCell::new(None::<JianHandle>));
    let error_called = Rc::new(Cell::new(0_u32));
    let callback = {
        let handle = error_handle.clone();
        let called = error_called.clone();
        Closure::wrap(Box::new(move |_payload: JsValue| {
            called.set(called.get() + 1);
            if let Some(handle) = handle.borrow().as_ref() {
                handle.dispose();
            }
        }) as Box<dyn FnMut(JsValue)>)
    };
    let handle = mount_jian(
        error_canvas.clone(),
        JsValue::from_str(
            r#"{"version":"1.2","children":[{"type":"rectangle","id":"bad","width":100,"height":80,"events":{"onTap":[{"not_registered":null}]}}]}"#,
        ),
        options("onError", callback.as_ref().unchecked_ref()),
    )
    .await
    .unwrap();
    *error_handle.borrow_mut() = Some(handle);
    let bounds = error_canvas.get_bounding_client_rect();
    // Queue two synchronous action errors before the microtask-driven pump
    // reports them. Disposing from the first callback must suppress the rest
    // of the already-drained batch.
    for pointer_id in [1, 2] {
        for kind in ["pointerdown", "pointerup"] {
            let init = web_sys::PointerEventInit::new();
            init.set_pointer_id(pointer_id);
            init.set_pointer_type("mouse");
            init.set_client_x((bounds.left() + 5.0) as i32);
            init.set_client_y((bounds.top() + 5.0) as i32);
            init.set_buttons(if kind == "pointerdown" { 1 } else { 0 });
            let event = web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
            error_canvas.dispatch_event(&event).unwrap();
        }
    }
    wait(100).await;
    assert_eq!(error_called.get(), 1);
    assert!(error_handle.borrow().as_ref().unwrap().test_disposed());
    error_canvas.remove();
}

#[wasm_bindgen_test(async)]
async fn reload_abort_listener_can_dispose_without_refcell_panics() {
    ensure_canvaskit();
    let global = js_sys::global();
    let original_fetch = Reflect::get(&global, &JsValue::from_str("fetch")).unwrap();
    let handle_slot = Rc::new(RefCell::new(None::<JianHandle>));
    let abort_called = Rc::new(Cell::new(false));
    let on_abort = {
        let handle_slot = handle_slot.clone();
        let abort_called = abort_called.clone();
        Closure::wrap(Box::new(move || {
            abort_called.set(true);
            if let Some(handle) = handle_slot.borrow().as_ref() {
                handle.dispose();
            }
        }) as Box<dyn FnMut()>)
    };
    Reflect::set(
        &global,
        &JsValue::from_str("__jianReloadAbort"),
        on_abort.as_ref().unchecked_ref(),
    )
    .unwrap();
    Reflect::set(
        &global,
        &JsValue::from_str("fetch"),
        Function::new_with_args(
            "request",
            "return new Promise((resolve,reject) => request.signal.addEventListener(\
             'abort', () => { globalThis.__jianReloadAbort(); \
             reject(new DOMException('aborted','AbortError')); }, {once:true}));",
        )
        .as_ref(),
    )
    .unwrap();

    let canvas = canvas();
    let handle = mount_jian(
        canvas.clone(),
        JsValue::from_str(
            r#"{"version":"1.2","app":{"name":"t","version":"1","id":"t","capabilities":["network"]},
            "children":[{"type":"rectangle","id":"button","width":30,"height":30,
            "events":{"onTap":[{"fetch":{"url":"'https://example.test/never'","timeout_ms":60000}}]}}]}"#,
        ),
        options("onWarning", Function::new_no_args("").as_ref()),
    )
    .await
    .unwrap();
    *handle_slot.borrow_mut() = Some(handle);
    let bounds = canvas.get_bounding_client_rect();
    for kind in ["pointerdown", "pointerup"] {
        let init = web_sys::PointerEventInit::new();
        init.set_pointer_id(1);
        init.set_pointer_type("mouse");
        init.set_client_x((bounds.left() + 5.0) as i32);
        init.set_client_y((bounds.top() + 5.0) as i32);
        init.set_buttons(if kind == "pointerdown" { 1 } else { 0 });
        canvas
            .dispatch_event(&web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap())
            .unwrap();
    }
    wait(20).await;

    let promise = handle_slot
        .borrow()
        .as_ref()
        .unwrap()
        .set_document(JsValue::from_str(
            r#"{"version":"1.2","children":[{"type":"rectangle","id":"next","width":10,"height":10}]}"#,
        ));
    assert!(wasm_bindgen_futures::JsFuture::from(promise).await.is_err());
    assert!(abort_called.get());
    assert!(handle_slot.borrow().as_ref().unwrap().test_disposed());

    Reflect::set(&global, &JsValue::from_str("fetch"), &original_fetch).unwrap();
    Reflect::delete_property(&global, &JsValue::from_str("__jianReloadAbort")).unwrap();
    canvas.remove();
}

#[wasm_bindgen_test(async)]
async fn dispose_defers_abort_until_hidden_ime_listeners_are_removed() {
    ensure_canvaskit();
    let global = js_sys::global();
    let original_fetch = Reflect::get(&global, &JsValue::from_str("fetch")).unwrap();
    Reflect::set(
        &global,
        &JsValue::from_str("fetch"),
        Function::new_with_args(
            "request",
            "return new Promise((resolve,reject) => request.signal.addEventListener(\
             'abort', () => { globalThis.__disposeAbortCalled=true; \
             globalThis.__disposeIme.dispatchEvent(new CompositionEvent('compositionstart')); \
             reject(new DOMException('aborted','AbortError')); }, {once:true}));",
        )
        .as_ref(),
    )
    .unwrap();
    let canvas = canvas();
    let handle = mount_jian(
        canvas.clone(),
        JsValue::from_str(
            r#"{"version":"1.2","app":{"name":"t","version":"1","id":"t","capabilities":["network"]},
            "children":[{"type":"rectangle","id":"button","width":30,"height":30,
            "events":{"onTap":[{"fetch":{"url":"'https://example.test/never'","timeout_ms":60000}}]}}]}"#,
        ),
        options("onWarning", Function::new_no_args("").as_ref()),
    )
    .await
    .unwrap();
    let hidden = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .query_selector("input[aria-hidden='true']")
        .unwrap()
        .expect("hidden IME input");
    Reflect::set(&global, &JsValue::from_str("__disposeIme"), &hidden).unwrap();
    let bounds = canvas.get_bounding_client_rect();
    for kind in ["pointerdown", "pointerup"] {
        let init = web_sys::PointerEventInit::new();
        init.set_pointer_id(1);
        init.set_pointer_type("mouse");
        init.set_client_x((bounds.left() + 5.0) as i32);
        init.set_client_y((bounds.top() + 5.0) as i32);
        init.set_buttons(if kind == "pointerdown" { 1 } else { 0 });
        canvas
            .dispatch_event(&web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap())
            .unwrap();
    }
    wait(20).await;

    handle.dispose();
    assert_eq!(
        Reflect::get(&global, &JsValue::from_str("__disposeAbortCalled"))
            .unwrap()
            .as_bool(),
        Some(true)
    );
    assert!(web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .query_selector("input[aria-hidden='true']")
        .unwrap()
        .is_none());

    Reflect::set(&global, &JsValue::from_str("fetch"), &original_fetch).unwrap();
    Reflect::delete_property(&global, &JsValue::from_str("__disposeIme")).unwrap();
    Reflect::delete_property(&global, &JsValue::from_str("__disposeAbortCalled")).unwrap();
    canvas.remove();
}

#[wasm_bindgen_test(async)]
async fn synchronous_fetch_reentry_queues_dom_event_until_runtime_returns() {
    ensure_canvaskit();
    let global = js_sys::global();
    let original_fetch = Reflect::get(&global, &JsValue::from_str("fetch")).unwrap();
    let canvas = canvas();
    Reflect::set(&global, &JsValue::from_str("__syncFetchCanvas"), &canvas).unwrap();
    let bounds = canvas.get_bounding_client_rect();
    Reflect::set(
        &global,
        &JsValue::from_str("__syncFetchX"),
        &JsValue::from_f64(bounds.left() + 5.0),
    )
    .unwrap();
    Reflect::set(
        &global,
        &JsValue::from_str("__syncFetchY"),
        &JsValue::from_f64(bounds.top() + 5.0),
    )
    .unwrap();
    Reflect::set(
        &global,
        &JsValue::from_str("fetch"),
        Function::new_with_args(
            "request",
            "globalThis.__syncFetchCanvas.dispatchEvent(new WheelEvent('wheel', \
             {clientX:globalThis.__syncFetchX,clientY:globalThis.__syncFetchY,deltaY:1,cancelable:true})); \
             return Promise.reject(new TypeError('synthetic network failure'));",
        )
        .as_ref(),
    )
    .unwrap();
    let handle = mount_jian(
        canvas.clone(),
        JsValue::from_str(
            r#"{"version":"1.2","app":{"name":"t","version":"1","id":"t","capabilities":["network"]},
            "state":{"scrolled":{"type":"bool","default":false},"failed":{"type":"bool","default":false}},
            "children":[{"type":"rectangle","id":"button","width":30,"height":30,
            "events":{"onScroll":[{"set":{"$app.scrolled":"true"}}],
            "onTap":[{"fetch":{"url":"'https://example.test/fail'","on_error":[{"set":{"$app.failed":"true"}}]}}]}}]}"#,
        ),
        options("onWarning", Function::new_no_args("").as_ref()),
    )
    .await
    .unwrap();
    for kind in ["pointerdown", "pointerup"] {
        let init = web_sys::PointerEventInit::new();
        init.set_pointer_id(1);
        init.set_pointer_type("mouse");
        init.set_client_x((bounds.left() + 5.0) as i32);
        init.set_client_y((bounds.top() + 5.0) as i32);
        init.set_buttons(if kind == "pointerdown" { 1 } else { 0 });
        canvas
            .dispatch_event(&web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap())
            .unwrap();
    }
    wait(100).await;
    let runtime = handle.test_runtime().unwrap();
    assert_eq!(
        runtime
            .borrow()
            .state
            .app_get("scrolled")
            .unwrap()
            .as_bool(),
        Some(true)
    );
    assert_eq!(
        runtime.borrow().state.app_get("failed").unwrap().as_bool(),
        Some(true)
    );

    handle.dispose();
    Reflect::set(&global, &JsValue::from_str("fetch"), &original_fetch).unwrap();
    Reflect::delete_property(&global, &JsValue::from_str("__syncFetchCanvas")).unwrap();
    Reflect::delete_property(&global, &JsValue::from_str("__syncFetchX")).unwrap();
    Reflect::delete_property(&global, &JsValue::from_str("__syncFetchY")).unwrap();
    canvas.remove();
}

#[wasm_bindgen_test(async)]
async fn image_over_cap_abort_reentry_queues_dom_event() {
    ensure_canvaskit();
    let global = js_sys::global();
    let original_fetch = Reflect::get(&global, &JsValue::from_str("fetch")).unwrap();
    let canvas = canvas();
    Reflect::set(&global, &JsValue::from_str("__overCapCanvas"), &canvas).unwrap();
    let bounds = canvas.get_bounding_client_rect();
    Reflect::set(
        &global,
        &JsValue::from_str("__overCapX"),
        &JsValue::from_f64(bounds.left() + 5.0),
    )
    .unwrap();
    Reflect::set(
        &global,
        &JsValue::from_str("__overCapY"),
        &JsValue::from_f64(bounds.top() + 5.0),
    )
    .unwrap();
    Reflect::set(
        &global,
        &JsValue::from_str("fetch"),
        Function::new_with_args(
            "request",
            "request.signal.addEventListener('abort', () => { \
             globalThis.__overCapAborted=true; \
             globalThis.__overCapCanvas.dispatchEvent(new WheelEvent('wheel', \
             {clientX:globalThis.__overCapX,clientY:globalThis.__overCapY,deltaY:1,cancelable:true})); }, {once:true}); \
             return Promise.resolve(new Response(new Uint8Array([1]), \
             {headers:{'Content-Length':'67108865'}}));",
        )
        .as_ref(),
    )
    .unwrap();
    let handle = mount_jian(
        canvas.clone(),
        JsValue::from_str(
            r#"{"version":"1.2","responsive":true,"app":{"name":"t","version":"1","id":"t","capabilities":["network"]},"state":{"scrolled":{"type":"bool","default":false}},"children":[{"type":"frame","id":"root","width":200,"height":100,"events":{"onScroll":[{"set":{"$app.scrolled":"true"}}]},"children":[{"type":"image","id":"hero","src":"https://example.test/huge.png","width":20,"height":20}]}]}"#,
        ),
        options("onWarning", Function::new_no_args("").as_ref()),
    )
    .await
    .unwrap();
    wait(120).await;
    assert_eq!(
        Reflect::get(&global, &JsValue::from_str("__overCapAborted"))
            .unwrap()
            .as_bool(),
        Some(true)
    );
    assert_eq!(
        handle
            .test_runtime()
            .unwrap()
            .borrow()
            .state
            .app_get("scrolled")
            .unwrap()
            .as_bool(),
        Some(true)
    );

    handle.dispose();
    Reflect::set(&global, &JsValue::from_str("fetch"), &original_fetch).unwrap();
    Reflect::delete_property(&global, &JsValue::from_str("__overCapCanvas")).unwrap();
    Reflect::delete_property(&global, &JsValue::from_str("__overCapX")).unwrap();
    Reflect::delete_property(&global, &JsValue::from_str("__overCapY")).unwrap();
    Reflect::delete_property(&global, &JsValue::from_str("__overCapAborted")).unwrap();
    canvas.remove();
}
