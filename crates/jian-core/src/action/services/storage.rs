use async_trait::async_trait;
use serde_json::Value;

#[async_trait(?Send)]
pub trait StorageBackend {
    async fn get(&self, key: &str) -> Option<Value>;
    async fn set(&self, key: &str, value: Value);
    async fn delete(&self, key: &str);
    async fn clear(&self);
    async fn keys(&self) -> Vec<String>;
}
