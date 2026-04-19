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
