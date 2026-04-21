//! Integration: load counter.op, wire a binding from $app.count into a
//! mutable slot, increment, flush, and verify the slot updated.

use jian_core::expression::Expression;
use jian_core::{BindingEffect, Runtime};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[test]
fn counter_binding_propagates() {
    let mut rt = Runtime::new();
    rt.load_str(
        &std::fs::read_to_string(format!("{}/tests/counter.op", env!("CARGO_MANIFEST_DIR")))
            .unwrap(),
    )
    .unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();

    let state = rt.state.clone();
    let latest = Rc::new(RefCell::new(String::new()));
    let latest2 = latest.clone();
    let expr = Expression::compile("`Count: ${$app.count}`").unwrap();
    let _b = BindingEffect::new(
        &rt.effects,
        expr,
        state.clone(),
        None,
        None,
        move |v, _| *latest2.borrow_mut() = v.as_str().unwrap_or("").to_owned(),
    );

    assert_eq!(*latest.borrow(), "Count: 0");

    state.app_set("count", json!(42));
    rt.scheduler.flush();
    assert_eq!(*latest.borrow(), "Count: 42");

    state.app_set("count", json!(100));
    rt.scheduler.flush();
    assert_eq!(*latest.borrow(), "Count: 100");
}

#[test]
fn fresh_binding_does_not_leak_after_drop() {
    let mut rt = Runtime::new();
    rt.load_str(r#"{"version":"0.8.0","children":[]}"#).unwrap();
    let state = rt.state.clone();
    state.app_set("x", json!(0));

    let hits = Rc::new(Cell::new(0));
    {
        let hits2 = hits.clone();
        let expr = Expression::compile("$app.x").unwrap();
        let _b = BindingEffect::new(
            &rt.effects,
            expr,
            state.clone(),
            None,
            None,
            move |_, _| hits2.set(hits2.get() + 1),
        );
        // _b is dropped at end of block
    }

    let before = hits.get();
    state.app_set("x", json!(99));
    rt.scheduler.flush();
    assert_eq!(hits.get(), before, "effect after drop should not run");
}
