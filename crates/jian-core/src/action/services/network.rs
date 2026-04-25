use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub url: String,
    pub method: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Value>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Value,
}

#[async_trait(?Send)]
pub trait NetworkClient {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, String>;
    /// Open a WebSocket session. Hosts that don't ship a websocket
    /// implementation return an error and the `ws_*` actions surface a
    /// warning. The returned session is single-threaded — `Rc<...>` not
    /// `Arc<...>` — to match the rest of the runtime.
    async fn connect_websocket(&self, url: String) -> Result<Rc<dyn WebSocketSession>, String> {
        let _ = url;
        Err("WebSocket not implemented for this NetworkClient".into())
    }
}

#[async_trait(?Send)]
pub trait WebSocketSession {
    async fn send(&self, text: String) -> Result<(), String>;
    async fn close(&self) -> Result<(), String>;
    /// Drain any messages received since the last call. Hosts that
    /// don't ship a real implementation return an empty Vec; the
    /// runtime polls this each frame from `Runtime::tick` (or on
    /// each event loop iteration) to fire `on_message` handlers.
    ///
    /// Default impl returns empty so existing callers stay
    /// backward-compatible while production hosts override.
    async fn receive(&self) -> Vec<String> {
        Vec::new()
    }
}
