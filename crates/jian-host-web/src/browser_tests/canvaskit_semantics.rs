use crate::CanvasKitBackend;
use jian_core::geometry::{point, rect, size};
use jian_core::layout::measure::{FontStyleKind, MeasureBackend, MeasureRequest, StyledRun};
use jian_core::render::{
    collect_rich_draws_with_state, BorderRadii, DrawOp, ImageSource, RenderBackend, ShaderSpec,
    ShaderUniform, StrokeOp, TextAlign, TextRun,
};
use jian_core::scene::Color;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test;

fn canvas() -> web_sys::HtmlCanvasElement {
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas = document
        .create_element("canvas")
        .unwrap()
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();
    canvas
}

fn close(actual: u8, expected: u8, tolerance: u8) -> bool {
    actual.abs_diff(expected) <= tolerance
}

#[wasm_bindgen_test(async)]
async fn canvaskit_runtime_fonts_are_isolated_per_backend() {
    super::tests::ensure_canvaskit();
    let first = CanvasKitBackend::load(canvas(), "/assets/canvaskit/")
        .await
        .unwrap();
    assert_eq!(first.registered_font_count(), 2);
    first
        .font_registry()
        .register(
            "first-mount-only",
            include_bytes!("../../assets/fonts/Roboto-Regular.ttf"),
        )
        .unwrap();
    assert_eq!(first.registered_font_count(), 3);

    let second = CanvasKitBackend::load(canvas(), "/assets/canvaskit/")
        .await
        .unwrap();
    assert_eq!(second.registered_font_count(), 2);
    drop(first);
    assert_eq!(second.registered_font_count(), 2);
}

#[wasm_bindgen_test(async)]
async fn missing_and_invalid_images_paint_the_native_placeholder() {
    super::tests::ensure_canvaskit();
    let mut backend = CanvasKitBackend::load(canvas(), "/assets/canvaskit/")
        .await
        .unwrap();
    let mut surface = backend.new_surface(size(136.0, 40.0));
    backend.begin_frame(&mut surface, 0x00000000);
    backend.draw(&DrawOp::Image {
        source: ImageSource::Url("missing:image".into()),
        dst: rect(0.0, 0.0, 32.0, 32.0),
        opacity: 0.5,
    });
    backend.draw(&DrawOp::Image {
        source: ImageSource::Bytes(std::sync::Arc::new(b"not an image".to_vec())),
        dst: rect(40.0, 0.0, 32.0, 32.0),
        opacity: 1.0,
    });
    backend.draw(&DrawOp::Image {
        source: ImageSource::DataUrl("data:image/png;base64,bm90IGFuIGltYWdl".into()),
        dst: rect(80.0, 0.0, 32.0, 32.0),
        opacity: 1.0,
    });
    backend.end_frame(&mut surface);

    let missing = surface.read_pixel(12, 12);
    assert!(close(missing[0], 128, 3) && close(missing[3], 128, 3));
    let invalid = surface.read_pixel(52, 12);
    assert!(close(invalid[0], 128, 3) && invalid[3] > 250);
    let invalid_data_url = surface.read_pixel(92, 12);
    assert!(close(invalid_data_url[0], 128, 3) && invalid_data_url[3] > 250);
}

#[wasm_bindgen_test(async)]
async fn shader_uniforms_bind_by_name_and_fill_opacity_is_honored() {
    super::tests::ensure_canvaskit();
    let mut backend = CanvasKitBackend::load(canvas(), "/assets/canvaskit/")
        .await
        .unwrap();
    let mut surface = backend.new_surface(size(96.0, 40.0));
    let source = "uniform float4 color; uniform float strength; half4 main(float2 p) { return half4(color * strength); }";
    backend.begin_frame(&mut surface, 0x00000000);
    backend.draw(&DrawOp::ShaderRect {
        rect: rect(0.0, 0.0, 32.0, 32.0),
        radii: BorderRadii::zero(),
        shader: ShaderSpec {
            sksl: source.into(),
            // Deliberately reverse declaration order. An authored-order
            // flattening paints the wrong bytes here.
            uniforms: vec![
                ShaderUniform {
                    name: "strength".into(),
                    values: vec![1.0],
                },
                ShaderUniform {
                    name: "ignored".into(),
                    values: vec![9.0, 9.0],
                },
                ShaderUniform {
                    name: "color".into(),
                    values: vec![0.0, 1.0],
                },
                ShaderUniform {
                    name: "color".into(),
                    values: vec![1.0, 0.0, 0.0, 1.0],
                },
            ],
            opacity: 1.0,
            fallback: Color::rgb(1, 2, 3),
        },
        stroke: None,
    });
    backend.draw(&DrawOp::ShaderRect {
        rect: rect(40.0, 0.0, 32.0, 32.0),
        radii: BorderRadii::zero(),
        shader: ShaderSpec {
            sksl: source.into(),
            uniforms: vec![
                ShaderUniform {
                    name: "color".into(),
                    values: vec![0.0, 1.0, 0.0, 1.0],
                },
                ShaderUniform {
                    name: "strength".into(),
                    values: vec![1.0],
                },
            ],
            opacity: 0.25,
            fallback: Color::rgb(1, 2, 3),
        },
        stroke: Some(StrokeOp {
            color: Color::rgb(0, 0, 255),
            width: 4.0,
        }),
    });
    backend.end_frame(&mut surface);

    let named = surface.read_pixel(12, 12);
    assert!(named[0] > 245 && named[1] < 8 && named[2] < 8);
    let translucent = surface.read_pixel(52, 12);
    assert!(translucent[1] > 245 && close(translucent[3], 64, 4));
    let stroke = surface.read_pixel(41, 16);
    assert!(stroke[2] > 200 && stroke[3] > 245);
}

#[wasm_bindgen_test(async)]
async fn cjk_coverage_and_rich_render_share_measurement_styles() {
    super::tests::ensure_canvaskit();
    let mut backend = CanvasKitBackend::load(canvas(), "/assets/canvaskit/")
        .await
        .unwrap();
    assert!(backend.font_registry().covers_text("Roboto", "Jian 你好"));

    let mut runtime = jian_core::Runtime::new();
    runtime
        .load_str(
            r##"{"version":"1.2","responsive":true,"children":[{"type":"text","id":"copy","content":[{"text":"Jian ","fontFamily":"Roboto","fontSize":18,"fontWeight":400},{"text":"你好 styled wrapping text","fontFamily":"Roboto","fontSize":28,"fontWeight":700,"fontStyle":"italic","fill":"#ff0000"}],"fontFamily":"Roboto","fontSize":14,"fontWeight":300,"fontStyle":"normal","letterSpacing":2,"lineHeight":1.25,"textGrowth":"fixed-width","width":"fill_container","fill":[{"type":"solid","color":"#0000ff"}]}]}"##,
        )
        .unwrap();
    let key = runtime.document.as_ref().unwrap().tree.get("copy").unwrap();
    runtime
        .build_layout_with(std::rc::Rc::new(backend.measure_backend()), (360.0, 300.0))
        .unwrap();
    let wide_height = runtime.layout.node_rect(key).unwrap().size.height;
    runtime
        .build_layout_with(std::rc::Rc::new(backend.measure_backend()), (115.0, 300.0))
        .unwrap();
    let narrow_height = runtime.layout.node_rect(key).unwrap().size.height;
    assert!(narrow_height > 60.0 && narrow_height > wide_height);

    let rich = collect_rich_draws_with_state(
        runtime.document.as_ref().unwrap(),
        &runtime.layout,
        &runtime.state,
    );
    let (op_index, spans) = rich.text_runs.first().expect("text metadata");
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].font_size, 18.0);
    assert_eq!(spans[1].font_size, 28.0);
    assert_eq!(spans[1].font_weight, 700);
    assert!(spans[1].italic);
    assert_eq!(spans[1].letter_spacing, 2.0);
    assert_eq!(spans[1].color, Color::rgb(255, 0, 0));

    let mut surface = backend.new_surface(size(115.0, 300.0));
    backend.begin_frame(&mut surface, 0xffffffff);
    for (index, op) in rich.ops.iter().enumerate() {
        if index == *op_index {
            let DrawOp::Text(run) = op else {
                panic!("metadata must index text")
            };
            backend.draw_text_runs(run, spans);
        } else {
            backend.draw(op);
        }
    }
    backend.end_frame(&mut surface);
    assert!(surface.region_has_ink(0, 0, 115, 120));

    let measure_runs = [
        StyledRun {
            text: "Jian ",
            font_family: Some("Roboto"),
            font_size: 18.0,
            font_weight: 400,
            font_style: FontStyleKind::Normal,
            letter_spacing: 2.0,
        },
        StyledRun {
            text: "你好 styled wrapping text",
            font_family: Some("Roboto"),
            font_size: 28.0,
            font_weight: 700,
            font_style: FontStyleKind::Italic,
            letter_spacing: 2.0,
        },
    ];
    let measured = backend.measure_backend().measure(&MeasureRequest {
        runs: &measure_runs,
        line_height: 1.25,
        max_width: Some(115.0),
    });
    assert!(measured.line_count > 1 && measured.height > 60.0);
    assert!((surface.last_text_width() - measured.width).abs() <= 1.0);
    assert!((narrow_height - measured.height).abs() <= 1.0);

    // Plain DrawOp text retains its protected native-compatible behavior.
    backend.begin_frame(&mut surface, 0xffffffff);
    backend.draw(&DrawOp::Text(TextRun {
        content: "plain".into(),
        font_family: "Roboto".into(),
        font_size: 14.0,
        font_weight: 400,
        color: Color::rgb(0, 0, 0),
        origin: point(0.0, 0.0),
        max_width: 115.0,
        align: TextAlign::Start,
        line_height: 1.2,
    }));
    backend.end_frame(&mut surface);
}

#[wasm_bindgen_test(async)]
async fn auto_and_fixed_width_height_render_with_their_unwrapped_measurement() {
    super::tests::ensure_canvaskit();
    let mut backend = CanvasKitBackend::load(canvas(), "/assets/canvaskit/")
        .await
        .unwrap();
    let runs = [StyledRun {
        text: "Jian unwrapped agreement text 你好",
        font_family: Some("Roboto"),
        font_size: 24.0,
        font_weight: 400,
        font_style: FontStyleKind::Normal,
        letter_spacing: 1.0,
    }];
    let natural = backend.measure_backend().measure(&MeasureRequest {
        runs: &runs,
        line_height: 1.2,
        max_width: None,
    });
    let constrained = backend.measure_backend().measure(&MeasureRequest {
        runs: &runs,
        line_height: 1.2,
        max_width: Some(115.0),
    });
    assert_eq!(natural.line_count, 1);
    assert!(constrained.line_count > 1);

    for growth in ["fixed-width-height", "auto"] {
        let raw = serde_json::json!({
            "version": "1.2",
            "responsive": true,
            "children": [{
                "type": "text",
                "id": "copy",
                "content": "Jian unwrapped agreement text 你好",
                "fontFamily": "Roboto",
                "fontSize": 24,
                "letterSpacing": 1,
                "lineHeight": 1.2,
                "textGrowth": growth,
                "width": 115,
            }],
        })
        .to_string();
        let mut runtime = jian_core::Runtime::new();
        runtime.load_str(&raw).unwrap();
        runtime
            .build_layout_with(std::rc::Rc::new(backend.measure_backend()), (360.0, 160.0))
            .unwrap();
        let rich = collect_rich_draws_with_state(
            runtime.document.as_ref().unwrap(),
            &runtime.layout,
            &runtime.state,
        );
        let (op_index, spans) = rich.text_runs.first().expect("text metadata");
        let DrawOp::Text(run) = &rich.ops[*op_index] else {
            panic!("metadata must index text")
        };
        let mut surface = backend.new_surface(size(360.0, 160.0));
        backend.begin_frame(&mut surface, 0xffffffff);
        backend.draw_text_runs(run, spans);
        backend.end_frame(&mut surface);

        assert!(
            (surface.last_text_width() - natural.width).abs() <= 1.0,
            "{growth} rendered width {} disagrees with natural measure {}",
            surface.last_text_width(),
            natural.width,
        );
    }
}
