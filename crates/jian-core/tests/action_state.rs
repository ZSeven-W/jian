use jian_core::action::cancel::CancellationToken;
use jian_core::action::capability::DummyCapabilityGate;
use jian_core::action::services::{
    NullClipboard, NullFeedback, NullNetworkClient, NullRouter, NullStorageBackend,
};
use jian_core::action::{default_registry, execute_list_shared, ActionContext};
use jian_core::expression::ExpressionCache;
use jian_core::signal::scheduler::Scheduler;
use jian_core::state::StateGraph;
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

fn setup_ctx() -> (Rc<Scheduler>, Rc<StateGraph>, ActionContext) {
    let sched = Rc::new(Scheduler::new());
    let state = Rc::new(StateGraph::new(sched.clone()));
    let ctx = ActionContext {
        state: state.clone(),
        scheduler: sched.clone(),
        event: None,
        locals: RefCell::new(BTreeMap::new()),
        page_id: None,
        node_id: None,
        network: Rc::new(NullNetworkClient),
        ws_sessions: std::rc::Rc::new(std::cell::RefCell::new(std::collections::HashMap::new())),
        storage: Rc::new(NullStorageBackend),
        router: Rc::new(NullRouter),
        feedback: Rc::new(NullFeedback),
        async_fb: Rc::new(NullFeedback),
        clipboard: Rc::new(NullClipboard),
        capabilities: Rc::new(DummyCapabilityGate),
        logic: Rc::new(jian_core::logic::NullLogicProvider),
        expr_cache: Rc::new(ExpressionCache::new()),
        cancel: CancellationToken::new(),
        warnings: RefCell::new(Vec::new()),
    };
    (sched, state, ctx)
}

#[test]
fn set_shorthand() {
    let (_sched, state, ctx) = setup_ctx();
    state.app_set("count", json!(0));
    let reg = default_registry();
    let list = json!([{"set": {"$app.count": "$app.count + 1"}}]);
    let out = execute_list_shared(&reg, &list, &ctx);
    assert!(out.result.is_ok(), "{:?}", out.result);
    assert_eq!(state.app_get("count").unwrap().as_i64(), Some(1));
}

#[test]
fn set_full_form() {
    let (_s, state, ctx) = setup_ctx();
    state.app_set("x", json!(10));
    let reg = default_registry();
    let list = json!([{"set": {"target": "$app.x", "value": "$app.x * 2"}}]);
    let out = execute_list_shared(&reg, &list, &ctx);
    assert!(out.result.is_ok());
    assert_eq!(state.app_get("x").unwrap().as_i64(), Some(20));
}

#[test]
fn set_multi_pair() {
    let (_s, state, ctx) = setup_ctx();
    state.app_set("a", json!(1));
    state.app_set("b", json!(2));
    let reg = default_registry();
    let list = json!([{"set": {"$app.a": "10", "$app.b": "20"}}]);
    let out = execute_list_shared(&reg, &list, &ctx);
    assert!(out.result.is_ok());
    assert_eq!(state.app_get("a").unwrap().as_i64(), Some(10));
    assert_eq!(state.app_get("b").unwrap().as_i64(), Some(20));
}

#[test]
fn delete_nulls_target() {
    let (_s, state, ctx) = setup_ctx();
    state.app_set("tmp", json!("some data"));
    let reg = default_registry();
    let list = json!([{"delete": "$app.tmp"}]);
    execute_list_shared(&reg, &list, &ctx);
    assert!(state.app_get("tmp").unwrap().is_null());
}

#[test]
fn reset_clears_scope() {
    let (_s, state, ctx) = setup_ctx();
    state.app_set("a", json!(1));
    state.app_set("b", json!(2));
    let reg = default_registry();
    let list = json!([{"reset": "$app"}]);
    execute_list_shared(&reg, &list, &ctx);
    assert!(state.app_get("a").is_none());
    assert!(state.app_get("b").is_none());
}
