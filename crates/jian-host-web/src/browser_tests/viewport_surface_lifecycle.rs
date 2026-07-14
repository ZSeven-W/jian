use crate::tests::ensure_canvaskit;
use js_sys::{Object, Promise, Reflect};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test;

fn unstyled_canvas() -> web_sys::HtmlCanvasElement {
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();
    canvas
}

async fn wait(ms: i32) {
    let promise = Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    JsFuture::from(promise).await.unwrap();
}

fn close_enough(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 0.5
}

#[wasm_bindgen_test]
fn forced_2x_backing_resize_preserves_unstyled_logical_box() {
    let canvas = unstyled_canvas();
    let initial = canvas.get_bounding_client_rect();
    let width = (initial.width() * 2.0).round() as u32;
    let height = (initial.height() * 2.0).round() as u32;

    crate::viewport::resize_backing_store_preserving_css_box(&canvas, width, height).unwrap();

    let after = canvas.get_bounding_client_rect();
    assert_eq!(canvas.width(), width);
    assert_eq!(canvas.height(), height);
    assert!(close_enough(after.width(), initial.width()));
    assert!(close_enough(after.height(), initial.height()));
    canvas.remove();
}

#[wasm_bindgen_test]
fn backing_resize_does_not_pin_stylesheet_sized_canvas() {
    let document = web_sys::window().unwrap().document().unwrap();
    let style = document.create_element("style").unwrap();
    style.set_text_content(Some(".jian-viewport-test { width: 173px; height: 91px; }"));
    document.body().unwrap().append_child(&style).unwrap();
    let canvas = unstyled_canvas();
    canvas.set_class_name("jian-viewport-test");
    let initial = canvas.get_bounding_client_rect();

    crate::viewport::resize_backing_store_preserving_css_box(&canvas, 346, 182).unwrap();

    let after = canvas.get_bounding_client_rect();
    assert!(close_enough(after.width(), initial.width()));
    assert!(close_enough(after.height(), initial.height()));
    assert_eq!(canvas.style().get_property_value("width").unwrap(), "");
    assert_eq!(canvas.style().get_property_value("height").unwrap(), "");
    canvas.remove();
    style.remove();
}

#[wasm_bindgen_test(async)]
async fn mounted_unstyled_canvas_converges_to_logical_size_times_dpr() {
    ensure_canvaskit();
    let canvas = unstyled_canvas();
    let initial = canvas.get_bounding_client_rect();
    let dpr = web_sys::window().unwrap().device_pixel_ratio().max(1.0);
    let handle = crate::mount_jian(
        canvas.clone(),
        JsValue::from_str(
            r#"{"version":"1.2","children":[{"type":"rectangle","id":"viewport-root","width":40,"height":20}]}"#,
        ),
        JsValue::UNDEFINED,
    )
    .await
    .unwrap();

    wait(100).await;
    let settled = canvas.get_bounding_client_rect();
    assert!(close_enough(settled.width(), initial.width()));
    assert!(close_enough(settled.height(), initial.height()));
    assert_eq!(canvas.width(), (initial.width() * dpr).round() as u32);
    assert_eq!(canvas.height(), (initial.height() * dpr).round() as u32);

    wait(80).await;
    let stable = canvas.get_bounding_client_rect();
    assert!(close_enough(stable.width(), initial.width()));
    assert!(close_enough(stable.height(), initial.height()));
    assert_eq!(canvas.width(), (initial.width() * dpr).round() as u32);
    assert_eq!(canvas.height(), (initial.height() * dpr).round() as u32);

    handle.dispose();
    canvas.remove();
}

#[wasm_bindgen_test(async)]
async fn surface_creation_failure_reports_once_and_stays_paint_idle() {
    ensure_canvaskit();
    let errors = Rc::new(RefCell::new(Vec::<JsValue>::new()));
    let on_error = {
        let errors = errors.clone();
        Closure::wrap(Box::new(move |value: JsValue| {
            errors.borrow_mut().push(value);
        }) as Box<dyn FnMut(JsValue)>)
    };
    let options = Object::new();
    Reflect::set(
        &options,
        &JsValue::from_str("onError"),
        on_error.as_ref().unchecked_ref(),
    )
    .unwrap();
    let canvas = unstyled_canvas();
    canvas.style().set_property("width", "180px").unwrap();
    canvas.style().set_property("height", "100px").unwrap();
    let handle = crate::mount_jian(
        canvas.clone(),
        JsValue::from_str(
            r#"{"version":"1.2","children":[{"type":"rectangle","id":"surface-root","width":40,"height":20}]}"#,
        ),
        options.into(),
    )
    .await
    .unwrap();
    wait(120).await;
    let before = handle.test_presented_frames();
    assert!(before > 0);

    handle.test_fail_next_surface();
    canvas.style().set_property("width", "220px").unwrap();
    wait(120).await;

    assert_eq!(handle.test_presented_frames(), before);
    assert!(handle.test_needs_paint());
    assert!(!handle.test_has_pending_frame());
    {
        let captured = errors.borrow();
        assert_eq!(captured.len(), 1);
        let payload = &captured[0];
        assert_eq!(
            Reflect::get(payload, &JsValue::from_str("kind"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("internal")
        );
        assert_eq!(
            Reflect::get(payload, &JsValue::from_str("source"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("surface")
        );
    }

    wait(100).await;
    assert_eq!(handle.test_presented_frames(), before);
    assert_eq!(errors.borrow().len(), 1);
    assert!(!handle.test_has_pending_frame());

    handle.dispose();
    canvas.remove();
    drop(on_error);
}
