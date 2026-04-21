use async_trait::async_trait;
use jian_core::action::cancel::CancellationToken;
use jian_core::action::capability::{Capability, DeclaredCapabilityGate, DummyCapabilityGate};
use jian_core::action::services::{
    HttpRequest, HttpResponse, NetworkClient, NullClipboard, NullFeedback, NullRouter,
    NullStorageBackend,
};
use jian_core::action::{default_registry, execute_list_shared, ActionContext};
use jian_core::expression::ExpressionCache;
use jian_core::signal::scheduler::Scheduler;
use jian_core::state::StateGraph;
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

struct FixtureNet {
    canned: RefCell<Vec<Result<HttpResponse, String>>>,
    calls: RefCell<Vec<HttpRequest>>,
}

#[async_trait(?Send)]
impl NetworkClient for FixtureNet {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, String> {
        self.calls.borrow_mut().push(req);
        self.canned
            .borrow_mut()
            .pop()
            .unwrap_or(Err("no canned response".into()))
    }
}

fn setup_with_net(
    net: Rc<dyn NetworkClient>,
    cap_allow_net: bool,
) -> (Rc<StateGraph>, ActionContext) {
    let sched = Rc::new(Scheduler::new());
    let state = Rc::new(StateGraph::new(sched.clone()));
    let cap: Rc<dyn jian_core::action::capability::CapabilityGate> = if cap_allow_net {
        Rc::new(DummyCapabilityGate)
    } else {
        let empty: [Capability; 0] = [];
        Rc::new(DeclaredCapabilityGate::from_iter(empty))
    };
    let ctx = ActionContext {
        state: state.clone(),
        scheduler: sched,
        event: None,
        locals: RefCell::new(BTreeMap::new()),
        page_id: None,
        node_id: None,
        network: net,
        storage: Rc::new(NullStorageBackend),
        router: Rc::new(NullRouter),
        feedback: Rc::new(NullFeedback),
        async_fb: Rc::new(NullFeedback),
        clipboard: Rc::new(NullClipboard),
        capabilities: cap,
        expr_cache: Rc::new(ExpressionCache::new()),
        cancel: CancellationToken::new(),
        warnings: RefCell::new(Vec::new()),
    };
    (state, ctx)
}

#[test]
fn fetch_writes_into_state() {
    let net = Rc::new(FixtureNet {
        canned: RefCell::new(vec![Ok(HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: json!({"name": "Alice"}),
        })]),
        calls: RefCell::new(vec![]),
    });
    let (state, ctx) = setup_with_net(net.clone(), true);
    let reg = default_registry();
    let list = json!([{"fetch": {
        "url": "\"/api/me\"",
        "into": "$app.user",
        "loading": "$app.isLoading"
    }}]);
    let out = execute_list_shared(&reg, &list, &ctx);
    assert!(out.result.is_ok(), "{:?}", out.result);
    assert_eq!(state.app_get("user").unwrap().0, json!({"name": "Alice"}));
    assert_eq!(state.app_get("isLoading").unwrap().as_bool(), Some(false));
    assert_eq!(net.calls.borrow()[0].url, "/api/me");
}

#[test]
fn fetch_on_error_runs() {
    let net = Rc::new(FixtureNet {
        canned: RefCell::new(vec![Err("boom".into())]),
        calls: RefCell::new(vec![]),
    });
    let (state, ctx) = setup_with_net(net.clone(), true);
    state.app_set("lastError", json!(""));
    let reg = default_registry();
    let list = json!([{"fetch": {
        "url": "\"/x\"",
        "on_error": [{"set": {"$app.lastError": "\"failed\""}}]
    }}]);
    execute_list_shared(&reg, &list, &ctx);
    assert_eq!(state.app_get("lastError").unwrap().as_str(), Some("failed"));
}

#[test]
fn fetch_without_capability_denied() {
    let net = Rc::new(FixtureNet {
        canned: RefCell::new(vec![]),
        calls: RefCell::new(vec![]),
    });
    let (_state, ctx) = setup_with_net(net.clone(), false);
    let reg = default_registry();
    let list = json!([{"fetch": {"url": "\"/x\""}}]);
    let out = execute_list_shared(&reg, &list, &ctx);
    assert!(matches!(
        out.result,
        Err(jian_core::action::error::ActionError::CapabilityDenied { .. })
    ));
    assert!(net.calls.borrow().is_empty());
}
