use super::ServiceError;
use async_trait::async_trait;

#[async_trait(?Send)]
pub trait ClipboardService {
    async fn read_text(&self) -> Result<String, ServiceError>;
    async fn write_text(&self, text: &str) -> Result<(), ServiceError>;
}
