//! End-to-end: load a small `.op`, walk the layout/scene, and render
//! via `SkiaBackend` to a raster surface. PNG bytes are inspected for
//! a non-trivial size (not a byte-exact golden — golden PNGs land in a
//! later commit with a helper to re-bless them).

use jian_core::geometry::rect;
use jian_core::render::{DrawOp, Paint, RenderBackend};
use jian_core::scene::Color;
use jian_core::Runtime;
use jian_skia::SkiaBackend;

const COUNTER_OP: &str = r##"{
  "formatVersion": "1.0",
  "version": "1.0.0",
  "state": { "count": { "type": "int", "default": 0 } },
  "children": [
    { "type": "rectangle", "id": "btn", "width": 200, "height": 100,
      "fills": [{ "type": "solid", "color": "#1e88e5" }] }
  ]
}"##;

#[test]
fn counter_op_renders_to_png() {
    let mut rt = Runtime::new();
    rt.load_str(COUNTER_OP).unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();

    // Walk the layout and draw every rect with its fill colour.
    let mut backend = SkiaBackend::new();
    let mut surface = backend.new_surface(jian_core::geometry::size(800.0, 600.0));
    backend.begin_frame(&mut surface, 0xffffffff);

    let doc = rt.document.as_ref().unwrap();
    for (key, _node) in doc.tree.nodes.iter() {
        if let Some(r) = rt.layout.node_rect(key) {
            backend.draw_on(
                &mut surface,
                &DrawOp::Rect {
                    rect: rect(r.min_x(), r.min_y(), r.size.width, r.size.height),
                    paint: Paint::solid(Color::rgb(0x1e, 0x88, 0xe5)),
                },
            );
        }
    }

    backend.end_frame(&mut surface);

    let png = surface.encode_png().expect("encode_png");
    assert_eq!(&png[0..8], b"\x89PNG\r\n\x1a\n");
    assert!(png.len() > 500, "png should encode some content");
}
