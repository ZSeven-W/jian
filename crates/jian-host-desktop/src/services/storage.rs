//! In-memory `StorageBackend` — good enough for the desktop host MVP
//! and all the tests. The real SQLite-backed version arrives behind a
//! future `sqlite` feature flag (Plan 8 T6 follow-up).

use async_trait::async_trait;
use jian_core::action::services::StorageBackend;
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
    async fn get(&self, key: &str) -> Option<Value> {
        self.inner.borrow().get(key).cloned()
    }
    async fn set(&self, key: &str, value: Value) {
        self.inner.borrow_mut().insert(key.to_owned(), value);
    }
    async fn delete(&self, key: &str) {
        self.inner.borrow_mut().remove(key);
    }
    async fn clear(&self) {
        self.inner.borrow_mut().clear();
    }
    async fn keys(&self) -> Vec<String> {
        self.inner.borrow().keys().cloned().collect()
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
            s.set("k", json!(42)).await;
            s.get("k").await
        };
        let v = futures::executor::block_on(fut).unwrap();
        assert_eq!(v, json!(42));
    }

    #[test]
    fn clear_empties_all() {
        let s = InMemoryStorage::new();
        let fut = async {
            s.set("a", json!(1)).await;
            s.set("b", json!(2)).await;
            s.clear().await;
            s.keys().await
        };
        let ks = futures::executor::block_on(fut);
        assert!(ks.is_empty());
    }
}
