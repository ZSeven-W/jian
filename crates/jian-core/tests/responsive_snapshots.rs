use jian_core::document::loader;
use jian_core::layout::LayoutEngine;
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
