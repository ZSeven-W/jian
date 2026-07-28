//! `layout: "none"` stacking semantics.
//!
//! A `layout: "none"` frame is the `.op` spelling of a Figma
//! "absolute position" frame: its children all share the frame's origin and
//! overlap, rather than flowing side by side. Before this was modelled
//! explicitly, `layout: "none"` fell through the flex-direction default and
//! became a plain row, so the canonical hero stack
//! `[overlay content, gradient scrim, photo]` rendered as adjacent columns.

use jian_core::document::{loader, RuntimeDocument};
use jian_core::geometry::Rect;
use jian_core::layout::LayoutEngine;
use jian_core::signal::scheduler::Scheduler;
use jian_core::state::StateGraph;
use jian_ops_schema::load_str;
use std::rc::Rc;

fn build(src: &str) -> RuntimeDocument {
    let schema = load_str(src).unwrap().value;
    let sched = Rc::new(Scheduler::new());
    let state = StateGraph::new(sched);
    loader::build(schema, &state).unwrap()
}

/// Compute at a phone viewport and return a lookup closure over node ids.
fn laid_out(src: &str) -> impl Fn(&str) -> Rect {
    let doc = build(src);
    let mut eng = LayoutEngine::new();
    let roots = eng.build(&doc.tree).unwrap();
    eng.compute(roots[0], (375.0, 812.0)).unwrap();
    move |id: &str| {
        eng.node_rect(doc.tree.get(id).expect("unknown id"))
            .expect("no rect")
    }
}

fn assert_rect(actual: Rect, expected: (f32, f32, f32, f32), what: &str) {
    let got = (
        actual.origin.x,
        actual.origin.y,
        actual.size.width,
        actual.size.height,
    );
    assert_eq!(got, expected, "{what}");
}

#[test]
fn stack_children_overlap_at_the_frame_origin() {
    // The reported hero: a 375x320 `layout:"none"` frame holding an overlay
    // content layer, a gradient scrim and the photo. All three must occupy
    // the whole frame at (0, 0). As a flex row the two `fill_container`
    // layers instead split the width ~187 each and sat side by side.
    let at = laid_out(
        r##"{
      "version":"0.8.0",
      "children":[{
        "type":"frame","id":"page","width":375,"height":812,"layout":"vertical",
        "children":[{
          "type":"frame","id":"hero","width":"fill_container","height":320,
          "layout":"none","clipContent":true,
          "children":[
            {"type":"frame","id":"overlay","width":"fill_container","height":320,
             "layout":"vertical"},
            {"type":"rectangle","id":"scrim","width":"fill_container","height":320},
            {"type":"image","id":"photo","x":0,"y":0,"width":375,"height":320,"src":"still.png"}
          ]
        }]
      }]
    }"##,
    );
    assert_rect(at("hero"), (0.0, 0.0, 375.0, 320.0), "hero");
    assert_rect(
        at("overlay"),
        (0.0, 0.0, 375.0, 320.0),
        "overlay content layer",
    );
    assert_rect(at("scrim"), (0.0, 0.0, 375.0, 320.0), "gradient scrim");
    assert_rect(at("photo"), (0.0, 0.0, 375.0, 320.0), "photo");
}

#[test]
fn stack_fill_container_resolves_against_the_content_box() {
    // `fill_container` inside a stack means "the parent's content box", not
    // "an equal share of a flex line" — so a lone fill child is inset by the
    // parent's padding on every side and a fixed sibling takes nothing away
    // from it.
    let at = laid_out(
        r##"{
      "version":"0.8.0",
      "children":[{
        "type":"frame","id":"card","width":200,"height":100,"layout":"none","padding":[10,20],
        "children":[
          {"type":"rectangle","id":"bg","width":"fill_container","height":"fill_container"},
          {"type":"rectangle","id":"chip","width":40,"height":16}
        ]
      }]
    }"##,
    );
    assert_rect(
        at("bg"),
        (20.0, 10.0, 160.0, 80.0),
        "fill child spans the content box",
    );
    assert_rect(
        at("chip"),
        (20.0, 10.0, 40.0, 16.0),
        "fixed sibling keeps its size",
    );
}

#[test]
fn stack_honours_explicit_offsets_without_moving_siblings() {
    // An authored `x`/`y` offsets that one layer from the frame origin and
    // leaves every other layer where it was — the defining property a flex
    // row could not provide, since each child consumed main-axis space.
    let at = laid_out(
        r##"{
      "version":"0.8.0",
      "children":[{
        "type":"frame","id":"page","width":375,"height":812,"layout":"vertical",
        "children":[
          {"type":"rectangle","id":"spacer","width":375,"height":50},
          {"type":"frame","id":"hero","width":"fill_container","height":120,"layout":"none",
           "children":[
             {"type":"rectangle","id":"bg","width":"fill_container","height":"fill_container"},
             {"type":"rectangle","id":"badge","x":16,"y":24,"width":32,"height":32},
             {"type":"rectangle","id":"dot","x":300,"y":8,"width":8,"height":8}
           ]}
        ]
      }]
    }"##,
    );
    assert_rect(at("hero"), (0.0, 50.0, 375.0, 120.0), "hero");
    assert_rect(at("bg"), (0.0, 50.0, 375.0, 120.0), "background layer");
    assert_rect(
        at("badge"),
        (16.0, 74.0, 32.0, 32.0),
        "badge at its own offset",
    );
    assert_rect(
        at("dot"),
        (300.0, 58.0, 8.0, 8.0),
        "dot unaffected by the badge",
    );
}

#[test]
fn absolute_child_keeps_its_fill_container_height() {
    // Regression: a child with an authored `x`/`y` is laid out out-of-flow,
    // where neither `align_self: Stretch` nor `flex_grow` (the two
    // `fill_container`-height remedies for flow children) has any effect.
    // Rewriting its height to `auto` therefore left it with no height source
    // and it measured 0 — an `x:0, y:0` scrim or photo vanished.
    let at = laid_out(
        r##"{
      "version":"0.8.0",
      "children":[{
        "type":"frame","id":"tile","width":120,"height":90,"layout":"none",
        "children":[
          {"type":"image","id":"shot","x":0,"y":0,
           "width":"fill_container","height":"fill_container","src":"p.png"}
        ]
      }]
    }"##,
    );
    assert_rect(at("shot"), (0.0, 0.0, 120.0, 90.0), "absolute fill photo");
}

#[test]
fn stack_parent_still_hugs_its_children() {
    // `fit_content` on a stack must keep working: the frame sizes to its
    // largest layer. Modelling the children as plain CSS `position:
    // absolute` would take them out of flow entirely and collapse a
    // `fit_content` badge to 0x0.
    let at = laid_out(
        r##"{
      "version":"0.8.0",
      "children":[{
        "type":"frame","id":"badge","width":"fit_content","height":"fit_content",
        "layout":"none","padding":[4,8],
        "children":[
          {"type":"rectangle","id":"wide","width":60,"height":12},
          {"type":"rectangle","id":"tall","width":20,"height":30}
        ]
      }]
    }"##,
    );
    assert_rect(
        at("badge"),
        (0.0, 0.0, 76.0, 38.0),
        "hug = widest + tallest + padding",
    );
    assert_rect(at("wide"), (8.0, 4.0, 60.0, 12.0), "wide layer");
    assert_rect(
        at("tall"),
        (8.0, 4.0, 20.0, 30.0),
        "tall layer shares the origin",
    );
}

#[test]
fn stack_alignment_centres_a_lone_layer() {
    // `layout:"none" + justifyContent:center + alignItems:center` is how
    // real designs author a circular icon badge around one glyph. Under the
    // old flex-row reading `justifyContent` was horizontal and `alignItems`
    // vertical; stacking must preserve exactly that placement.
    let at = laid_out(
        r##"{
      "version":"0.8.0",
      "children":[{
        "type":"frame","id":"icon","width":32,"height":32,"layout":"none",
        "justifyContent":"center","alignItems":"center",
        "children":[{"type":"rectangle","id":"glyph","width":12,"height":12}]
      }]
    }"##,
    );
    assert_rect(
        at("glyph"),
        (10.0, 10.0, 12.0, 12.0),
        "glyph centred in the badge",
    );
}

#[test]
fn stack_alignment_end_pins_to_the_far_corner() {
    let at = laid_out(
        r##"{
      "version":"0.8.0",
      "children":[{
        "type":"frame","id":"box","width":100,"height":60,"layout":"none",
        "justifyContent":"end","alignItems":"end",
        "children":[{"type":"rectangle","id":"tag","width":20,"height":10}]
      }]
    }"##,
    );
    assert_rect(
        at("tag"),
        (80.0, 50.0, 20.0, 10.0),
        "tag pinned bottom-right",
    );
}

#[test]
fn nested_stacks_compose() {
    // A stack inside a stack: the inner frame overlays the outer photo, and
    // its own child centres inside it. This is the thumbnail + play-button
    // pattern, which as nested flex rows pushed the button off the photo.
    let at = laid_out(
        r##"{
      "version":"0.8.0",
      "children":[{
        "type":"frame","id":"thumb","width":88,"height":88,"layout":"none",
        "children":[
          {"type":"rectangle","id":"photo","width":"fill_container","height":"fill_container"},
          {"type":"frame","id":"play","width":32,"height":32,"layout":"none",
           "justifyContent":"center","alignItems":"center",
           "children":[{"type":"rectangle","id":"tri","width":12,"height":12}]}
        ]
      }]
    }"##,
    );
    assert_rect(
        at("photo"),
        (0.0, 0.0, 88.0, 88.0),
        "photo fills the thumbnail",
    );
    assert_rect(
        at("play"),
        (0.0, 0.0, 32.0, 32.0),
        "play button overlays the photo",
    );
    assert_rect(
        at("tri"),
        (10.0, 10.0, 12.0, 12.0),
        "triangle centred in the button",
    );
}

#[test]
fn flow_siblings_are_untouched_by_the_stack_rules() {
    // Guard rail: a normal horizontal frame keeps flowing its children, so
    // the stack path can only ever fire for `layout: "none"`.
    let at = laid_out(
        r##"{
      "version":"0.8.0",
      "children":[{
        "type":"frame","id":"row","width":300,"height":40,"layout":"horizontal","gap":10,
        "children":[
          {"type":"rectangle","id":"a","width":100,"height":40},
          {"type":"rectangle","id":"b","width":100,"height":40}
        ]
      }]
    }"##,
    );
    assert_rect(at("a"), (0.0, 0.0, 100.0, 40.0), "first cell");
    assert_rect(
        at("b"),
        (110.0, 0.0, 100.0, 40.0),
        "second cell after the gap",
    );
}
