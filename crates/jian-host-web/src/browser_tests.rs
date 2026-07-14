use crate::CanvasKitBackend;
use base64::Engine as _;
use jian_core::geometry::{rect, size};
use jian_core::layout::measure::{FontStyleKind, MeasureBackend, MeasureRequest, StyledRun};
use jian_core::render::{DrawOp, ImageSource, Paint, RenderBackend, TextAlign, TextRun};
use jian_core::scene::Color;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test;

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

pub(crate) fn ensure_canvaskit() {
    use js_sys::{Reflect, Uint8Array};
    use wasm_bindgen::JsValue;

    let global = js_sys::global();
    let init = Reflect::get(&global, &JsValue::from_str("CanvasKitInit")).unwrap();
    if !init.is_function() {
        let script = format!(
            "{}\nglobalThis.CanvasKitInit = CanvasKitInit;",
            include_str!("../assets/canvaskit/canvaskit.js")
        );
        js_sys::eval(&script).expect("vendored CanvasKit JS must evaluate");
    }
    let binary_key = JsValue::from_str("__jianCanvasKitWasmBinary");
    if Reflect::get(&global, &binary_key).unwrap().is_undefined() {
        let bytes = include_bytes!("../assets/canvaskit/canvaskit.wasm");
        let binary = Uint8Array::new_with_length(bytes.len() as u32);
        binary.copy_from(bytes);
        Reflect::set(&global, &binary_key, binary.as_ref()).unwrap();
    }
}

#[wasm_bindgen_test]
fn browser_smoke() {
    assert!(web_sys::window().is_some());
}

#[wasm_bindgen_test(async)]
async fn production_backend_renders_and_releases_registered_image() {
    ensure_canvaskit();
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(128);
    canvas.set_height(96);
    document.body().unwrap().append_child(&canvas).unwrap();

    let mut backend = CanvasKitBackend::load(canvas, "/assets/canvaskit/")
        .await
        .unwrap();
    let mut surface = backend.new_surface(size(128.0, 96.0));
    backend.begin_frame(&mut surface, 0xffffffff);
    backend.draw(&DrawOp::Rect {
        rect: rect(8.0, 8.0, 32.0, 24.0),
        paint: Paint::solid(Color::rgb(255, 0, 0)),
    });
    backend.draw(&DrawOp::Text(TextRun {
        content: "Jian".into(),
        font_family: String::new(),
        font_size: 20.0,
        font_weight: 400,
        color: Color::rgb(0, 0, 0),
        origin: jian_core::geometry::point(8.0, 40.0),
        max_width: 80.0,
        align: TextAlign::Start,
        line_height: 1.2,
    }));
    backend.end_frame(&mut surface);

    let red = surface.read_pixel(12, 12);
    assert!(red[0] > 220 && red[1] < 40 && red[2] < 40);
    assert!(surface.region_has_ink(8, 40, 80, 28));

    // 1x1 opaque red PNG.
    let png = base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z6S8AAAAASUVORK5CYII=")
        .unwrap();
    backend.register_image("fixture:red", &png).unwrap();
    assert!(backend.has_image("fixture:red"));
    backend.begin_frame(&mut surface, 0xffffffff);
    backend.draw(&DrawOp::Image {
        source: ImageSource::Url("fixture:red".into()),
        dst: rect(48.0, 8.0, 16.0, 16.0),
        opacity: 1.0,
    });
    backend.end_frame(&mut surface);
    assert!(surface.read_pixel(52, 12)[0] > 200);
    backend.release_image("fixture:red");
    assert!(!backend.has_image("fixture:red"));
}

#[wasm_bindgen_test(async)]
async fn canvas_kit_measure_matches_render_and_drives_runtime_wrap_height() {
    ensure_canvaskit();
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    let mut backend = CanvasKitBackend::load(canvas, "/assets/canvaskit/")
        .await
        .unwrap();
    let fonts = backend.font_registry();
    let parsed = fonts
        .register(
            "Roboto",
            include_bytes!("../assets/fonts/Roboto-Regular.ttf"),
        )
        .unwrap();
    assert_eq!(parsed, "Roboto");

    let measure = backend.measure_backend();
    let runs = [StyledRun {
        text: "Jian 你好",
        font_family: Some("Roboto"),
        font_size: 24.0,
        font_weight: 400,
        font_style: FontStyleKind::Normal,
        letter_spacing: 0.0,
    }];
    let natural = measure.measure(&MeasureRequest {
        runs: &runs,
        line_height: 1.2,
        max_width: None,
    });
    assert!(natural.width > 20.0 && natural.line_count == 1);

    let mut surface = backend.new_surface(size(240.0, 80.0));
    backend.begin_frame(&mut surface, 0xffffffff);
    backend.draw(&DrawOp::Text(TextRun {
        content: "Jian 你好".into(),
        font_family: "Roboto".into(),
        font_size: 24.0,
        font_weight: 400,
        color: Color::rgb(0, 0, 0),
        origin: jian_core::geometry::point(0.0, 0.0),
        max_width: 240.0,
        align: TextAlign::Start,
        line_height: 1.2,
    }));
    backend.end_frame(&mut surface);
    assert!((surface.last_text_width() - natural.width).abs() <= 1.0);
    assert!(surface.region_has_ink(0, 0, 180, 40));

    let wrapped = measure.measure(&MeasureRequest {
        runs: &runs,
        line_height: 1.2,
        max_width: Some(natural.width * 0.45),
    });
    assert!(wrapped.line_count > 1 && wrapped.height > natural.height);

    let mut runtime = jian_core::Runtime::new();
    runtime
        .load_str(r#"{"version":"1.2","responsive":true,"children":[{"type":"text","id":"copy","content":"Jian 你好 Jian 你好","fontFamily":"Roboto","fontSize":24,"width":"fill_container"}]}"#)
        .unwrap();
    runtime
        .build_layout_with(Rc::new(measure), (100.0, 200.0))
        .unwrap();
    let key = runtime.document.as_ref().unwrap().tree.get("copy").unwrap();
    assert!(runtime.layout.node_rect(key).unwrap().size.height > 24.0);
}

#[wasm_bindgen_test]
fn dom_events_drive_runtime_with_css_scaled_coordinates_and_wheel_contract() {
    let mut runtime = jian_core::Runtime::new();
    runtime
        .load_str(r#"{"formatVersion":"1.0","version":"1.0","state":{"count":{"type":"int","default":0},"scrolled":{"type":"bool","default":false}},"children":[{"type":"rectangle","id":"btn","x":80,"y":40,"width":80,"height":80,"events":{"onTap":[{"set":{"$app.count":"$app.count + 1"}}],"onScroll":[{"set":{"$app.scrolled":"true"}}]}}]}"#)
        .unwrap();
    runtime.build_layout((400.0, 200.0)).unwrap();
    runtime.rebuild_spatial();
    runtime.set_viewport_size((400.0, 200.0));
    let runtime = Rc::new(crate::runtime_slot::RuntimeSlot::new(runtime));

    let document = web_sys::window().unwrap().document().unwrap();
    let canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(400);
    canvas.set_height(200);
    canvas.style().set_property("width", "200px").unwrap();
    canvas.style().set_property("height", "100px").unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();
    let _bridge = crate::event::EventBridge::attach(canvas.clone(), runtime.clone()).unwrap();
    assert_eq!(
        canvas.style().get_property_value("touch-action").unwrap(),
        "none"
    );

    let bounds = canvas.get_bounding_client_rect();
    for kind in ["pointerdown", "pointerup"] {
        let init = web_sys::PointerEventInit::new();
        init.set_pointer_id(7);
        init.set_pointer_type("mouse");
        init.set_client_x((bounds.left() + 50.0) as i32);
        init.set_client_y((bounds.top() + 30.0) as i32);
        init.set_buttons(if kind == "pointerdown" { 1 } else { 0 });
        let event = web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
        canvas.dispatch_event(&event).unwrap();
    }
    assert_eq!(
        runtime.borrow().state.app_get("count").unwrap().as_i64(),
        Some(1)
    );

    let wheel_init = web_sys::WheelEventInit::new();
    // A synthetic event must be cancelable for `preventDefault()` to
    // register — without this the non-passive assertion below fails
    // even though the listener does call it.
    wheel_init.set_cancelable(true);
    wheel_init.set_bubbles(true);
    wheel_init.set_client_x((bounds.left() + 50.0) as i32);
    wheel_init.set_client_y((bounds.top() + 30.0) as i32);
    wheel_init.set_delta_y(12.0);
    wheel_init.set_delta_mode(1);
    let wheel = web_sys::WheelEvent::new_with_event_init_dict("wheel", &wheel_init).unwrap();
    canvas.dispatch_event(&wheel).unwrap();
    assert!(runtime
        .borrow()
        .state
        .app_get("scrolled")
        .unwrap()
        .as_bool()
        .unwrap());
    let mapped = crate::event::wheel::map_delta(2.0, 12.0, 0.0, 1);
    assert_eq!(mapped.0.y, -12.0);
    assert_eq!(mapped.2, jian_core::gesture::ScrollMode::Line);
    assert!(
        wheel.default_prevented(),
        "wheel listener must be non-passive"
    );
}

#[wasm_bindgen_test]
fn hidden_input_commits_cjk_and_tracks_focused_field() {
    let mut runtime = jian_core::Runtime::new();
    runtime
        .load_str(r#"{"version":"1.2","children":[{"type":"text_input","id":"field","x":20,"y":30,"width":100,"height":30,"value":""}]}"#)
        .unwrap();
    runtime.build_layout((200.0, 100.0)).unwrap();
    runtime.rebuild_spatial();
    runtime.set_viewport_size((200.0, 100.0));
    let key = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    runtime.focus_request(key).unwrap();
    let runtime = Rc::new(crate::runtime_slot::RuntimeSlot::new(runtime));

    let document = web_sys::window().unwrap().document().unwrap();
    let canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.style().set_property("width", "200px").unwrap();
    canvas.style().set_property("height", "100px").unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();
    let mut ime = crate::ime_input::ImeInput::attach(&canvas, runtime.clone()).unwrap();
    ime.sync_from_runtime();
    let canvas_bounds = canvas.get_bounding_client_rect();
    let left: f64 = ime
        .input()
        .style()
        .get_property_value("left")
        .unwrap()
        .trim_end_matches("px")
        .parse()
        .unwrap();
    let top: f64 = ime
        .input()
        .style()
        .get_property_value("top")
        .unwrap()
        .trim_end_matches("px")
        .parse()
        .unwrap();
    assert!((left - (canvas_bounds.left() + 20.0)).abs() < 0.01);
    assert!((top - (canvas_bounds.top() + 30.0)).abs() < 0.01);

    let target: web_sys::EventTarget = ime.input().clone().into();
    let start = web_sys::CompositionEvent::new("compositionstart").unwrap();
    target.dispatch_event(&start).unwrap();
    let update_init = web_sys::CompositionEventInit::new();
    update_init.set_data("ni");
    let update =
        web_sys::CompositionEvent::new_with_event_init_dict("compositionupdate", &update_init)
            .unwrap();
    target.dispatch_event(&update).unwrap();
    let end_init = web_sys::CompositionEventInit::new();
    end_init.set_data("你");
    let end =
        web_sys::CompositionEvent::new_with_event_init_dict("compositionend", &end_init).unwrap();
    target.dispatch_event(&end).unwrap();

    let mut runtime = runtime.borrow_mut();
    let node = runtime.document.as_ref().unwrap().tree.nodes[key]
        .schema
        .clone();
    let jian_core::Runtime {
        widget_states,
        state,
        ..
    } = &mut *runtime;
    let state = widget_states.get_or_init(&node, state).unwrap();
    let jian_core::widget_state::WidgetState::TextInput(field) = state else {
        panic!()
    };
    assert_eq!(field.text(), "你");
}

#[wasm_bindgen_test(async)]
async fn pump_handles_breakpoints_zero_size_timers_and_context_restore() {
    ensure_canvaskit();
    use wasm_bindgen::JsValue;

    async fn wait(ms: i32) {
        let promise = js_sys::Promise::new(&mut |resolve, _reject| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    }

    let document = web_sys::window().unwrap().document().unwrap();
    let canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.style().set_property("width", "320px").unwrap();
    canvas.style().set_property("height", "240px").unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();
    let handle = crate::mount_jian(
        canvas.clone(),
        JsValue::from_str(include_str!(
            "../../jian-core/tests/fixtures/responsive_variants.json"
        )),
        JsValue::UNDEFINED,
    )
    .await
    .unwrap();
    wait(40).await;
    assert_eq!(
        handle.test_runtime().unwrap().borrow().active_page_key(),
        "home-m@0-480"
    );

    canvas.style().set_property("width", "1200px").unwrap();
    wait(40).await;
    assert_eq!(
        handle.test_runtime().unwrap().borrow().active_page_key(),
        "home-d"
    );

    wasm_bindgen_futures::JsFuture::from(handle.set_document(JsValue::from_str(
        r#"{"version":"1.2","state":{"done":{"type":"bool","default":false}},"children":[{"type":"text_input","id":"slow","x":240,"y":20,"width":80,"height":40,"value":"","events":{"onKey":[{"delay":{"ms":20}},{"set":{"$app.done":"true"}}]}}]}"#,
    )))
    .await
    .unwrap();
    let runtime = handle.test_runtime().unwrap();
    let slow = runtime
        .borrow()
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("slow")
        .unwrap();
    let slow_rect = runtime.borrow().layout.node_rect(slow).unwrap();
    assert_eq!(slow_rect.origin.x, 240.0);
    assert_eq!(slow_rect.origin.y, 20.0);

    let bounds = canvas.get_bounding_client_rect();
    for kind in ["pointerdown", "pointerup"] {
        let init = web_sys::PointerEventInit::new();
        init.set_pointer_id(1);
        init.set_pointer_type("mouse");
        init.set_client_x((bounds.left() + 260.0) as i32);
        init.set_client_y((bounds.top() + 40.0) as i32);
        init.set_buttons(if kind == "pointerdown" { 1 } else { 0 });
        canvas
            .dispatch_event(&web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap())
            .unwrap();
    }
    wait(40).await;
    canvas.style().set_property("width", "0px").unwrap();
    canvas.style().set_property("height", "0px").unwrap();
    // Let the ResizeObserver-driven pump wake fully drain. The keyboard
    // event below must schedule its own pump work while painting is idle
    // and the canvas remains zero-sized.
    wait(40).await;
    assert!(!runtime
        .borrow()
        .state
        .app_get("done")
        .unwrap()
        .as_bool()
        .unwrap());
    let key_init = web_sys::KeyboardEventInit::new();
    key_init.set_key("Enter");
    canvas
        .dispatch_event(
            &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &key_init)
                .unwrap(),
        )
        .unwrap();
    wait(70).await;
    assert!(runtime
        .borrow()
        .state
        .app_get("done")
        .unwrap()
        .as_bool()
        .unwrap());

    canvas.style().set_property("width", "400px").unwrap();
    canvas.style().set_property("height", "240px").unwrap();
    wait(40).await;
    let before = handle.test_presented_frames();
    canvas
        .dispatch_event(&web_sys::Event::new("webglcontextlost").unwrap())
        .unwrap();
    canvas
        .dispatch_event(&web_sys::Event::new("webglcontextrestored").unwrap())
        .unwrap();
    wait(50).await;
    assert!(
        handle.test_presented_frames() > before,
        "restore must repaint retained dirt"
    );
    handle.dispose();
}

#[wasm_bindgen_test(async)]
async fn web_services_follow_degraded_state_and_asset_trust_contracts() {
    use jian_core::action::services::PlatformService;
    use jian_core::gesture::{PointerEvent, PointerPhase};
    use jian_core::render::image_store::ImageResolver;
    use js_sys::{Function, Reflect};
    use wasm_bindgen::JsValue;

    async fn microtask() {
        wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&JsValue::NULL))
            .await
            .unwrap();
    }
    fn tap(runtime: &mut jian_core::Runtime, y: f32, now: u64) {
        runtime.dispatch_pointer(PointerEvent::simple_at(
            1,
            PointerPhase::Down,
            jian_core::geometry::point(20.0, y),
            now,
        ));
        runtime.dispatch_pointer(PointerEvent::simple_at(
            1,
            PointerPhase::Up,
            jian_core::geometry::point(20.0, y),
            now + 1,
        ));
        runtime.pump(now + 1);
    }

    let global = js_sys::global();
    let original_fetch = Reflect::get(&global, &JsValue::from_str("fetch")).unwrap();
    let window = web_sys::window().unwrap();
    let original_open = Reflect::get(window.as_ref(), &JsValue::from_str("open")).unwrap();
    let storage = window.local_storage().unwrap().unwrap();
    let original_clear = Reflect::get(storage.as_ref(), &JsValue::from_str("clear")).unwrap();
    Reflect::set(
        window.as_ref(),
        &JsValue::from_str("open"),
        Function::new_no_args("return null").as_ref(),
    )
    .unwrap();
    Reflect::set(
        storage.as_ref(),
        &JsValue::from_str("clear"),
        Function::new_no_args("throw new DOMException('quota', 'QuotaExceededError')").as_ref(),
    )
    .unwrap();
    Reflect::set(
        &global,
        &JsValue::from_str("fetch"),
        Function::new_with_args(
            "request",
            "globalThis.__jianCredentials=request.credentials; globalThis.__jianRedirect=request.redirect; return Promise.reject(new Error('network down'));",
        )
        .as_ref(),
    )
    .unwrap();

    let mut runtime = jian_core::Runtime::new();
    runtime
        .load_str(r#"{"version":"1.2","app":{"name":"svc","version":"1","id":"svc","capabilities":["network","storage"]},"state":{"storageCaught":{"type":"bool","default":false},"storageContinued":{"type":"bool","default":false},"popupContinued":{"type":"bool","default":false},"fetchCaught":{"type":"bool","default":false},"fetchContinued":{"type":"bool","default":false},"wsContinued":{"type":"bool","default":false}},"children":[{"type":"rectangle","id":"storage","y":0,"width":80,"height":40,"events":{"onTap":[{"storage_wipe":{"on_error":[{"set":{"$app.storageCaught":"true"}}]}},{"set":{"$app.storageContinued":"true"}}]}},{"type":"rectangle","id":"popup","y":50,"width":80,"height":40,"events":{"onTap":[{"open_url":{"url":"'https://example.test/'"}},{"set":{"$app.popupContinued":"true"}}]}},{"type":"rectangle","id":"fetch","y":100,"width":80,"height":40,"events":{"onTap":[{"fetch":{"url":"'https://example.test/data'","on_error":[{"set":{"$app.fetchCaught":"true"}}]}},{"set":{"$app.fetchContinued":"true"}}]}},{"type":"rectangle","id":"ws","y":150,"width":80,"height":40,"events":{"onTap":[{"ws_connect":{"id":"x","url":"'wss://example.test/'"}},{"set":{"$app.wsContinued":"true"}}]}}]}"#)
        .unwrap();
    runtime.build_layout((200.0, 220.0)).unwrap();
    runtime.rebuild_spatial();
    let _services = crate::services::WebServices::install(
        &mut runtime,
        None,
        crate::services::AbortRegistry::default(),
        Rc::new(|| {}),
    )
    .unwrap();

    tap(&mut runtime, 20.0, 10);
    tap(&mut runtime, 70.0, 20);
    tap(&mut runtime, 120.0, 30);
    microtask().await;
    runtime.pump(31);
    tap(&mut runtime, 170.0, 40);
    runtime.pump(41);
    assert!(runtime
        .state
        .app_get("storageCaught")
        .unwrap()
        .as_bool()
        .unwrap());
    assert!(runtime
        .state
        .app_get("storageContinued")
        .unwrap()
        .as_bool()
        .unwrap());
    assert!(runtime
        .state
        .app_get("popupContinued")
        .unwrap()
        .as_bool()
        .unwrap());
    assert!(runtime
        .state
        .app_get("fetchCaught")
        .unwrap()
        .as_bool()
        .unwrap());
    assert!(runtime
        .state
        .app_get("fetchContinued")
        .unwrap()
        .as_bool()
        .unwrap());
    assert!(!runtime
        .state
        .app_get("wsContinued")
        .unwrap()
        .as_bool()
        .unwrap());
    assert!(crate::services::platform::WebPlatform
        .open_url("https://example.test/")
        .unwrap_err()
        .0
        .contains("popup blocked"));

    Reflect::set(
        &global,
        &JsValue::from_str("fetch"),
        Function::new_with_args(
            "request",
            "globalThis.__jianCredentials=request.credentials; globalThis.__jianRedirect=request.redirect; return Promise.resolve(new Response(new Uint8Array([1,2,3,4])));",
        )
        .as_ref(),
    )
    .unwrap();
    let policy = crate::services::AssetPolicy::parse("https://example.test/assets/").unwrap();
    let resolver = crate::services::WebImageResolver::new(
        Some(policy),
        crate::services::AbortRegistry::default(),
        Rc::new(|| {}),
    );
    assert_eq!(
        resolver.resolve("icons/a.bin").await.unwrap(),
        vec![1, 2, 3, 4]
    );
    let untrusted = resolver
        .admission("//other.test/assets/a.bin")
        .unwrap()
        .unwrap();
    assert!(untrusted.requires_network);
    assert_eq!(
        resolver.resolve("//other.test/assets/a.bin").await.unwrap(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(
        Reflect::get(&global, &JsValue::from_str("__jianCredentials"))
            .unwrap()
            .as_string()
            .as_deref(),
        Some("omit")
    );
    assert_eq!(
        Reflect::get(&global, &JsValue::from_str("__jianRedirect"))
            .unwrap()
            .as_string()
            .as_deref(),
        Some("error")
    );

    Reflect::set(&global, &JsValue::from_str("fetch"), &original_fetch).unwrap();
    Reflect::set(window.as_ref(), &JsValue::from_str("open"), &original_open).unwrap();
    Reflect::set(
        storage.as_ref(),
        &JsValue::from_str("clear"),
        &original_clear,
    )
    .unwrap();
}

#[wasm_bindgen_test(async)]
async fn mount_fifo_hot_reload_and_dispose_contract() {
    ensure_canvaskit();
    use js_sys::{Array, Function, Promise, Reflect, JSON};
    use wasm_bindgen::JsValue;

    let document = web_sys::window().unwrap().document().unwrap();
    let invalid_canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    assert!(crate::mount_jian(
        invalid_canvas,
        JsValue::from_str("{not json"),
        JsValue::UNDEFINED,
    )
    .await
    .is_err());

    let global = js_sys::global();
    let original_fetch = Reflect::get(&global, &JsValue::from_str("fetch")).unwrap();
    Reflect::set(
        &global,
        &JsValue::from_str("fetch"),
        Function::new_with_args(
            "request",
            "globalThis.__jianAssetRequestUrl=request.url; const png=Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z6S8AAAAASUVORK5CYII='), c=>c.charCodeAt(0)); return new Promise((resolve) => setTimeout(() => resolve(new Response(png)), 20));",
        )
        .as_ref(),
    )
    .unwrap();

    let canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.style().set_property("width", "320px").unwrap();
    canvas.style().set_property("height", "240px").unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();
    let options = JSON::parse(r#"{"assetBase":"https://example.test/assets/"}"#).unwrap();
    let handle = crate::mount_jian(
        canvas.clone(),
        JsValue::from_str(r#"{"version":"1.2","state":{"count":{"type":"int","default":0}},"children":[{"type":"rectangle","id":"button","width":80,"height":40,"events":{"onTap":[{"set":{"$app.count":"$app.count + 1"}}]}}]}"#),
        options,
    )
    .await
    .unwrap();
    assert_eq!(handle.test_presented_frames(), 0);
    let bounds = canvas.get_bounding_client_rect();
    for kind in ["pointerdown", "pointerup"] {
        let init = web_sys::PointerEventInit::new();
        init.set_pointer_id(2);
        init.set_pointer_type("mouse");
        init.set_client_x((bounds.left() + 10.0) as i32);
        init.set_client_y((bounds.top() + 10.0) as i32);
        init.set_buttons(if kind == "pointerdown" { 1 } else { 0 });
        canvas
            .dispatch_event(&web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap())
            .unwrap();
    }
    let preserve = handle.set_document(JsValue::from_str(r#"{"version":"1.2","state":{"count":{"type":"int","default":99}},"children":[{"type":"rectangle","id":"button","width":80,"height":40}]}"#));
    wasm_bindgen_futures::JsFuture::from(preserve)
        .await
        .unwrap();
    assert_eq!(
        handle
            .test_runtime()
            .unwrap()
            .borrow()
            .state
            .app_get("count")
            .unwrap()
            .as_i64(),
        Some(1)
    );

    wasm_bindgen_futures::JsFuture::from(handle.set_document(JsValue::from_str(
        r#"{"version":"1.2","responsive":true,"children":[{"type":"image","id":"hero","src":"slow.png","width":10,"height":10}]}"#,
    )))
    .await
    .unwrap();
    crate::production_bridges::wait(40).await;
    assert_eq!(
        Reflect::get(&global, &JsValue::from_str("__jianAssetRequestUrl"))
            .unwrap()
            .as_string()
            .as_deref(),
        Some("https://example.test/assets/slow.png")
    );
    {
        let runtime = handle.test_runtime().unwrap();
        let runtime = runtime.borrow();
        let key = runtime
            .state
            .image_key("slow.png")
            .expect("authored source must retain a keyed ImageStore admission");
        assert!(!key.starts_with("data:"));
        assert!(matches!(
            runtime.image_store.state(&key),
            Some(
                jian_core::render::image_store::ImageState::Bytes
                    | jian_core::render::image_store::ImageState::Registered
            )
        ));
    }

    let slow = handle.set_document(JsValue::from_str(r#"{"version":"1.2","responsive":true,"children":[{"type":"image","id":"hero","src":"fifo.png","width":10,"height":10}]}"#));
    let final_document = handle.set_document(JsValue::from_str(include_str!(
        "../../jian-core/tests/fixtures/responsive_variants.json"
    )));
    let promises = Array::new();
    promises.push(&slow);
    promises.push(&final_document);
    wasm_bindgen_futures::JsFuture::from(Promise::all(&promises))
        .await
        .unwrap();
    assert_eq!(
        handle.test_runtime().unwrap().borrow().active_page_key(),
        "home-m@0-480"
    );

    Reflect::set(
        &global,
        &JsValue::from_str("fetch"),
        Function::new_with_args(
            "request",
            "globalThis.__jianPendingSignal=request.signal; return new Promise((_resolve,reject) => request.signal.addEventListener('abort', () => reject(new DOMException('aborted','AbortError'))));",
        )
        .as_ref(),
    )
    .unwrap();
    wasm_bindgen_futures::JsFuture::from(handle.set_document(JsValue::from_str(r#"{"version":"1.2","children":[{"type":"image","id":"hero","src":"pending.png","width":10,"height":10}]}"#)))
        .await
        .unwrap();
    crate::production_bridges::wait(30).await;
    let in_flight = handle.set_document(JsValue::from_str(
        r#"{"version":"1.2","children":[{"type":"rectangle","id":"next","width":10,"height":10}]}"#,
    ));
    let queued = handle.set_document(JsValue::from_str(r#"{"version":"1.2","children":[]}"#));
    // One microtask lets the FIFO worker dequeue the first item and pause at
    // its cancellable pre-commit boundary; the second item remains queued.
    wasm_bindgen_futures::JsFuture::from(Promise::resolve(&JsValue::NULL))
        .await
        .unwrap();
    handle.dispose();
    let pending = Array::new();
    pending.push(&in_flight);
    pending.push(&queued);
    let outcomes = wasm_bindgen_futures::JsFuture::from(Promise::all_settled(&pending))
        .await
        .unwrap();
    for outcome in Array::from(&outcomes).iter() {
        assert_eq!(
            Reflect::get(&outcome, &JsValue::from_str("status"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("rejected")
        );
    }
    assert!(handle.test_disposed());
    let signal = Reflect::get(&global, &JsValue::from_str("__jianPendingSignal")).unwrap();
    assert!(Reflect::get(&signal, &JsValue::from_str("aborted"))
        .unwrap()
        .as_bool()
        .unwrap());
    Reflect::set(&global, &JsValue::from_str("fetch"), &original_fetch).unwrap();
}
