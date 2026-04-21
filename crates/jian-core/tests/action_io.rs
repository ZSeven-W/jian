//! Storage + feedback + platform-stub + tier-3 call integration tests.

use async_trait::async_trait;
use jian_core::action::cancel::CancellationToken;
use jian_core::action::capability::{Capability, DeclaredCapabilityGate, DummyCapabilityGate};
use jian_core::action::services::{
    AsyncFeedback, ClipboardService, FeedbackLevel, FeedbackSink, NetworkClient, NullClipboard,
    NullNetworkClient, NullRouter, StorageBackend,
};
use jian_core::action::{default_registry, execute_list_shared, ActionContext};
use jian_core::expression::ExpressionCache;
use jian_core::signal::scheduler::Scheduler;
use jian_core::state::StateGraph;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

// ---- mocks ----

#[derive(Default)]
struct MemStore {
    map: RefCell<HashMap<String, Value>>,
}

#[async_trait(?Send)]
impl StorageBackend for MemStore {
    async fn get(&self, key: &str) -> Option<Value> {
        self.map.borrow().get(key).cloned()
    }
    async fn set(&self, key: &str, value: Value) {
        self.map.borrow_mut().insert(key.to_owned(), value);
    }
    async fn delete(&self, key: &str) {
        self.map.borrow_mut().remove(key);
    }
    async fn clear(&self) {
        self.map.borrow_mut().clear();
    }
    async fn keys(&self) -> Vec<String> {
        self.map.borrow().keys().cloned().collect()
    }
}

#[derive(Default)]
struct RecordingFeedback {
    toasts: RefCell<Vec<(String, FeedbackLevel, u32)>>,
    alerts: RefCell<Vec<(String, String)>>,
    confirm_response: std::cell::Cell<bool>,
}

impl FeedbackSink for RecordingFeedback {
    fn toast(&self, message: &str, level: FeedbackLevel, duration_ms: u32) {
        self.toasts
            .borrow_mut()
            .push((message.to_owned(), level, duration_ms));
    }
    fn alert(&self, title: &str, message: &str) {
        self.alerts
            .borrow_mut()
            .push((title.to_owned(), message.to_owned()));
    }
}

#[async_trait(?Send)]
impl AsyncFeedback for RecordingFeedback {
    async fn confirm(&self, _title: &str, _message: &str) -> bool {
        self.confirm_response.get()
    }
}

fn setup(
    net: Rc<dyn NetworkClient>,
    store: Rc<dyn StorageBackend>,
    fb: Rc<RecordingFeedback>,
    clipboard: Rc<dyn ClipboardService>,
    cap_set: &[Capability],
) -> (Rc<StateGraph>, ActionContext) {
    let sched = Rc::new(Scheduler::new());
    let state = Rc::new(StateGraph::new(sched.clone()));
    let cap: Rc<dyn jian_core::action::capability::CapabilityGate> = if cap_set.is_empty() {
        Rc::new(DummyCapabilityGate)
    } else {
        Rc::new(DeclaredCapabilityGate::from_iter(cap_set.iter().copied()))
    };
    let fb_sink: Rc<dyn FeedbackSink> = fb.clone();
    let fb_async: Rc<dyn AsyncFeedback> = fb.clone();
    let ctx = ActionContext {
        state: state.clone(),
        scheduler: sched,
        event: None,
        locals: RefCell::new(BTreeMap::new()),
        page_id: None,
        node_id: None,
        network: net,
        storage: store,
        router: Rc::new(NullRouter),
        feedback: fb_sink,
        async_fb: fb_async,
        clipboard,
        capabilities: cap,
        expr_cache: Rc::new(ExpressionCache::new()),
        cancel: CancellationToken::new(),
        warnings: RefCell::new(Vec::new()),
    };
    (state, ctx)
}

#[test]
fn storage_set_and_clear() {
    let store = Rc::new(MemStore::default());
    let fb = Rc::new(RecordingFeedback::default());
    let (_state, ctx) = setup(
        Rc::new(NullNetworkClient),
        store.clone(),
        fb,
        Rc::new(NullClipboard),
        &[],
    );
    let reg = default_registry();
    let list = json!([{"storage_set": {"theme": "\"dark\""}}]);
    let out = execute_list_shared(&reg, &list, &ctx);
    assert!(out.result.is_ok(), "{:?}", out.result);
    assert_eq!(store.map.borrow().get("theme"), Some(&json!("dark")));

    let list = json!([{"storage_clear": {"key": "theme"}}]);
    execute_list_shared(&reg, &list, &ctx);
    assert!(store.map.borrow().get("theme").is_none());
}

#[test]
fn storage_denied_without_capability() {
    let store = Rc::new(MemStore::default());
    let fb = Rc::new(RecordingFeedback::default());
    let (_s, ctx) = setup(
        Rc::new(NullNetworkClient),
        store,
        fb,
        Rc::new(NullClipboard),
        // Declared gate with no capabilities → all denied.
        &[Capability::Network],
    );
    let reg = default_registry();
    let list = json!([{"storage_set": {"x": "1"}}]);
    let out = execute_list_shared(&reg, &list, &ctx);
    assert!(matches!(
        out.result,
        Err(jian_core::action::error::ActionError::CapabilityDenied { .. })
    ));
}

#[test]
fn toast_and_alert_record() {
    let fb = Rc::new(RecordingFeedback::default());
    let (_s, ctx) = setup(
        Rc::new(NullNetworkClient),
        Rc::new(MemStore::default()),
        fb.clone(),
        Rc::new(NullClipboard),
        &[],
    );
    let reg = default_registry();
    let list = json!([
        {"toast": "\"hello\""},
        {"alert": {"title": "\"Title\"", "message": "\"Body\""}}
    ]);
    execute_list_shared(&reg, &list, &ctx);
    assert_eq!(fb.toasts.borrow()[0].0, "hello");
    assert_eq!(fb.alerts.borrow()[0], ("Title".into(), "Body".into()));
}

#[test]
fn confirm_branches_to_on_confirm() {
    let fb = Rc::new(RecordingFeedback::default());
    fb.confirm_response.set(true);
    let (state, ctx) = setup(
        Rc::new(NullNetworkClient),
        Rc::new(MemStore::default()),
        fb.clone(),
        Rc::new(NullClipboard),
        &[],
    );
    state.app_set("picked", json!(""));
    let reg = default_registry();
    let list = json!([{"confirm": {
        "title": "\"Delete?\"",
        "message": "\"Are you sure?\"",
        "on_confirm": [{"set": {"$app.picked": "\"yes\""}}],
        "on_cancel":  [{"set": {"$app.picked": "\"no\""}}]
    }}]);
    execute_list_shared(&reg, &list, &ctx);
    assert_eq!(state.app_get("picked").unwrap().as_str(), Some("yes"));
}

#[test]
fn confirm_cancel_runs_on_cancel() {
    let fb = Rc::new(RecordingFeedback::default());
    fb.confirm_response.set(false);
    let (state, ctx) = setup(
        Rc::new(NullNetworkClient),
        Rc::new(MemStore::default()),
        fb.clone(),
        Rc::new(NullClipboard),
        &[],
    );
    state.app_set("picked", json!(""));
    let reg = default_registry();
    let list = json!([{"confirm": {
        "title": "\"Delete?\"",
        "message": "\"Sure?\"",
        "on_confirm": [{"set": {"$app.picked": "\"yes\""}}],
        "on_cancel":  [{"set": {"$app.picked": "\"no\""}}]
    }}]);
    execute_list_shared(&reg, &list, &ctx);
    assert_eq!(state.app_get("picked").unwrap().as_str(), Some("no"));
}

#[test]
fn platform_stubs_warn() {
    let fb = Rc::new(RecordingFeedback::default());
    let (_s, ctx) = setup(
        Rc::new(NullNetworkClient),
        Rc::new(MemStore::default()),
        fb.clone(),
        Rc::new(NullClipboard),
        &[],
    );
    let reg = default_registry();
    let list = json!([{"share": {"url": "https://example.com"}}]);
    let out = execute_list_shared(&reg, &list, &ctx);
    assert!(out.result.is_ok());
    assert!(!out.warnings.is_empty());
}

#[test]
fn call_null_provider_goes_to_on_error() {
    let fb = Rc::new(RecordingFeedback::default());
    let (state, ctx) = setup(
        Rc::new(NullNetworkClient),
        Rc::new(MemStore::default()),
        fb.clone(),
        Rc::new(NullClipboard),
        &[],
    );
    state.app_set("failed", json!(false));
    let reg = default_registry();
    let list = json!([{"call": {
        "module": "finance",
        "function": "compute",
        "args": ["1", "2"],
        "on_error": [{"set": {"$app.failed": "true"}}]
    }}]);
    let out = execute_list_shared(&reg, &list, &ctx);
    assert!(out.result.is_ok());
    assert_eq!(state.app_get("failed").unwrap().as_bool(), Some(true));
}
