use jian_core::document::{loader, RuntimeDocument};
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

#[test]
fn text_growth_fixed_width_height_skips_wrap() {
    // A long string in a `fixed_width_height` text leaf must report
    // its natural single-line extent, even when the parent flex
    // would otherwise impose a narrow available width. The
    // estimator's natural width here is ~ `chars * fontSize * 0.58`.
    let doc = build(
        r##"{
      "version":"0.8.0",
      "children":[{
        "type":"frame","id":"row","width":120,"height":24,
        "layout":"horizontal",
        "children":[
          { "type":"text","id":"label",
            "content":"This sentence is intentionally too long to fit",
            "fontSize":16, "textGrowth":"fixed-width-height" }
        ]
      }]
    }"##,
    );
    let mut eng = LayoutEngine::new();
    let roots = eng.build(&doc.tree).unwrap();
    eng.compute(roots[0], (800.0, 600.0)).unwrap();
    let label = eng.node_rect(doc.tree.get("label").unwrap()).unwrap();
    assert!(
        label.size.width > 200.0,
        "fixed_width_height must report natural extent, got width={}",
        label.size.width,
    );
    // Single line: height ~= fontSize * 1.3 (default line_height).
    assert!(
        label.size.height < 16.0 * 1.4 + 0.5,
        "single-line text shouldn't wrap to 2+ rows, got height={}",
        label.size.height,
    );
}

#[test]
fn text_growth_fixed_width_honours_authored_width() {
    // FixedWidth on a node with an authored numeric width must
    // wrap to that width even when the parent column is wider.
    // Confirms Task 4's distinction from Auto: an auto-width
    // FixedWidth node would just stretch; an authored 60px
    // FixedWidth node wraps at 60.
    let doc = build(
        r##"{
      "version":"0.8.0",
      "children":[{
        "type":"frame","id":"col","width":400,"layout":"vertical",
        "children":[
          { "type":"text","id":"para",
            "width":60,
            "content":"This sentence is intentionally too long to fit",
            "fontSize":16, "textGrowth":"fixed-width" }
        ]
      }]
    }"##,
    );
    let mut eng = LayoutEngine::new();
    let roots = eng.build(&doc.tree).unwrap();
    eng.compute(roots[0], (800.0, 600.0)).unwrap();
    let para = eng.node_rect(doc.tree.get("para").unwrap()).unwrap();
    assert_eq!(
        para.size.width, 60.0,
        "fixed-width must respect authored 60, got width={}",
        para.size.width,
    );
    assert!(
        para.size.height > 16.0 * 1.4,
        "wrapping at 60 must produce 2+ rows, got height={}",
        para.size.height,
    );
}

#[test]
fn styled_segments_fan_out_into_runs() {
    // Schema-authored TextContent::Styled flows through
    // text_measure_for as one OwnedRun per segment. Concrete
    // observable: a styled mix of [400, 700] weights measures
    // strictly wider than the same total characters at uniform
    // 400 (the heuristic uses the heaviest weight ratio).
    let doc_uniform = build(
        r##"{
      "version":"0.8.0",
      "children":[{
        "type":"text","id":"t",
        "content":"Hello there friend",
        "fontSize":16
      }]
    }"##,
    );
    let doc_styled = build(
        r##"{
      "version":"0.8.0",
      "children":[{
        "type":"text","id":"t",
        "content":[
          { "text":"Hello ", "fontWeight":400 },
          { "text":"there ", "fontWeight":700 },
          { "text":"friend",  "fontWeight":400 }
        ],
        "fontSize":16
      }]
    }"##,
    );
    let mut eng = LayoutEngine::new();
    let roots = eng.build(&doc_uniform.tree).unwrap();
    eng.compute(roots[0], (800.0, 600.0)).unwrap();
    let uniform = eng.node_rect(doc_uniform.tree.get("t").unwrap()).unwrap();

    let mut eng2 = LayoutEngine::new();
    let roots2 = eng2.build(&doc_styled.tree).unwrap();
    eng2.compute(roots2[0], (800.0, 600.0)).unwrap();
    let styled = eng2.node_rect(doc_styled.tree.get("t").unwrap()).unwrap();

    // The estimator's weight ratio jumps 0.58 → 0.64 at weight >= 700,
    // a ~10% widen. Pin the styled run to *strictly wider* than 5%
    // over uniform-400 so a regression that collapses runs into a
    // single 400-weight measurement (effectively dropping the bold
    // segment) actually fails this test. Equality wouldn't.
    assert!(
        styled.size.width > uniform.size.width * 1.05,
        "styled mix-weight should be ≥5% wider than uniform 400 \
         (uniform={}, styled={})",
        uniform.size.width,
        styled.size.width,
    );
}

#[test]
fn text_growth_auto_wraps_to_available() {
    // Default (`auto`) text wraps when the available width is too
    // narrow, growing the row's height instead of the column's.
    let doc = build(
        r##"{
      "version":"0.8.0",
      "children":[{
        "type":"frame","id":"col","width":80,"layout":"vertical",
        "children":[
          { "type":"text","id":"para",
            "content":"This sentence is intentionally too long to fit",
            "fontSize":16 }
        ]
      }]
    }"##,
    );
    let mut eng = LayoutEngine::new();
    let roots = eng.build(&doc.tree).unwrap();
    eng.compute(roots[0], (800.0, 600.0)).unwrap();
    let para = eng.node_rect(doc.tree.get("para").unwrap()).unwrap();
    assert!(
        para.size.width <= 80.0 + 0.5,
        "auto wrap must respect column budget, got width={}",
        para.size.width,
    );
    assert!(
        para.size.height > 16.0 * 1.4,
        "wrapped text should occupy 2+ rows, got height={}",
        para.size.height,
    );
}

#[test]
fn fixed_size_rect() {
    let doc = build(
        r#"{
      "version":"0.8.0",
      "children":[{"type":"rectangle","id":"r","width":100,"height":50}]
    }"#,
    );
    let mut eng = LayoutEngine::new();
    let roots = eng.build(&doc.tree).unwrap();
    eng.compute(roots[0], (800.0, 600.0)).unwrap();
    let key = doc.tree.get("r").unwrap();
    let r = eng.node_rect(key).unwrap();
    assert_eq!(r.size.width, 100.0);
    assert_eq!(r.size.height, 50.0);
}

#[test]
fn horizontal_row_distributes_children() {
    let doc = build(
        r#"{
      "version":"0.8.0",
      "children":[{
        "type":"frame","id":"row","width":300,"height":40,
        "layout":"horizontal","gap":0,
        "children":[
          {"type":"rectangle","id":"a","width":100,"height":40},
          {"type":"rectangle","id":"b","width":200,"height":40}
        ]
      }]
    }"#,
    );
    let mut eng = LayoutEngine::new();
    let roots = eng.build(&doc.tree).unwrap();
    eng.compute(roots[0], (800.0, 600.0)).unwrap();
    let a = eng.node_rect(doc.tree.get("a").unwrap()).unwrap();
    let b = eng.node_rect(doc.tree.get("b").unwrap()).unwrap();
    assert_eq!(a.size.width, 100.0);
    assert_eq!(b.size.width, 200.0);
    assert!(b.origin.x >= 100.0);
}
