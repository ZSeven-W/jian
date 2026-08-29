use jian_core::action::cancel::CancellationToken;
use jian_core::action::capability::DummyCapabilityGate;
use jian_core::action::services::{
    NullClipboard, NullFeedback, NullNetworkClient, NullRouter, NullStorageBackend,
};
use jian_core::action::{default_registry, execute_list_async, ActionContext};
fn execute_list_blocking_for_test(
    reg: &std::rc::Rc<std::cell::RefCell<jian_core::action::ActionRegistry>>,
    list: &serde_json::Value,
    ctx: &ActionContext,
) -> jian_core::action::ExecOutcome {
    let registry = reg.borrow();
    futures::executor::block_on(execute_list_async(&registry, list, ctx))
}
use jian_core::expression::ExpressionCache;
use jian_core::signal::scheduler::Scheduler;
use jian_core::state::StateGraph;
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

fn setup() -> (Rc<Scheduler>, Rc<StateGraph>, ActionContext) {
    let sched = Rc::new(Scheduler::new());
    let state = Rc::new(StateGraph::new(sched.clone()));
    let ctx = ActionContext {
        state: state.clone(),
        scheduler: sched.clone(),
        clock: None,
        document_generation: 0,
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
        platform: Rc::new(jian_core::action::services::NullPlatform),
        capabilities: Rc::new(DummyCapabilityGate),
        policy: None,
        effect_sink: std::rc::Rc::new(jian_core::action::services::effect_sink::NullEffectSink),
        activation: None,
        logic: Rc::new(jian_core::logic::NullLogicProvider),
        expr_cache: Rc::new(ExpressionCache::new()),
        cancel: CancellationToken::new(),
        warnings: RefCell::new(Vec::new()),
    };
    (sched, state, ctx)
}

#[test]
fn if_then_branch_runs() {
    let (_s, state, ctx) = setup();
    state.app_set("flag", json!(true));
    state.app_set("count", json!(0));
    let reg = default_registry();
    let list = json!([{
        "if": {
            "expr": "$app.flag",
            "then": [{"set": {"$app.count": "42"}}]
        }
    }]);
    let out = execute_list_blocking_for_test(&reg, &list, &ctx);
    assert!(out.result.is_ok(), "{:?}", out.result);
    assert_eq!(state.app_get("count").unwrap().as_i64(), Some(42));
}

#[test]
fn if_else_branch_runs() {
    let (_s, state, ctx) = setup();
    state.app_set("flag", json!(false));
    state.app_set("count", json!(0));
    let reg = default_registry();
    let list = json!([{
        "if": {
            "expr": "$app.flag",
            "then": [{"set": {"$app.count": "1"}}],
            "else": [{"set": {"$app.count": "99"}}]
        }
    }]);
    execute_list_blocking_for_test(&reg, &list, &ctx);
    assert_eq!(state.app_get("count").unwrap().as_i64(), Some(99));
}

#[test]
fn abort_stops_remaining_chain() {
    let (_s, state, ctx) = setup();
    state.app_set("a", json!(0));
    state.app_set("b", json!(0));
    let reg = default_registry();
    let list = json!([
        {"set": {"$app.a": "1"}},
        {"abort": null},
        {"set": {"$app.b": "1"}}
    ]);
    let out = execute_list_blocking_for_test(&reg, &list, &ctx);
    assert!(out.result.is_err());
    assert_eq!(state.app_get("a").unwrap().as_i64(), Some(1));
    assert_eq!(state.app_get("b").unwrap().as_i64(), Some(0));
}

#[test]
fn for_each_iterates_with_item() {
    let (_s, state, ctx) = setup();
    state.app_set("items", json!([1, 2, 3]));
    state.app_set("sum", json!(0));
    let reg = default_registry();
    let list = json!([{
        "for_each": {
            "in": "$app.items",
            "as": "item",
            "do": [{"set": {"$app.sum": "$app.sum + $item"}}]
        }
    }]);
    let out = execute_list_blocking_for_test(&reg, &list, &ctx);
    assert!(out.result.is_ok(), "{:?}", out.result);
    assert_eq!(state.app_get("sum").unwrap().as_i64(), Some(6));
}

#[test]
fn for_each_respects_max_iter() {
    let (_s, state, ctx) = setup();
    let huge: Vec<i64> = (0..100_000).collect();
    state.app_set("items", json!(huge));
    let reg = default_registry();
    let list = json!([{"for_each": {"in": "$app.items", "as": "x", "do": []}}]);
    let out = execute_list_blocking_for_test(&reg, &list, &ctx);
    assert!(out.result.is_err());
}

#[test]
fn parallel_all_run() {
    let (_s, state, ctx) = setup();
    state.app_set("a", json!(0));
    state.app_set("b", json!(0));
    let reg = default_registry();
    let list = json!([{
        "parallel": [
            [{"set": {"$app.a": "1"}}],
            [{"set": {"$app.b": "2"}}]
        ]
    }]);
    let out = execute_list_blocking_for_test(&reg, &list, &ctx);
    assert!(out.result.is_ok());
    assert_eq!(state.app_get("a").unwrap().as_i64(), Some(1));
    assert_eq!(state.app_get("b").unwrap().as_i64(), Some(2));
}

#[test]
fn race_returns_first() {
    let (_s, state, ctx) = setup();
    state.app_set("winner", json!(""));
    let reg = default_registry();
    let list = json!([{
        "race": [
            [{"set": {"$app.winner": "\"a\""}}],
            [{"set": {"$app.winner": "\"b\""}}]
        ]
    }]);
    let out = execute_list_blocking_for_test(&reg, &list, &ctx);
    assert!(out.result.is_ok());
    // Both sync branches complete sequentially; `race` just picks the first
    // resolved. With sync actions the last-writer effectively wins but we
    // only assert the field is non-empty.
    assert!(!state
        .app_get("winner")
        .unwrap()
        .as_str()
        .unwrap()
        .is_empty());
}
