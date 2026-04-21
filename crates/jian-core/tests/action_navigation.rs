use jian_core::action::capability::DummyCapabilityGate;
use jian_core::action::cancel::CancellationToken;
use jian_core::action::services::{
    NullClipboard, NullFeedback, NullNetworkClient, NullStorageBackend, RouteState, Router,
};
use jian_core::action::{default_registry, execute_list_shared, ActionContext};
use jian_core::expression::ExpressionCache;
use jian_core::signal::scheduler::Scheduler;
use jian_core::state::StateGraph;
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

struct RecordingRouter {
    ops: RefCell<Vec<String>>,
}

impl Router for RecordingRouter {
    fn current(&self) -> RouteState {
        RouteState {
            path: "/".into(),
            params: BTreeMap::new(),
            query: BTreeMap::new(),
            stack: vec!["/".into()],
        }
    }
    fn push(&self, p: &str) {
        self.ops.borrow_mut().push(format!("push:{}", p));
    }
    fn replace(&self, p: &str) {
        self.ops.borrow_mut().push(format!("replace:{}", p));
    }
    fn pop(&self) {
        self.ops.borrow_mut().push("pop".into());
    }
    fn reset(&self, p: &str) {
        self.ops.borrow_mut().push(format!("reset:{}", p));
    }
}

fn setup(router: Rc<dyn Router>) -> (Rc<StateGraph>, ActionContext) {
    let sched = Rc::new(Scheduler::new());
    let state = Rc::new(StateGraph::new(sched.clone()));
    let ctx = ActionContext {
        state: state.clone(),
        scheduler: sched,
        event: None,
        locals: RefCell::new(BTreeMap::new()),
        page_id: None,
        node_id: None,
        network: Rc::new(NullNetworkClient),
        storage: Rc::new(NullStorageBackend),
        router,
        feedback: Rc::new(NullFeedback),
        async_fb: Rc::new(NullFeedback),
        clipboard: Rc::new(NullClipboard),
        capabilities: Rc::new(DummyCapabilityGate),
        expr_cache: Rc::new(ExpressionCache::new()),
        cancel: CancellationToken::new(),
        warnings: RefCell::new(Vec::new()),
    };
    (state, ctx)
}

#[test]
fn push_literal_path() {
    let rec = Rc::new(RecordingRouter {
        ops: RefCell::new(Vec::new()),
    });
    let (_s, ctx) = setup(rec.clone());
    let reg = default_registry();
    let list = json!([{"push": "\"/detail/42\""}]);
    execute_list_shared(&reg, &list, &ctx);
    assert_eq!(rec.ops.borrow().as_slice(), ["push:/detail/42"]);
}

#[test]
fn push_with_template() {
    let rec = Rc::new(RecordingRouter {
        ops: RefCell::new(Vec::new()),
    });
    let (state, ctx) = setup(rec.clone());
    state.app_set("id", json!(99));
    let reg = default_registry();
    let list = json!([{"push": "`/detail/${$app.id}`"}]);
    execute_list_shared(&reg, &list, &ctx);
    assert_eq!(rec.ops.borrow().as_slice(), ["push:/detail/99"]);
}

#[test]
fn pop_no_body() {
    let rec = Rc::new(RecordingRouter {
        ops: RefCell::new(Vec::new()),
    });
    let (_s, ctx) = setup(rec.clone());
    let reg = default_registry();
    execute_list_shared(&reg, &json!([{"pop": null}]), &ctx);
    assert_eq!(rec.ops.borrow().as_slice(), ["pop"]);
}

#[test]
fn reset_nav_string() {
    let rec = Rc::new(RecordingRouter {
        ops: RefCell::new(Vec::new()),
    });
    let (_s, ctx) = setup(rec.clone());
    let reg = default_registry();
    execute_list_shared(&reg, &json!([{"reset": "\"/\""}]), &ctx);
    assert_eq!(rec.ops.borrow().as_slice(), ["reset:/"]);
}

#[test]
fn reset_scope_still_works() {
    let rec = Rc::new(RecordingRouter {
        ops: RefCell::new(Vec::new()),
    });
    let (state, ctx) = setup(rec.clone());
    state.app_set("x", json!(1));
    let reg = default_registry();
    execute_list_shared(&reg, &json!([{"reset": "$app"}]), &ctx);
    assert!(state.app_get("x").is_none());
    // Router should not have been called.
    assert!(rec.ops.borrow().is_empty());
}
