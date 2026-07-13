use crate::services::network::fetch_bytes_with_limit;
use crate::services::{AbortRegistry, WebServices};
use crate::tests::ensure_canvaskit;
use jian_core::gesture::{PointerEvent, PointerPhase};
use jian_core::render::image_store::ImageState;
use js_sys::{Function, Promise, Reflect, JSON};
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test;

async fn wait(ms: i32) {
    let promise = Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    JsFuture::from(promise).await.unwrap();
}

fn install_fetch(source: &str) -> JsValue {
    let global = js_sys::global();
    let original = Reflect::get(&global, &JsValue::from_str("fetch")).unwrap();
    let replacement = Function::new_with_args("request", source);
    Reflect::set(&global, &JsValue::from_str("fetch"), replacement.as_ref()).unwrap();
    original
}

fn restore_global(name: &str, value: &JsValue) {
    Reflect::set(&js_sys::global(), &JsValue::from_str(name), value).unwrap();
}

fn signal_aborted(name: &str) -> bool {
    let signal = Reflect::get(&js_sys::global(), &JsValue::from_str(name)).unwrap();
    Reflect::get(&signal, &JsValue::from_str("aborted"))
        .unwrap()
        .as_bool()
        .unwrap()
}

#[wasm_bindgen_test(async)]
async fn image_stream_limit_aborts_before_over_cap_materialization() {
    let aborts = AbortRegistry::default();
    let wake: Rc<dyn Fn()> = Rc::new(|| {});

    let original = install_fetch(
        "globalThis.__streamSignal=request.signal; \
         return Promise.resolve(new Response(new Uint8Array([1]), \
           {headers:{'Content-Length':'5'}}));",
    );
    let error = fetch_bytes_with_limit("https://example.test/image", &aborts, &wake, 4)
        .await
        .unwrap_err();
    assert!(error.contains("exceeds 4 bytes"));
    assert!(signal_aborted("__streamSignal"));

    install_fetch(
        "globalThis.__streamSignal=request.signal; let index=0; \
         const chunks=[new Uint8Array([1,2]),new Uint8Array([3,4,5])]; \
         const body=new ReadableStream({pull(controller){ \
           if(index<chunks.length) controller.enqueue(chunks[index++]); else controller.close(); \
         }}); return Promise.resolve(new Response(body));",
    );
    let error = fetch_bytes_with_limit("https://example.test/image", &aborts, &wake, 4)
        .await
        .unwrap_err();
    assert!(error.contains("exceeds 4 bytes"));
    assert!(signal_aborted("__streamSignal"));

    install_fetch(
        "globalThis.__streamSignal=request.signal; let sent=false; \
         const body=new ReadableStream({pull(controller){ \
           if(!sent){ sent=true; controller.enqueue(new Uint8Array([7,8,9])); } \
           else controller.close(); }}); return Promise.resolve(new Response(body));",
    );
    let bytes = fetch_bytes_with_limit("https://example.test/image", &aborts, &wake, 4)
        .await
        .unwrap();
    assert_eq!(bytes, [7, 8, 9]);
    assert!(!signal_aborted("__streamSignal"));
    restore_global("fetch", &original);
}

fn canvas() -> web_sys::HtmlCanvasElement {
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.style().set_property("width", "200px").unwrap();
    canvas.style().set_property("height", "100px").unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();
    canvas
}

#[wasm_bindgen_test(async)]
async fn context_loss_deletes_cached_images_and_restore_rehydrates_them() {
    ensure_canvaskit();
    const RED_PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z6S8AAAAASUVORK5CYII=";
    let host_canvas = canvas();
    let document = format!(
        r#"{{"version":"1.2","responsive":true,"children":[{{"type":"image","id":"hero","src":"{RED_PNG}","width":20,"height":20}}]}}"#
    );
    let handle = crate::mount_jian(
        host_canvas.clone(),
        JsValue::from_str(&document),
        JsValue::UNDEFINED,
    )
    .await
    .unwrap();
    let runtime = handle.test_runtime().unwrap();
    let key = runtime
        .borrow()
        .state
        .image_key(RED_PNG)
        .expect("data URL must have a canonical image key");
    for _ in 0..20 {
        if handle.test_backend_has_image(&key) {
            break;
        }
        wait(10).await;
    }
    assert!(handle.test_backend_has_image(&key));

    let before = handle.test_presented_frames();
    host_canvas
        .dispatch_event(&web_sys::Event::new("webglcontextlost").unwrap())
        .unwrap();
    assert!(
        !handle.test_backend_has_image(&key),
        "context loss must delete stale CanvasKit image handles immediately"
    );
    host_canvas
        .dispatch_event(&web_sys::Event::new("webglcontextrestored").unwrap())
        .unwrap();
    for _ in 0..20 {
        if handle.test_backend_has_image(&key) {
            break;
        }
        wait(10).await;
    }
    assert!(
        handle.test_backend_has_image(&key),
        "the next frame must register retained bytes in the restored backend generation"
    );
    assert!(handle.test_presented_frames() > before);

    handle.dispose();
    host_canvas.remove();
}

#[wasm_bindgen_test(async)]
async fn set_document_transfers_pending_same_key_and_cancels_removed_image() {
    ensure_canvaskit();
    let original = install_fetch(
        "globalThis.__imageCalls=(globalThis.__imageCalls||0)+1; \
         globalThis.__imageSignals=globalThis.__imageSignals||[]; \
         globalThis.__imageSignals.push(request.signal); \
         return new Promise((resolve) => { globalThis.__imageResolve=resolve; });",
    );
    let host_canvas = canvas();
    let options = JSON::parse(r#"{"assetBase":"https://example.test/assets/"}"#).unwrap();
    let handle = crate::mount_jian(
        host_canvas.clone(),
        JsValue::from_str(
            r#"{"version":"1.2","responsive":true,"children":[{"type":"rectangle","id":"empty","width":10,"height":10}]}"#,
        ),
        options,
    )
    .await
    .unwrap();
    let image_document = JsValue::from_str(
        r#"{"version":"1.2","responsive":true,"children":[{"type":"image","id":"hero","src":"same.png","width":10,"height":10}]}"#,
    );
    let first = handle.set_document(image_document);
    wait(40).await;
    assert_eq!(
        Reflect::get(&js_sys::global(), &JsValue::from_str("__imageCalls"))
            .unwrap()
            .as_f64(),
        Some(1.0)
    );
    JsFuture::from(first).await.unwrap();

    let second = handle.set_document(JsValue::from_str(
        r#"{"version":"1.2","responsive":true,"children":[{"type":"image","id":"hero","src":"./same.png","width":10,"height":10}]}"#,
    ));
    JsFuture::from(second).await.unwrap();
    wait(40).await;
    assert_eq!(
        Reflect::get(&js_sys::global(), &JsValue::from_str("__imageCalls"))
            .unwrap()
            .as_f64(),
        Some(1.0)
    );
    let signals = Reflect::get(&js_sys::global(), &JsValue::from_str("__imageSignals")).unwrap();
    let first_signal = Reflect::get(&signals, &JsValue::from_f64(0.0)).unwrap();
    assert!(!Reflect::get(&first_signal, &JsValue::from_str("aborted"))
        .unwrap()
        .as_bool()
        .unwrap());

    let response = Function::new_no_args(
        "const png=Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z6S8AAAAASUVORK5CYII='), c=>c.charCodeAt(0)); return new Response(png);",
    )
    .call0(&JsValue::UNDEFINED)
    .unwrap();
    Reflect::get(&js_sys::global(), &JsValue::from_str("__imageResolve"))
        .unwrap()
        .dyn_into::<Function>()
        .unwrap()
        .call1(&JsValue::UNDEFINED, &response)
        .unwrap();
    wait(50).await;
    assert_eq!(
        Reflect::get(&js_sys::global(), &JsValue::from_str("__imageCalls"))
            .unwrap()
            .as_f64(),
        Some(1.0)
    );
    {
        let runtime = handle.test_runtime().unwrap();
        let runtime = runtime.borrow();
        let key = runtime.state.image_key("./same.png").unwrap();
        assert_eq!(key, "https://example.test/assets/same.png");
        assert!(matches!(
            runtime.image_store.state(&key),
            Some(ImageState::Bytes | ImageState::Registered)
        ));
    }

    handle.dispose();
    host_canvas.remove();
    Reflect::set(
        &js_sys::global(),
        &JsValue::from_str("__imageSignals"),
        &js_sys::Array::new(),
    )
    .unwrap();
    let removed_canvas = canvas();
    let removed_options = JSON::parse(r#"{"assetBase":"https://example.test/assets/"}"#).unwrap();
    let removed_handle = crate::mount_jian(
        removed_canvas.clone(),
        JsValue::from_str(
            r#"{"version":"1.2","responsive":true,"children":[{"type":"image","id":"other","src":"remove.png","width":10,"height":10}]}"#,
        ),
        removed_options,
    )
    .await
    .unwrap();
    wait(40).await;
    let empty = removed_handle.set_document(JsValue::from_str(
        r#"{"version":"1.2","responsive":true,"children":[{"type":"rectangle","id":"empty","width":10,"height":10}]}"#,
    ));
    JsFuture::from(empty).await.unwrap();
    let signals = Reflect::get(&js_sys::global(), &JsValue::from_str("__imageSignals")).unwrap();
    let removed_signal = Reflect::get(&signals, &JsValue::from_f64(0.0)).unwrap();
    assert!(Reflect::get(&removed_signal, &JsValue::from_str("aborted"))
        .unwrap()
        .as_bool()
        .unwrap());

    removed_handle.dispose();
    removed_canvas.remove();
    restore_global("fetch", &original);
}

#[wasm_bindgen_test(async)]
async fn removed_then_readmitted_same_key_ignores_the_stale_browser_request() {
    ensure_canvaskit();
    let original = install_fetch(
        "globalThis.__staleResolvers=globalThis.__staleResolvers||[]; \
         globalThis.__staleSignals=globalThis.__staleSignals||[]; \
         globalThis.__staleSignals.push(request.signal); \
         return new Promise(resolve => globalThis.__staleResolvers.push(resolve));",
    );
    let canvas = canvas();
    let options = JSON::parse(r#"{"assetBase":"https://example.test/assets/"}"#).unwrap();
    let handle = crate::mount_jian(
        canvas.clone(),
        JsValue::from_str(r#"{"version":"1.2","children":[]}"#),
        options,
    )
    .await
    .unwrap();
    let image = || {
        JsValue::from_str(
            r#"{"version":"1.2","children":[{"type":"image","id":"hero","src":"again.png","width":10,"height":10}]}"#,
        )
    };
    JsFuture::from(handle.set_document(image())).await.unwrap();
    wait(30).await;

    JsFuture::from(handle.set_document(JsValue::from_str(r#"{"version":"1.2","children":[]}"#)))
        .await
        .unwrap();
    let signals = Reflect::get(&js_sys::global(), &JsValue::from_str("__staleSignals")).unwrap();
    assert!(Reflect::get(
        &Reflect::get(&signals, &0.into()).unwrap(),
        &"aborted".into()
    )
    .unwrap()
    .as_bool()
    .unwrap());

    JsFuture::from(handle.set_document(image())).await.unwrap();
    wait(30).await;
    let resolvers =
        Reflect::get(&js_sys::global(), &JsValue::from_str("__staleResolvers")).unwrap();
    assert_eq!(js_sys::Array::from(&resolvers).length(), 2);
    let make_response = || {
        Function::new_no_args(
            "const png=Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z6S8AAAAASUVORK5CYII='), c=>c.charCodeAt(0)); return new Response(png);",
        )
        .call0(&JsValue::UNDEFINED)
        .unwrap()
    };
    Reflect::get(&resolvers, &0.into())
        .unwrap()
        .dyn_into::<Function>()
        .unwrap()
        .call1(&JsValue::UNDEFINED, &make_response())
        .unwrap();
    wait(30).await;
    let runtime = handle.test_runtime().unwrap();
    let key = runtime.borrow().state.image_key("again.png").unwrap();
    assert_eq!(
        runtime.borrow().image_store.state(&key),
        Some(ImageState::Pending)
    );

    Reflect::get(&resolvers, &1.into())
        .unwrap()
        .dyn_into::<Function>()
        .unwrap()
        .call1(&JsValue::UNDEFINED, &make_response())
        .unwrap();
    for _ in 0..20 {
        if matches!(
            runtime.borrow().image_store.state(&key),
            Some(ImageState::Bytes | ImageState::Registered)
        ) {
            break;
        }
        wait(10).await;
    }
    assert!(matches!(
        runtime.borrow().image_store.state(&key),
        Some(ImageState::Bytes | ImageState::Registered)
    ));

    handle.dispose();
    canvas.remove();
    restore_global("fetch", &original);
}

#[wasm_bindgen_test(async)]
async fn delayed_fetch_rejection_wakes_idle_pump_and_reports_action_error() {
    ensure_canvaskit();
    let original = install_fetch(
        "return new Promise((resolve,reject) => \
         setTimeout(() => reject(new Error('delayed network failure')), 20));",
    );
    let called = Rc::new(Cell::new(0));
    let on_error = {
        let called = called.clone();
        Closure::wrap(
            Box::new(move |_payload: JsValue| called.set(called.get() + 1))
                as Box<dyn FnMut(JsValue)>,
        )
    };
    let options = js_sys::Object::new();
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
            r#"{"version":"1.2","app":{"name":"t","version":"1","id":"t","capabilities":["network"]},"children":[{"type":"rectangle","id":"button","width":30,"height":30,"events":{"onTap":[{"fetch":{"url":"'https://example.test/fail'"}}]}}]}"#,
        ),
        options.into(),
    )
    .await
    .unwrap();
    wait(40).await;
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
    // This timer only observes. The fetch Promise's settlement observer must
    // wake the production pump after it has gone idle.
    wait(100).await;
    assert_eq!(called.get(), 1);
    handle.dispose();
    canvas.remove();
    restore_global("fetch", &original);
}

#[wasm_bindgen_test]
fn reload_cancellation_aborts_authored_fetch_and_clears_timeout() {
    let global = js_sys::global();
    let original_fetch = install_fetch(
        "globalThis.__authoredSignal=request.signal; \
         return new Promise((resolve,reject) => request.signal.addEventListener( \
           'abort', () => reject(new DOMException('aborted','AbortError')), {once:true}));",
    );
    let original_set_timeout = Reflect::get(&global, &JsValue::from_str("setTimeout")).unwrap();
    let original_clear_timeout = Reflect::get(&global, &JsValue::from_str("clearTimeout")).unwrap();
    Reflect::set(
        &global,
        &JsValue::from_str("setTimeout"),
        Function::new_with_args(
            "callback,delay",
            "globalThis.__armedTimeout=delay; return 4242;",
        )
        .as_ref(),
    )
    .unwrap();
    Reflect::set(
        &global,
        &JsValue::from_str("clearTimeout"),
        Function::new_with_args("id", "globalThis.__clearedTimeout=id;").as_ref(),
    )
    .unwrap();

    let mut runtime = jian_core::Runtime::new();
    runtime
        .load_str(
            r#"{"version":"1.2","app":{"name":"t","version":"1","id":"t","capabilities":["network"]},"children":[{"type":"rectangle","id":"button","width":30,"height":30,"events":{"onTap":[{"fetch":{"url":"'https://example.test/never'","timeout_ms":60000}}]}}]}"#,
        )
        .unwrap();
    runtime.build_layout((100.0, 100.0)).unwrap();
    let services =
        WebServices::install(&mut runtime, None, AbortRegistry::default(), Rc::new(|| {})).unwrap();
    for phase in [PointerPhase::Down, PointerPhase::Up] {
        runtime.dispatch_pointer(PointerEvent::simple(
            1,
            phase,
            jian_core::geometry::point(5.0, 5.0),
        ));
    }
    runtime.pump(1);
    runtime
        .load_str(
            r#"{"version":"1.2","children":[{"type":"rectangle","id":"empty","width":10,"height":10}]}"#,
        )
        .unwrap();

    restore_global("fetch", &original_fetch);
    restore_global("setTimeout", &original_set_timeout);
    restore_global("clearTimeout", &original_clear_timeout);
    assert!(signal_aborted("__authoredSignal"));
    assert_eq!(
        Reflect::get(&global, &JsValue::from_str("__armedTimeout"))
            .unwrap()
            .as_f64(),
        Some(60_000.0)
    );
    assert_eq!(
        Reflect::get(&global, &JsValue::from_str("__clearedTimeout"))
            .unwrap()
            .as_f64(),
        Some(4242.0)
    );
    drop(services);
}
