//! WebSocket actions: `ws_connect`, `ws_send`, `ws_close`.
//!
//! Sessions are addressed by an author-chosen string handle stored in
//! `ActionContext::ws_sessions`:
//!
//! ```jsonc
//! { "ws_connect": { "handle": "chat", "url": "wss://example.com/chat" } }
//! { "ws_send":    { "handle": "chat", "text": "$state.draft" } }
//! { "ws_close":   { "handle": "chat" } }
//! ```
//!
//! Capability gating: every verb checks `Capability::Network` before
//! touching the wire — same gate that protects `fetch`. The
//! `WebSocketSession` trait is single-threaded (`Rc<dyn ...>`) to match
//! the rest of the runtime.

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

// --- ws_connect -------------------------------------------------------

struct WsConnect {
    handle: String,
    url_expr: Expression,
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
        let (url_v, ws) = self.url_expr.eval_with_locals(
            &ctx.state,
            ctx.page_id.as_deref(),
            ctx.node_id.as_deref(),
            &locals,
        );
        for w in ws {
            ctx.warn(w);
        }
        let url = url_v.as_str().unwrap_or("").to_owned();
        if url.is_empty() {
            return Err(ActionError::Custom(
                "ws_connect: url evaluated to empty".into(),
            ));
        }
        match ctx.network.connect_websocket(url.clone()).await {
            Ok(session) => {
                ctx.ws_sessions
                    .borrow_mut()
                    .insert(self.handle.clone(), session);
                Ok(())
            }
            Err(e) => Err(ActionError::Custom(format!(
                "ws_connect({}): {}",
                self.handle, e
            ))),
        }
    }
}

pub fn factory_ws_connect(body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "ws_connect",
        field: "body",
        message: "must be object with `handle` and `url`".into(),
    })?;
    let handle = read_str_field(obj, "ws_connect", "handle")?;
    let url_src = read_str_field(obj, "ws_connect", "url")?;
    Ok(Box::new(WsConnect {
        handle,
        url_expr: Expression::compile(&url_src)?,
    }))
}

// --- ws_send ----------------------------------------------------------

struct WsSend {
    handle: String,
    text_expr: Expression,
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
        let (text_v, ws) = self.text_expr.eval_with_locals(
            &ctx.state,
            ctx.page_id.as_deref(),
            ctx.node_id.as_deref(),
            &locals,
        );
        for w in ws {
            ctx.warn(w);
        }
        let text = text_v.as_str().unwrap_or("").to_owned();
        let session = ctx.ws_sessions.borrow().get(&self.handle).cloned();
        let Some(session) = session else {
            return Err(ActionError::Custom(format!(
                "ws_send: no session named {:?}",
                self.handle
            )));
        };
        match session.send(text).await {
            Ok(()) => Ok(()),
            Err(e) => Err(ActionError::Custom(format!("ws_send: {}", e))),
        }
    }
}

pub fn factory_ws_send(body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "ws_send",
        field: "body",
        message: "must be object with `handle` and `text`".into(),
    })?;
    let handle = read_str_field(obj, "ws_send", "handle")?;
    let text_src = read_str_field(obj, "ws_send", "text")?;
    Ok(Box::new(WsSend {
        handle,
        text_expr: Expression::compile(&text_src)?,
    }))
}

// --- ws_close ---------------------------------------------------------

struct WsClose {
    handle: String,
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
        let session = ctx.ws_sessions.borrow_mut().remove(&self.handle);
        let Some(session) = session else {
            // Closing a missing handle is a no-op + warning, not an
            // error — apps frequently call close defensively.
            return Ok(());
        };
        match session.close().await {
            Ok(()) => Ok(()),
            Err(e) => Err(ActionError::Custom(format!("ws_close: {}", e))),
        }
    }
}

pub fn factory_ws_close(body: &Value) -> Result<BoxedAction, ActionError> {
    let obj = body.as_object().ok_or(ActionError::FieldType {
        name: "ws_close",
        field: "body",
        message: "must be object with `handle`".into(),
    })?;
    let handle = read_str_field(obj, "ws_close", "handle")?;
    Ok(Box::new(WsClose { handle }))
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
            "handle": "chat",
            "url": "\"wss://example.com/chat\""
        }))
        .unwrap();
        block_on(connect.execute(&ctx)).unwrap();
        assert_eq!(
            last_url.borrow().as_deref(),
            Some("wss://example.com/chat")
        );
        assert!(ctx.ws_sessions.borrow().contains_key("chat"));

        let send = factory_ws_send(&serde_json::json!({
            "handle": "chat",
            "text": "\"hi\""
        }))
        .unwrap();
        block_on(send.execute(&ctx)).unwrap();
        assert_eq!(*sent.borrow(), vec!["hi"]);

        let close = factory_ws_close(&serde_json::json!({ "handle": "chat" })).unwrap();
        block_on(close.execute(&ctx)).unwrap();
        assert!(*closed.borrow());
        assert!(!ctx.ws_sessions.borrow().contains_key("chat"));
    }

    #[test]
    fn ws_send_unknown_handle_errors() {
        use futures::executor::block_on;
        let net = Rc::new(MockNet {
            sent: Rc::new(RefCell::new(vec![])),
            closed: Rc::new(RefCell::new(false)),
            last_url: Rc::new(RefCell::new(None)),
        });
        let ctx = ctx_with_mock_net(net);
        let send = factory_ws_send(&serde_json::json!({
            "handle": "ghost",
            "text": "\"hi\""
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
        let close = factory_ws_close(&serde_json::json!({ "handle": "ghost" })).unwrap();
        block_on(close.execute(&ctx)).unwrap();
    }
}
