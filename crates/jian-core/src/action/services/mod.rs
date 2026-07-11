pub mod clipboard;
pub mod feedback;
pub mod network;
pub mod null_impls;
pub mod platform;
pub mod router;
pub mod storage;

pub use clipboard::ClipboardService;
pub use feedback::{AsyncFeedback, FeedbackLevel, FeedbackSink};
pub use network::{HttpRequest, HttpResponse, NetworkClient, WebSocketSession};
pub use null_impls::{
    NullClipboard, NullFeedback, NullNetworkClient, NullRouter, NullStorageBackend,
};
pub use platform::{NullPlatform, PlatformService};
pub use router::{RouteState, Router};
pub use storage::StorageBackend;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceError(pub String);

impl std::fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ServiceError {}
