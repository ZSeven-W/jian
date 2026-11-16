use super::ensure_canvaskit;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::wasm_bindgen_test;

async fn wait(ms: i32) {
    let completion = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(completion)
        .await
        .unwrap();
}

#[wasm_bindgen_test(async)]
async fn standalone_canvas_paints_motion_and_stops_requesting_frames() {
    ensure_canvaskit();
    let window = web_sys::window().unwrap();
    assert!(!window
        .match_media("(prefers-reduced-motion: reduce)")
        .unwrap()
        .unwrap()
        .matches());
    let document = window.document().unwrap();
    let canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.style().set_property("width", "64px").unwrap();
    canvas.style().set_property("height", "64px").unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();

    let source = include_str!("../../../jian-core/tests/fixtures/motion_opacity.op")
        .replace("\"durationMs\":160", "\"durationMs\":400");
    let handle = crate::mount_jian(
        canvas.clone(),
        JsValue::from_str(&source),
        JsValue::UNDEFINED,
    )
    .await
    .unwrap();

    let mut saw_partial = false;
    for _ in 0..7 {
        wait(45).await;
        let single = handle.test_read_pixel(8.0, 8.0);
        let overlap = handle.test_read_pixel(14.0, 8.0);
        if let (Some(single), Some(overlap)) = (single, overlap) {
            assert_eq!(single, overlap, "children composite together");
            saw_partial |= (1..255).contains(&single[0]);
        }
    }
    assert!(
        saw_partial,
        "a presented frame must contain partial opacity"
    );
    wait(200).await;
    assert_eq!(handle.test_read_pixel(14.0, 8.0), Some([0, 0, 0, 255]));
    let settled = handle.test_presented_frames();
    assert!(settled > 2);
    wait(70).await;
    assert_eq!(handle.test_presented_frames(), settled);
    handle.dispose();
    canvas.remove();
}
