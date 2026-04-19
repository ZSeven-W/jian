use jian_core::geometry::{point, size};
use jian_core::render::{CaptureBackend, DrawOp, RenderBackend, RenderCommand};
use jian_core::scene::Color;
use jian_core::Runtime;

fn counter_src() -> String {
    std::fs::read_to_string(format!("{}/tests/counter.op", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn load_layout_hit_spatial() {
    let mut rt = Runtime::new();
    rt.load_str(&counter_src()).unwrap();
    assert_eq!(rt.document.as_ref().unwrap().node_count(), 3);

    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();

    let hits = rt.spatial.hit(point(50.0, 50.0));
    assert!(!hits.is_empty(), "expected at least one node to be hit");

    let count = rt.state.app_get("count").unwrap();
    assert_eq!(count.as_i64(), Some(0));
}

#[test]
fn capture_backend_records_smoke() {
    let mut backend = CaptureBackend::new();
    let mut surface = backend.new_surface(size(800.0, 600.0));
    backend.begin_frame(&mut surface, 0xffffffff);
    backend.draw(&DrawOp::Rect {
        rect: jian_core::geometry::rect(10.0, 10.0, 100.0, 50.0),
        paint: jian_core::render::Paint::solid(Color::rgb(0, 0x66, 0xff)),
    });
    backend.end_frame(&mut surface);
    let cmds = backend.take();
    assert_eq!(cmds.len(), 3);
    assert!(matches!(cmds[0], RenderCommand::BeginFrame { .. }));
    assert!(matches!(cmds[1], RenderCommand::Draw(DrawOp::Rect { .. })));
    assert!(matches!(cmds[2], RenderCommand::EndFrame));
}

// --- Corpus pipeline tests (Plan 1 fixtures) -----------------------------

fn corpus_path(name: &str) -> String {
    format!(
        "{}/../jian-ops-schema/tests/corpus/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    )
}

fn load_corpus(name: &str) -> String {
    std::fs::read_to_string(corpus_path(name))
        .unwrap_or_else(|e| panic!("cannot read corpus {}: {}", name, e))
}

#[test]
fn corpus_minimal_pipeline() {
    let mut rt = Runtime::new();
    rt.load_str(&load_corpus("minimal.op")).unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
}

#[test]
fn corpus_nested_frame_pipeline() {
    let mut rt = Runtime::new();
    rt.load_str(&load_corpus("nested-frame.op")).unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    assert!(!rt.spatial.is_empty());
}

#[test]
fn corpus_with_variables_seeds_vars_scope() {
    let mut rt = Runtime::new();
    rt.load_str(&load_corpus("with-variables.op")).unwrap();
    assert!(rt.state.vars_get("bg").is_some());
    assert!(rt.state.vars_get("fg").is_some());
}

#[test]
fn corpus_full_jian_extensions_pipeline() {
    let mut rt = Runtime::new();
    rt.load_str(&load_corpus("full-jian-extensions.op"))
        .unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    assert_eq!(rt.state.app_get("count").unwrap().as_i64(), Some(0));
    assert_eq!(rt.state.app_get("target").unwrap().as_i64(), Some(10));
}
