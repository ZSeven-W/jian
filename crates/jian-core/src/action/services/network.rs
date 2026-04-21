use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;

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
}

#[async_trait(?Send)]
pub trait WebSocketSession {
    async fn send(&self, text: String) -> Result<(), String>;
    async fn close(&self) -> Result<(), String>;
}
