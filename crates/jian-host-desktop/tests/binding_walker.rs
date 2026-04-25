//! End-to-end coverage for `apply_bindings` walker extensions.
//!
//! `winit_click_demo.rs` style: build a runtime, mutate `$state.*`,
//! re-collect draws, assert the new render reflects the binding.

use jian_core::render::DrawOp;
use jian_core::scene::Color;
use jian_core::Runtime;
use jian_host_desktop::scene::collect_draws_with_state;
use jian_ops_schema::load_str;
use serde_json::json;

fn rt(src: &str) -> Runtime {
    let schema = load_str(src).unwrap().value;
    let mut rt = Runtime::new_from_document(schema).unwrap();
    rt.build_layout((400.0, 200.0)).unwrap();
    rt.rebuild_spatial();
    rt
}

#[test]
fn opacity_binding_flows_into_paint() {
    let rt = rt(
        r##"{ "formatVersion":"1.0", "version":"1.0.0", "id":"x",
              "app": { "name":"x","version":"1","id":"x" },
              "state": { "alpha": { "type":"float", "default":0.25 } },
              "children": [
                { "type":"rectangle", "id":"a", "width":100, "height":50,
                  "fill":[{ "type":"solid", "color":"#1e88e5" }],
                  "bindings": { "opacity": "$state.alpha" } }
              ]}"##,
    );
    let ops = collect_draws_with_state(rt.document.as_ref().unwrap(), &rt.layout, &rt.state);
    let opacity = ops
        .iter()
        .find_map(|op| match op {
            DrawOp::Rect { paint, .. } | DrawOp::RoundedRect { paint, .. } => Some(paint.opacity),
            _ => None,
        })
        .expect("rect emitted");
    assert!((opacity - 0.25).abs() < 1e-4, "opacity={opacity}");
}

#[test]
fn fill_color_binding_writes_first_solid_color() {
    let rt = rt(
        r##"{ "formatVersion":"1.0", "version":"1.0.0", "id":"x",
              "app": { "name":"x","version":"1","id":"x" },
              "state": { "tint": { "type":"string", "default":"#ff0000" } },
              "children": [
                { "type":"rectangle", "id":"a", "width":100, "height":50,
                  "fill":[{ "type":"solid", "color":"#ffffff" }],
                  "bindings": { "fill[0].color": "$state.tint" } }
              ]}"##,
    );
    let ops = collect_draws_with_state(rt.document.as_ref().unwrap(), &rt.layout, &rt.state);
    let fill = ops
        .iter()
        .find_map(|op| match op {
            DrawOp::Rect { paint, .. } | DrawOp::RoundedRect { paint, .. } => paint.fill,
            _ => None,
        })
        .expect("rect with fill");
    assert_eq!(fill, Color::from_hex("#ff0000").unwrap());
}

#[test]
fn visible_false_drops_node_and_subtree() {
    let rt = rt(
        r##"{ "formatVersion":"1.0", "version":"1.0.0", "id":"x",
              "app": { "name":"x","version":"1","id":"x" },
              "state": { "show": { "type":"bool", "default":true } },
              "children": [
                { "type":"frame", "id":"root", "width":200, "height":100,
                  "fill":[{ "type":"solid", "color":"#eeeeee" }],
                  "bindings": { "visible": "$state.show" },
                  "children": [
                    { "type":"rectangle", "id":"inner", "width":50, "height":50,
                      "fill":[{ "type":"solid", "color":"#1e88e5" }] }
                  ]}
              ]}"##,
    );

    // Initially visible: parent + child both render.
    let on = collect_draws_with_state(rt.document.as_ref().unwrap(), &rt.layout, &rt.state);
    assert_eq!(on.len(), 2, "both ops render when show=true");

    // Flip state — re-render reflects the binding without rebuilding layout.
    rt.state.app_set("show", json!(false));
    let off = collect_draws_with_state(rt.document.as_ref().unwrap(), &rt.layout, &rt.state);
    assert!(
        off.is_empty(),
        "visible:false on parent drops both parent and child, got {:?}",
        off
    );
}

#[test]
fn position_bindings_override_layout_rect() {
    let rt = rt(
        r##"{ "formatVersion":"1.0", "version":"1.0.0", "id":"x",
              "app": { "name":"x","version":"1","id":"x" },
              "state": {
                "tx": { "type":"float", "default":42 },
                "ty": { "type":"float", "default":17 }
              },
              "children": [
                { "type":"rectangle", "id":"a", "width":80, "height":40,
                  "fill":[{ "type":"solid", "color":"#1e88e5" }],
                  "bindings": { "x": "$state.tx", "y": "$state.ty" } }
              ]}"##,
    );
    let ops = collect_draws_with_state(rt.document.as_ref().unwrap(), &rt.layout, &rt.state);
    let bbox = ops
        .iter()
        .find_map(|op| match op {
            DrawOp::Rect { rect, .. } | DrawOp::RoundedRect { rect, .. } => Some(*rect),
            _ => None,
        })
        .expect("rect emitted");
    assert!((bbox.origin.x - 42.0).abs() < f32::EPSILON);
    assert!((bbox.origin.y - 17.0).abs() < f32::EPSILON);
}

#[test]
fn disabled_binding_writes_through_without_breaking_render() {
    // `disabled` is metadata for the action-surface state-gate; the
    // scene walker just propagates it through. This test confirms the
    // binding doesn't trip the walker even though no DrawOp is gated
    // on the flag.
    let rt = rt(
        r##"{ "formatVersion":"1.0", "version":"1.0.0", "id":"x",
              "app": { "name":"x","version":"1","id":"x" },
              "state": { "off": { "type":"bool", "default":true } },
              "children": [
                { "type":"rectangle", "id":"a", "width":80, "height":40,
                  "fill":[{ "type":"solid", "color":"#1e88e5" }],
                  "bindings": { "disabled": "$state.off" } }
              ]}"##,
    );
    let ops = collect_draws_with_state(rt.document.as_ref().unwrap(), &rt.layout, &rt.state);
    assert_eq!(ops.len(), 1, "rect still renders when disabled=true");
}
