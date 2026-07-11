use super::ServiceError;
use async_trait::async_trait;
use serde_json::Value;

#[async_trait(?Send)]
pub trait StorageBackend {
    async fn get(&self, key: &str) -> Result<Option<Value>, ServiceError>;
    async fn set(&self, key: &str, value: Value) -> Result<(), ServiceError>;
    async fn delete(&self, key: &str) -> Result<(), ServiceError>;
    async fn clear(&self) -> Result<(), ServiceError>;
    async fn keys(&self) -> Result<Vec<String>, ServiceError>;
}
