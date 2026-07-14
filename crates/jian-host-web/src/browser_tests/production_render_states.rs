use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test;

fn canvas(width: u32, height: u32) -> web_sys::HtmlCanvasElement {
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas = document
        .create_element("canvas")
        .unwrap()
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .unwrap();
    canvas
        .style()
        .set_property("width", &format!("{width}px"))
        .unwrap();
    canvas
        .style()
        .set_property("height", &format!("{height}px"))
        .unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();
    canvas
}

async fn mount(canvas: &web_sys::HtmlCanvasElement, document: &str) -> crate::JianHandle {
    super::tests::ensure_canvaskit();
    let handle = crate::mount_jian(
        canvas.clone(),
        JsValue::from_str(document),
        JsValue::UNDEFINED,
    )
    .await
    .unwrap();
    wait_for_presented_frame_after(&handle, 0).await;
    handle
}

async fn wait_for_presented_frame_after(handle: &crate::JianHandle, previous: u64) {
    for _ in 0..100 {
        if handle.test_presented_frames() > previous {
            return;
        }
        crate::production_bridges::wait(10).await;
    }
    panic!(
        "timed out waiting for a presented frame after {previous}; current={}",
        handle.test_presented_frames()
    );
}

fn red(pixel: [u8; 4]) -> bool {
    pixel[0] > 180 && pixel[1] < 90 && pixel[2] < 90 && pixel[3] > 245
}

fn green(pixel: [u8; 4]) -> bool {
    pixel[1] > 180 && pixel[0] < 90 && pixel[2] < 90 && pixel[3] > 245
}

fn log_frame_evidence(handle: &crate::JianHandle, label: &str, control: [u8; 4]) {
    wasm_bindgen_test::console_log!(
        "{label}: frame={} control={control:?} trace={} layers={}",
        handle.test_presented_frames(),
        handle.test_last_frame_trace(),
        handle.test_last_frame_layer_trace(),
    );
}

fn assert_trace_contains(handle: &crate::JianHandle, required: &[&str]) {
    let trace = handle.test_last_frame_trace();
    for entry in required {
        assert!(
            trace.split(" -> ").any(|actual| actual == *entry),
            "production frame trace is missing `{entry}`: {trace}"
        );
    }
}

#[wasm_bindgen_test(async)]
async fn fixed_width_height_text_clips_but_stays_naturally_unwrapped() {
    let canvas = canvas(260, 80);
    let fixed = r##"{
      "version":"1.2","responsive":true,
      "children":[{"type":"text","id":"copy","x":10,"y":10,"width":60,"height":24,
        "content":"MMMMMMMMMMMM","fontSize":24,"textGrowth":"fixed-width-height",
        "fill":[{"type":"solid","color":"#000000"}]},
        {"type":"rectangle","id":"control","x":230,"y":55,"width":20,"height":20,
         "fill":[{"type":"solid","color":"#00ff00"}]}]
    }"##;
    let handle = mount(&canvas, fixed).await;

    let control = handle.test_read_pixel(240.0, 65.0).unwrap();
    log_frame_evidence(&handle, "fixed text", control);
    assert!(
        green(control),
        "plain control rect must paint, got {control:?}"
    );
    assert_trace_contains(
        &handle,
        &["backend.draw_text_plan", "surface.draw_rich_text"],
    );

    assert!(handle.test_region_has_ink(10.0, 10.0, 55.0, 24.0));
    assert!(
        !handle.test_region_has_ink(75.0, 10.0, 170.0, 28.0),
        "fixed-width-height ink must not escape its authored bounds"
    );

    let auto = fixed.replace("fixed-width-height", "auto");
    let previous = handle.test_presented_frames();
    JsFuture::from(handle.set_document(JsValue::from_str(&auto)))
        .await
        .unwrap();
    wait_for_presented_frame_after(&handle, previous).await;
    assert!(
        handle.test_region_has_ink(75.0, 10.0, 170.0, 28.0),
        "auto text should retain its natural unwrapped extent"
    );

    handle.dispose();
    canvas.remove();
}

#[wasm_bindgen_test(async)]
async fn production_frame_clips_children_and_rotates_subtrees() {
    let canvas = canvas(180, 120);
    let handle = mount(
        &canvas,
        r##"{
          "version":"1.2","responsive":true,
          "children":[{"type":"frame","id":"viewport","width":"fill_container","height":"fill_container",
            "children":[
              {"type":"frame","id":"clip","x":5,"y":5,"width":60,"height":45,"clipContent":true,
               "children":[{"type":"rectangle","id":"wide","x":40,"y":10,"width":50,"height":20,
                 "fill":[{"type":"solid","color":"#ff0000"}]}]},
              {"type":"rectangle","id":"rotated","x":80,"y":60,"width":40,"height":10,"rotation":90,
               "fill":[{"type":"solid","color":"#ff0000"}]},
              {"type":"rectangle","id":"later","x":140,"y":10,"width":20,"height":20,
               "fill":[{"type":"solid","color":"#00ff00"}]}
            ]}]
        }"##,
    )
    .await;

    let control = handle.test_read_pixel(150.0, 20.0).unwrap();
    log_frame_evidence(&handle, "clip and transform", control);
    assert!(
        green(control),
        "plain control rect must paint, got {control:?}"
    );
    assert_trace_contains(
        &handle,
        &[
            "backend.push_clip",
            "backend.push_transform",
            "surface.push_clip",
            "surface.push_transform",
        ],
    );

    let clipped_child = handle.test_read_pixel(50.0, 25.0).unwrap();
    assert!(
        red(clipped_child),
        "expected clipped child source pixel at (50,25), got {clipped_child:?}"
    );
    assert!(
        !handle.test_region_has_ink(70.0, 15.0, 15.0, 20.0),
        "child pixels outside the frame must be clipped"
    );
    assert!(
        red(handle.test_read_pixel(100.0, 48.0).unwrap()),
        "90-degree rotation must move the horizontal rect vertically"
    );
    assert!(
        !handle.test_region_has_ink(81.0, 62.0, 8.0, 6.0),
        "the unrotated left footprint must be empty"
    );
    assert!(green(handle.test_read_pixel(150.0, 20.0).unwrap()));

    handle.dispose();
    canvas.remove();
}

#[wasm_bindgen_test(async)]
async fn production_frame_applies_blur_and_shadow_layers() {
    let canvas = canvas(160, 90);
    let handle = mount(
        &canvas,
        r##"{
          "version":"1.2","responsive":true,
          "children":[
            {"type":"rectangle","id":"blurred","x":20,"y":25,"width":25,"height":25,
             "fill":[{"type":"solid","color":"#ff0000"}],
             "effects":[{"type":"blur","radius":4}]},
            {"type":"rectangle","id":"shadowed","x":80,"y":25,"width":20,"height":20,
             "fill":[{"type":"solid","color":"#ff0000"}],
             "effects":[{"type":"shadow","offsetX":12,"offsetY":0,"blur":0,"spread":4,"color":"#000000"}]},
            {"type":"rectangle","id":"control","x":130,"y":5,"width":20,"height":15,
             "fill":[{"type":"solid","color":"#00ff00"}]}
          ]
        }"##,
    )
    .await;

    let control = handle.test_read_pixel(140.0, 12.0).unwrap();
    log_frame_evidence(&handle, "blur and shadow", control);
    assert!(
        green(control),
        "plain control rect must paint, got {control:?}"
    );
    assert_trace_contains(
        &handle,
        &[
            "backend.apply_blur",
            "backend.apply_shadow",
            "backend.push_layer",
            "surface.push_blur_layer",
            "surface.push_shadow_layer",
        ],
    );
    assert!(
        handle.test_region_has_ink(13.0, 28.0, 6.0, 18.0),
        "blur must paint a visible tail outside the source rect"
    );
    let source = handle.test_read_pixel(88.0, 35.0).unwrap();
    assert!(
        red(source),
        "shadow layer must preserve its red source at (88,35), got {source:?}"
    );
    let shadow = handle.test_read_pixel(106.0, 35.0).unwrap();
    assert!(
        shadow[0] < 60 && shadow[1] < 60 && shadow[2] < 60 && shadow[3] > 245,
        "shadow layer must paint its offset tail, got {shadow:?}"
    );
    let spread = handle.test_read_pixel(106.0, 22.0).unwrap();
    assert!(
        spread[0] < 60 && spread[1] < 60 && spread[2] < 60 && spread[3] > 245,
        "responsive shadow spread must dilate the layer before offsetting it, got {spread:?}"
    );

    handle.dispose();
    canvas.remove();
}

#[wasm_bindgen_test(async)]
async fn nonresponsive_production_keeps_legacy_direct_shadow_geometry() {
    let canvas = canvas(100, 80);
    let handle = mount(
        &canvas,
        r##"{
          "version":"1.2",
          "children":[{
            "type":"rectangle","id":"legacy","x":30,"y":25,"width":20,"height":20,
            "fill":[{"type":"solid","color":"#ff0000"}],
            "effects":[{
              "type":"shadow","offsetX":0,"offsetY":0,"blur":0,"spread":4,
              "color":"#000000"
            }]
          },{"type":"rectangle","id":"control","x":70,"y":5,"width":20,"height":15,
              "fill":[{"type":"solid","color":"#00ff00"}]}]
        }"##,
    )
    .await;

    let control = handle.test_read_pixel(80.0, 12.0).unwrap();
    log_frame_evidence(&handle, "legacy shadow", control);
    assert!(
        green(control),
        "plain control rect must paint, got {control:?}"
    );
    assert_trace_contains(&handle, &["backend.draw", "surface.draw_shadowed_rect"]);

    let spread = handle.test_read_pixel(27.0, 35.0).unwrap();
    assert!(
        spread[0] < 60 && spread[1] < 60 && spread[2] < 60 && spread[3] > 245,
        "legacy direct-shadow spread must remain visible outside the source rect, got {spread:?}"
    );
    let source = handle.test_read_pixel(40.0, 35.0).unwrap();
    assert!(
        red(source),
        "legacy direct shadow must leave its red source visible, got {source:?}"
    );

    handle.dispose();
    canvas.remove();
}

#[wasm_bindgen_test(async)]
async fn responsive_composed_effect_layers_keep_their_accumulated_tail() {
    let canvas = canvas(140, 90);
    let handle = mount(
        &canvas,
        r##"{
          "version":"1.2","responsive":true,
          "children":[{
            "type":"rectangle","id":"composed","x":40,"y":30,"width":20,"height":20,
            "fill":[{"type":"solid","color":"#ff0000"}],
            "effects":[
              {"type":"blur","radius":16},
              {"type":"shadow","offsetX":12,"offsetY":0,"blur":5,"spread":0,"color":"#000000"}
            ]
          },{"type":"rectangle","id":"control","x":110,"y":5,"width":20,"height":15,
              "fill":[{"type":"solid","color":"#00ff00"}]}]
        }"##,
    )
    .await;

    let control = handle.test_read_pixel(120.0, 12.0).unwrap();
    log_frame_evidence(&handle, "composed effects", control);
    assert!(
        green(control),
        "plain control rect must paint, got {control:?}"
    );
    assert_trace_contains(
        &handle,
        &[
            "backend.apply_blur",
            "backend.apply_shadow",
            "surface.push_blur_layer",
            "surface.push_shadow_layer",
        ],
    );
    assert_eq!(
        handle.test_last_frame_layer_trace(),
        "backend.blur=[-23.0,-33.0,158.0,146.0] -> backend.shadow=[25.0,15.0,62.0,50.0] -> surface.blur=[-23.0,-33.0,158.0,146.0] -> surface.shadow=[25.0,15.0,62.0,50.0]",
        "mounted production replay must preserve the accumulated logical layer bounds at the CanvasKit boundary"
    );

    // Content x=[40,60]. The inner shadow's 3σ outset plus dx=12 gives
    // x=[25,87]. The outer blur's 3σ outset is 48, so the accumulated
    // outer layer is x=[-23,135]. The unchanged x=[90,98) probe is thus
    // beyond every single-effect bound but inside the accumulated layer,
    // where radius 16 retains measurable 8-bit ink.
    assert!(
        handle.test_region_has_ink(90.0, 32.0, 8.0, 16.0),
        "the outer blur tail of the nested shadow layer must not be clipped to the largest single-effect bound"
    );

    handle.dispose();
    canvas.remove();
}
