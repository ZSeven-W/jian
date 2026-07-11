//! No-op service implementations used when a real service isn't wired up.

use super::{
    clipboard::ClipboardService,
    feedback::{AsyncFeedback, FeedbackLevel, FeedbackSink},
    network::{HttpRequest, HttpResponse, NetworkClient},
    router::{RouteState, Router},
    storage::StorageBackend,
    ServiceError,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;

pub struct NullNetworkClient;

#[async_trait(?Send)]
impl NetworkClient for NullNetworkClient {
    async fn request(&self, _req: HttpRequest) -> Result<HttpResponse, String> {
        Err("NullNetworkClient: network not available".into())
    }
}

pub struct NullStorageBackend;

#[async_trait(?Send)]
impl StorageBackend for NullStorageBackend {
    async fn get(&self, _: &str) -> Result<Option<Value>, ServiceError> {
        Ok(None)
    }
    async fn set(&self, _: &str, _: Value) -> Result<(), ServiceError> {
        Ok(())
    }
    async fn delete(&self, _: &str) -> Result<(), ServiceError> {
        Ok(())
    }
    async fn clear(&self) -> Result<(), ServiceError> {
        Ok(())
    }
    async fn keys(&self) -> Result<Vec<String>, ServiceError> {
        Ok(Vec::new())
    }
}

pub struct NullRouter;

impl Router for NullRouter {
    fn current(&self) -> RouteState {
        RouteState {
            path: "/".into(),
            params: BTreeMap::new(),
            query: BTreeMap::new(),
            stack: vec!["/".into()],
        }
    }
    fn push(&self, _: &str) {}
    fn replace(&self, _: &str) {}
    fn pop(&self) {}
    fn reset(&self, _: &str) {}
}

pub struct NullFeedback;

impl FeedbackSink for NullFeedback {
    fn toast(&self, _: &str, _: FeedbackLevel, _: u32) {}
    fn alert(&self, _: &str, _: &str) {}
}

#[async_trait(?Send)]
impl AsyncFeedback for NullFeedback {
    async fn confirm(&self, _: &str, _: &str) -> bool {
        false
    }
}

pub struct NullClipboard;

#[async_trait(?Send)]
impl ClipboardService for NullClipboard {
    async fn read_text(&self) -> Result<String, ServiceError> {
        Err(ServiceError("clipboard unavailable".into()))
    }
    async fn write_text(&self, _: &str) -> Result<(), ServiceError> {
        Err(ServiceError("clipboard unavailable".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    #[test]
    fn null_network_errors() {
        let n = NullNetworkClient;
        let req = HttpRequest {
            url: "http://x".into(),
            method: "GET".into(),
            headers: BTreeMap::new(),
            body: None,
            timeout_ms: None,
        };
        let r = block_on(n.request(req));
        assert!(r.is_err());
    }

    #[test]
    fn null_storage_empty() {
        let s = NullStorageBackend;
        assert!(block_on(s.get("x")).unwrap().is_none());
        assert!(block_on(s.keys()).unwrap().is_empty());
    }

    #[test]
    fn null_router_stack_home() {
        assert_eq!(NullRouter.current().path, "/");
    }
}
