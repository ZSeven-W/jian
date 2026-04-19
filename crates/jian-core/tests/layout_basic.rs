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
