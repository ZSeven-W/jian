//! In-memory `StorageBackend` — good enough for the desktop host MVP
//! and all the tests. The real SQLite-backed version arrives behind a
//! future `sqlite` feature flag (Plan 8 T6 follow-up).

use async_trait::async_trait;
use jian_core::action::services::{ServiceError, StorageBackend};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::BTreeMap;

pub struct InMemoryStorage {
    inner: RefCell<BTreeMap<String, Value>>,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(BTreeMap::new()),
        }
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait(?Send)]
impl StorageBackend for InMemoryStorage {
    async fn get(&self, key: &str) -> Result<Option<Value>, ServiceError> {
        Ok(self.inner.borrow().get(key).cloned())
    }
    async fn set(&self, key: &str, value: Value) -> Result<(), ServiceError> {
        self.inner.borrow_mut().insert(key.to_owned(), value);
        Ok(())
    }
    async fn delete(&self, key: &str) -> Result<(), ServiceError> {
        self.inner.borrow_mut().remove(key);
        Ok(())
    }
    async fn clear(&self) -> Result<(), ServiceError> {
        self.inner.borrow_mut().clear();
        Ok(())
    }
    async fn keys(&self) -> Result<Vec<String>, ServiceError> {
        Ok(self.inner.borrow().keys().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn set_then_get_roundtrips() {
        let s = InMemoryStorage::new();
        let fut = async {
            s.set("k", json!(42)).await.unwrap();
            s.get("k").await
        };
        let v = futures::executor::block_on(fut).unwrap().unwrap();
        assert_eq!(v, json!(42));
    }

    #[test]
    fn clear_empties_all() {
        let s = InMemoryStorage::new();
        let fut = async {
            s.set("a", json!(1)).await.unwrap();
            s.set("b", json!(2)).await.unwrap();
            s.clear().await.unwrap();
            s.keys().await
        };
        let ks = futures::executor::block_on(fut).unwrap();
        assert!(ks.is_empty());
    }
}
