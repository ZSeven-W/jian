use jian_core::document::loader;
use jian_core::layout::LayoutEngine;
use jian_core::render::{collect_draws_with_state, DrawOp};
use jian_core::signal::scheduler::Scheduler;
use jian_core::state::StateGraph;
use jian_core::Runtime;
use jian_ops_schema::load_str;
use std::collections::BTreeMap;
use std::rc::Rc;

const LEGACY_FIXTURES: &[&str] = &[
    "minimal.op",
    "rectangle.op",
    "pages.op",
    "image.op",
    "nested-frame.op",
    "with-variables.op",
    "full-jian-extensions.op",
];

fn corpus_source(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../jian-ops-schema/tests/corpus/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn rect_bits(runtime: &Runtime) -> BTreeMap<String, [u32; 4]> {
    let document = runtime.document.as_ref().unwrap();
    document
        .tree
        .by_id
        .iter()
        .map(|(id, key)| {
            let rect = runtime.layout.node_rect(*key).unwrap();
            (
                id.clone(),
                [
                    rect.origin.x.to_bits(),
                    rect.origin.y.to_bits(),
                    rect.size.width.to_bits(),
                    rect.size.height.to_bits(),
                ],
            )
        })
        .collect()
}

#[test]
fn non_responsive_documents_are_bit_identical() {
    for fixture in LEGACY_FIXTURES {
        let source = corpus_source(fixture);
        let schema = load_str(&source).unwrap().value;
        let state = StateGraph::new(Rc::new(Scheduler::new()));
        let document = loader::build(schema.clone(), &state).unwrap();
        let mut legacy = LayoutEngine::new();
        let roots = legacy.build(&document.tree).unwrap();
        for root in roots {
            legacy.compute(root, (800.0, 600.0)).unwrap();
        }
        let legacy_bits: BTreeMap<String, [u32; 4]> = document
            .tree
            .by_id
            .iter()
            .map(|(id, key)| {
                let rect = legacy.node_rect(*key).unwrap();
                (
                    id.clone(),
                    [
                        rect.origin.x.to_bits(),
                        rect.origin.y.to_bits(),
                        rect.size.width.to_bits(),
                        rect.size.height.to_bits(),
                    ],
                )
            })
            .collect();

        let mut gated = Runtime::new_from_document(schema).unwrap();
        gated.build_layout((800.0, 600.0)).unwrap();
        assert_eq!(legacy_bits, rect_bits(&gated), "fixture {fixture}");
    }
}

#[test]
fn non_responsive_document_ignores_authored_min_max_fields() {
    let with_limits = r#"{"version":"1.1","children":[
        {"type":"frame","id":"root","width":100,"height":100,"children":[
            {"type":"rectangle","id":"c","width":30,"height":10,"minWidth":80}]}]}"#;
    let without_limits = r#"{"version":"1.1","children":[
        {"type":"frame","id":"root","width":100,"height":100,"children":[
            {"type":"rectangle","id":"c","width":30,"height":10}]}]}"#;
    let layout = |source: &str| {
        let schema = load_str(source).unwrap().value;
        let mut runtime = Runtime::new_from_document(schema).unwrap();
        runtime.build_layout((100.0, 100.0)).unwrap();
        rect_bits(&runtime)
    };
    assert_eq!(layout(with_limits), layout(without_limits));

    let explicitly_false = with_limits.replace(
        r#""version":"1.1""#,
        r#""version":"1.1","responsive":false"#,
    );
    assert_eq!(layout(&explicitly_false), layout(without_limits));
}

#[test]
fn responsive_layout_binding_reflows_through_the_real_pump() {
    let schema = load_str(
        r#"{"version":"1.2","responsive":true,
        "state":{"w":{"type":"int","default":20}},
        "children":[{"type":"frame","id":"root","width":100,"height":100,"children":[
          {"type":"rectangle","id":"box","width":10,"height":10,
           "bindings":{"width":"$app.w"}}]}]}"#,
    )
    .unwrap()
    .value;
    let mut runtime = Runtime::new_from_document(schema).unwrap();
    runtime.build_layout((100.0, 100.0)).unwrap();
    let key = runtime.document.as_ref().unwrap().tree.get("box").unwrap();
    assert_eq!(runtime.layout.node_rect(key).unwrap().size.width, 20.0);

    runtime.state.app_set("w", serde_json::json!(60));
    runtime.scheduler.flush();
    assert!(runtime.pump(1).needs_paint);
    assert_eq!(runtime.layout.node_rect(key).unwrap().size.width, 60.0);
}

#[test]
fn responsive_render_waits_for_bound_geometry_install() {
    let schema = load_str(
        r##"{"version":"1.2","responsive":true,
        "state":{"x":{"type":"int","default":5}},
        "children":[{"type":"frame","id":"root","width":100,"height":100,"children":[
          {"type":"rectangle","id":"box","x":0,"width":10,"height":10,
           "fill":[{"type":"solid","color":"#ff0000"}],"bindings":{"x":"$app.x"}}]}]}"##,
    )
    .unwrap()
    .value;
    let mut runtime = Runtime::new_from_document(schema).unwrap();
    runtime.build_layout((100.0, 100.0)).unwrap();
    runtime.state.app_set("x", serde_json::json!(40));
    runtime.scheduler.flush();

    let rect_x = collect_draws_with_state(
        runtime.document.as_ref().unwrap(),
        &runtime.layout,
        &runtime.state,
    )
    .into_iter()
    .find_map(|draw| match draw {
        DrawOp::Rect { rect, .. } if rect.size.width == 10.0 => Some(rect.origin.x),
        _ => None,
    })
    .unwrap();
    assert_eq!(rect_x, 5.0);

    runtime.pump(1);
    assert_eq!(
        runtime
            .layout
            .node_rect(runtime.document.as_ref().unwrap().tree.get("box").unwrap())
            .unwrap()
            .origin
            .x,
        40.0
    );
}

fn responsive_rects(width: f32, height: f32) -> BTreeMap<String, [f32; 4]> {
    let source = std::fs::read_to_string(format!(
        "{}/tests/fixtures/responsive_all_constraints.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let schema = load_str(&source).unwrap().value;
    let mut runtime = Runtime::new_from_document(schema).unwrap();
    runtime.build_layout((width, height)).unwrap();
    let document = runtime.document.as_ref().unwrap();
    document
        .tree
        .by_id
        .iter()
        .map(|(id, key)| {
            let rect = runtime.layout.node_rect(*key).unwrap();
            (
                id.clone(),
                [
                    rect.origin.x,
                    rect.origin.y,
                    rect.size.width,
                    rect.size.height,
                ],
            )
        })
        .collect()
}

#[test]
fn responsive_fixture_at_three_widths() {
    for width in [320.0, 768.0, 1280.0] {
        insta::assert_yaml_snapshot!(
            format!("all_constraints_{width:.0}"),
            responsive_rects(width, 800.0)
        );
    }
}
