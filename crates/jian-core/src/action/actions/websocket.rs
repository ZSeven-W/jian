//! WebSocket actions: `ws_connect`, `ws_send`, `ws_close`.
//!
//! Sessions are addressed by author-chosen string `id` stored in
//! `ActionContext::ws_sessions` — wire shape matches spec
//! 02-logic-and-reactivity.md §3.2:
//!
//! ```jsonc
//! { "ws_connect": { "id": "chat", "url": "wss://example.com/chat",
//!                    "on_message": [...] } }
//! { "ws_send":    { "id": "chat", "data": "$state.draft" } }
//! { "ws_close":   { "id": "chat" } }
//! ```
//!
//! Capability gating: every verb checks `Capability::Network` before
//! touching the wire — same gate that protects `fetch`. The
//! `WebSocketSession` trait is single-threaded (`Rc<dyn ...>`) to match
//! the rest of the runtime.
//!
//! `ws_connect` will close any existing session bound to the same `id`
//! before installing the new one — reconnect-by-id is a common pattern
//! and silently leaking the old socket would be a bug.
//!
//! `on_message` is accepted by the parser (so authored `.op` files
//! validate) but not yet dispatched — the underlying `WebSocketSession`
//! trait is send/close-only. A subsequent change adds a receiver
//! channel + ActionList wiring; for now we record the bound chain and
//! emit a one-shot warning so authors know the handler won't fire yet.

use crate::action::action_trait::{ActionImpl, BoxedAction};
use crate::action::capability::Capability;
use crate::action::context::ActionContext;
use crate::action::error::{ActionError, ActionResult};
use crate::expression::Expression;
use async_trait::async_trait;
use serde_json::Value;

fn read_str_field(
    obj: &serde_json::Map<String, Value>,
    name: &'static str,
    field: &'static str,
) -> Result<String, ActionError> {
    obj.get(field)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or(ActionError::MissingField { name, field })
}

/// Resolve an authored `id` field, accepting the legacy `handle` alias
/// from earlier drafts so existing `.op` files don't break.
fn read_session_id(
    obj: &serde_json::Map<String, Value>,
    name: &'static str,
) -> Result<String, ActionError> {
    if let Some(s) = obj.get("id").and_then(|v| v.as_str()) {
        return Ok(s.to_owned());
    }
    if let Some(s) = obj.get("handle").and_then(|v| v.as_str()) {
        return Ok(s.to_owned());
    }
    Err(ActionError::MissingField { name, field: "id" })
}

// --- ws_connect -------------------------------------------------------

struct WsConnect {
    id: String,
    url_expr: Expression,
    /// Stored verbatim so `Runtime::pump_websockets` can re-parse it
    /// each time a message arrives. Storing JSON (rather than a
    /// compiled chain) sidesteps ActionChain's lack of `Clone`.
    on_message: Option<Value>,
}

#[async_trait(?Send)]
impl ActionImpl for WsConnect {
    fn name(&self) -> &'static str {
        "ws_connect"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        if !ctx.capabilities.check(Capability::Network, "ws_connect") {
            return Err(ActionError::CapabilityDenied {
                action: "ws_connect",
                needed: Capability::Network,
            });
        }
        let locals = ctx.locals_snapshot();
        let (url_v, warns) = self.url_expr.eval_with_locals(
            &ctx.state,
            ctx.page_id.as_deref(),
            ctx.node_id.as_deref(),
            &locals,
        );
        for w in warns {
            ctx.warn(w);
        }
        let url = url_v.as_str().unwrap_or("").to_owned();
        if url.is_empty() {
            return Err(ActionError::Custom(
                "ws_connect: url evaluated to empty".into(),
            ));
        }
        // Reconnect-by-id: close any existing session under the same
        // handle before installing the new one. Errors on close are
        // demoted to a warning — they shouldn't block the new connect.
        let prior = ctx.ws_sessions.borrow_mut().remove(&self.id);
        if let Some(old) = prior {
            if let Err(e) = old.session.close().await {
                ctx.warn(crate::expression::Diagnostic::runtime_warning(format!(
                    "ws_connect({}): close of prior session failed: {}",
                    self.id, e
                )));
            }
        }
        match ctx.network.connect_websocket(url.clone()).await {
            Ok(session) => {
                ctx.ws_sessions.borrow_mut().insert(
                    self.id.clone(),
                    crate::action::context::WsHandle {
                        session,
                        on_message: self.on_message.clone(),
                    },
                );
                Ok(())
            }
            Err(e) => Err(ActionError::Custom(format!(
                "ws_connect({}): {}",
                self.id, e
            ))),
        }
    }
}

pub fn factory_ws_connect(body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "ws_connect",
        field: "body",
        message: "must be object with `id` and `url` (legacy `handle` accepted)".into(),
    })?;
    let id = read_session_id(obj, "ws_connect")?;
    let url_src = read_str_field(obj, "ws_connect", "url")?;
    let on_message = obj.get("on_message").cloned();
    Ok(Box::new(WsConnect {
        id,
        url_expr: Expression::compile(&url_src)?,
        on_message,
    }))
}

// --- ws_send ----------------------------------------------------------

struct WsSend {
    id: String,
    data_expr: Expression,
}

#[async_trait(?Send)]
impl ActionImpl for WsSend {
    fn name(&self) -> &'static str {
        "ws_send"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        if !ctx.capabilities.check(Capability::Network, "ws_send") {
            return Err(ActionError::CapabilityDenied {
                action: "ws_send",
                needed: Capability::Network,
            });
        }
        let locals = ctx.locals_snapshot();
        let (data_v, warns) = self.data_expr.eval_with_locals(
            &ctx.state,
            ctx.page_id.as_deref(),
            ctx.node_id.as_deref(),
            &locals,
        );
        for w in warns {
            ctx.warn(w);
        }
        let text = data_v.as_str().unwrap_or("").to_owned();
        let handle = ctx.ws_sessions.borrow().get(&self.id).cloned();
        let Some(handle) = handle else {
            return Err(ActionError::Custom(format!(
                "ws_send: no session named {:?}",
                self.id
            )));
        };
        match handle.session.send(text).await {
            Ok(()) => Ok(()),
            Err(e) => Err(ActionError::Custom(format!("ws_send: {}", e))),
        }
    }
}

pub fn factory_ws_send(body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "ws_send",
        field: "body",
        message: "must be object with `id` and `data` (legacy `handle`/`text` accepted)".into(),
    })?;
    let id = read_session_id(obj, "ws_send")?;
    // Spec field is `data`; legacy `text` accepted to keep older
    // authored payloads working.
    let data_src = obj
        .get("data")
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("text").and_then(|v| v.as_str()))
        .ok_or(ActionError::MissingField {
            name: "ws_send",
            field: "data",
        })?
        .to_owned();
    Ok(Box::new(WsSend {
        id,
        data_expr: Expression::compile(&data_src)?,
    }))
}

// --- ws_close ---------------------------------------------------------

struct WsClose {
    id: String,
}

#[async_trait(?Send)]
impl ActionImpl for WsClose {
    fn name(&self) -> &'static str {
        "ws_close"
    }
    async fn execute(&self, ctx: &ActionContext) -> ActionResult {
        if !ctx.capabilities.check(Capability::Network, "ws_close") {
            return Err(ActionError::CapabilityDenied {
                action: "ws_close",
                needed: Capability::Network,
            });
        }
        let handle = ctx.ws_sessions.borrow_mut().remove(&self.id);
        let Some(handle) = handle else {
            // Closing a missing id is a no-op — apps frequently call
            // close defensively.
            return Ok(());
        };
        match handle.session.close().await {
            Ok(()) => Ok(()),
            Err(e) => Err(ActionError::Custom(format!("ws_close: {}", e))),
        }
    }
}

pub fn factory_ws_close(body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "ws_close",
        field: "body",
        message: "must be object with `id` (legacy `handle` accepted)".into(),
    })?;
    let id = read_session_id(obj, "ws_close")?;
    Ok(Box::new(WsClose { id }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::services::{NetworkClient, WebSocketSession};
    use async_trait::async_trait;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct MockSession {
        sent: Rc<RefCell<Vec<String>>>,
        closed: Rc<RefCell<bool>>,
    }

    #[async_trait(?Send)]
    impl WebSocketSession for MockSession {
        async fn send(&self, text: String) -> Result<(), String> {
            self.sent.borrow_mut().push(text);
            Ok(())
        }
        async fn close(&self) -> Result<(), String> {
            *self.closed.borrow_mut() = true;
            Ok(())
        }
    }

    struct MockNet {
        sent: Rc<RefCell<Vec<String>>>,
        closed: Rc<RefCell<bool>>,
        last_url: Rc<RefCell<Option<String>>>,
    }

    #[async_trait(?Send)]
    impl NetworkClient for MockNet {
        async fn request(
            &self,
            _req: crate::action::services::HttpRequest,
        ) -> Result<crate::action::services::HttpResponse, String> {
            Err("not used in ws tests".into())
        }
        async fn connect_websocket(
            &self,
            url: String,
        ) -> Result<Rc<dyn WebSocketSession>, String> {
            *self.last_url.borrow_mut() = Some(url);
            Ok(Rc::new(MockSession {
                sent: self.sent.clone(),
                closed: self.closed.clone(),
            }))
        }
    }

    fn ctx_with_mock_net(net: Rc<MockNet>) -> ActionContext {
        use crate::action::capability::DummyCapabilityGate;
        use crate::action::services::{
            NullClipboard, NullFeedback, NullRouter, NullStorageBackend,
        };
        use crate::expression::ExpressionCache;
        use crate::signal::scheduler::Scheduler;
        use crate::state::StateGraph;
        use std::collections::{BTreeMap, HashMap};
        let sched = Rc::new(Scheduler::new());
        ActionContext {
            state: Rc::new(StateGraph::new(sched.clone())),
            scheduler: sched,
            event: None,
            locals: RefCell::new(BTreeMap::new()),
            page_id: None,
            node_id: None,
            network: net,
            ws_sessions: Rc::new(RefCell::new(HashMap::new())),
            storage: Rc::new(NullStorageBackend),
            router: Rc::new(NullRouter),
            feedback: Rc::new(NullFeedback),
            async_fb: Rc::new(NullFeedback),
            clipboard: Rc::new(NullClipboard),
            capabilities: Rc::new(DummyCapabilityGate),
            logic: Rc::new(crate::logic::NullLogicProvider),
            expr_cache: Rc::new(ExpressionCache::new()),
            cancel: crate::action::cancel::CancellationToken::new(),
            warnings: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn ws_connect_send_close_roundtrip() {
        use futures::executor::block_on;
        let sent = Rc::new(RefCell::new(Vec::<String>::new()));
        let closed = Rc::new(RefCell::new(false));
        let last_url = Rc::new(RefCell::new(None::<String>));
        let net = Rc::new(MockNet {
            sent: sent.clone(),
            closed: closed.clone(),
            last_url: last_url.clone(),
        });
        let ctx = ctx_with_mock_net(net);

        let connect = factory_ws_connect(&serde_json::json!({
            "id": "chat",
            "url": "\"wss://example.com/chat\""
        }))
        .unwrap();
        block_on(connect.execute(&ctx)).unwrap();
        assert_eq!(last_url.borrow().as_deref(), Some("wss://example.com/chat"));
        assert!(ctx.ws_sessions.borrow().contains_key("chat"));

        let send = factory_ws_send(&serde_json::json!({
            "id": "chat",
            "data": "\"hi\""
        }))
        .unwrap();
        block_on(send.execute(&ctx)).unwrap();
        assert_eq!(*sent.borrow(), vec!["hi"]);

        let close = factory_ws_close(&serde_json::json!({ "id": "chat" })).unwrap();
        block_on(close.execute(&ctx)).unwrap();
        assert!(*closed.borrow());
        assert!(!ctx.ws_sessions.borrow().contains_key("chat"));
    }

    #[test]
    fn ws_connect_legacy_handle_alias_still_works() {
        use futures::executor::block_on;
        let net = Rc::new(MockNet {
            sent: Rc::new(RefCell::new(vec![])),
            closed: Rc::new(RefCell::new(false)),
            last_url: Rc::new(RefCell::new(None)),
        });
        let ctx = ctx_with_mock_net(net);
        let connect = factory_ws_connect(&serde_json::json!({
            "handle": "legacy",
            "url": "\"wss://x\""
        }))
        .unwrap();
        block_on(connect.execute(&ctx)).unwrap();
        assert!(ctx.ws_sessions.borrow().contains_key("legacy"));
    }

    #[test]
    fn ws_connect_reconnect_closes_prior_session() {
        use futures::executor::block_on;
        let closed = Rc::new(RefCell::new(false));
        let net = Rc::new(MockNet {
            sent: Rc::new(RefCell::new(vec![])),
            closed: closed.clone(),
            last_url: Rc::new(RefCell::new(None)),
        });
        let ctx = ctx_with_mock_net(net);
        let connect = factory_ws_connect(&serde_json::json!({
            "id": "chat",
            "url": "\"wss://x\""
        }))
        .unwrap();
        block_on(connect.execute(&ctx)).unwrap();
        // Reconnect with the same id.
        let connect2 = factory_ws_connect(&serde_json::json!({
            "id": "chat",
            "url": "\"wss://x2\""
        }))
        .unwrap();
        block_on(connect2.execute(&ctx)).unwrap();
        // Prior session was closed.
        assert!(*closed.borrow(), "expected prior session to be closed");
    }

    #[test]
    fn ws_send_unknown_id_errors() {
        use futures::executor::block_on;
        let net = Rc::new(MockNet {
            sent: Rc::new(RefCell::new(vec![])),
            closed: Rc::new(RefCell::new(false)),
            last_url: Rc::new(RefCell::new(None)),
        });
        let ctx = ctx_with_mock_net(net);
        let send = factory_ws_send(&serde_json::json!({
            "id": "ghost",
            "data": "\"hi\""
        }))
        .unwrap();
        let r = block_on(send.execute(&ctx));
        assert!(r.is_err());
    }

    #[test]
    fn ws_close_missing_is_noop() {
        use futures::executor::block_on;
        let net = Rc::new(MockNet {
            sent: Rc::new(RefCell::new(vec![])),
            closed: Rc::new(RefCell::new(false)),
            last_url: Rc::new(RefCell::new(None)),
        });
        let ctx = ctx_with_mock_net(net);
        let close = factory_ws_close(&serde_json::json!({ "id": "ghost" })).unwrap();
        block_on(close.execute(&ctx)).unwrap();
    }

    #[test]
    fn ws_connect_stores_on_message_handler() {
        // Spec on_message used to surface a warning ("not yet
        // dispatched"); now `Runtime::pump_websockets` actually fires
        // the handler. ws_connect just needs to retain the JSON for
        // pumping to find later.
        use futures::executor::block_on;
        let net = Rc::new(MockNet {
            sent: Rc::new(RefCell::new(vec![])),
            closed: Rc::new(RefCell::new(false)),
            last_url: Rc::new(RefCell::new(None)),
        });
        let ctx = ctx_with_mock_net(net);
        let connect = factory_ws_connect(&serde_json::json!({
            "id": "chat",
            "url": "\"wss://x\"",
            "on_message": [{ "set": { "$state.last": "$event.data" } }]
        }))
        .unwrap();
        block_on(connect.execute(&ctx)).unwrap();
        let stored = ctx.ws_sessions.borrow().get("chat").cloned().unwrap();
        assert!(
            stored.on_message.is_some(),
            "on_message handler should be retained for runtime pump"
        );
    }
}
