use jian_core::action::services::{ServiceError, StorageBackend};
use serde_json::Value;
use std::rc::Rc;
use web_sys::Storage;

pub struct WebStorage {
    wake: Rc<dyn Fn()>,
}

impl WebStorage {
    pub fn new(wake: Rc<dyn Fn()>) -> Self {
        Self { wake }
    }

    fn storage(&self) -> Result<Storage, ServiceError> {
        web_sys::window()
            .ok_or_else(|| ServiceError("window unavailable".into()))?
            .local_storage()
            .map_err(js_error)?
            .ok_or_else(|| ServiceError("localStorage unavailable".into()))
    }
}

#[async_trait::async_trait(?Send)]
impl StorageBackend for WebStorage {
    async fn get(&self, key: &str) -> Result<Option<Value>, ServiceError> {
        let result = self
            .storage()?
            .get_item(key)
            .map_err(js_error)?
            .map(|text| serde_json::from_str(&text).unwrap_or(Value::String(text)));
        (self.wake)();
        Ok(result)
    }

    async fn set(&self, key: &str, value: Value) -> Result<(), ServiceError> {
        let text =
            serde_json::to_string(&value).map_err(|error| ServiceError(error.to_string()))?;
        let result = self.storage()?.set_item(key, &text).map_err(js_error);
        (self.wake)();
        result
    }

    async fn delete(&self, key: &str) -> Result<(), ServiceError> {
        let result = self.storage()?.remove_item(key).map_err(js_error);
        (self.wake)();
        result
    }

    async fn clear(&self) -> Result<(), ServiceError> {
        let result = self.storage()?.clear().map_err(js_error);
        (self.wake)();
        result
    }

    async fn keys(&self) -> Result<Vec<String>, ServiceError> {
        let storage = self.storage()?;
        let mut keys = Vec::new();
        for index in 0..storage.length().map_err(js_error)? {
            if let Some(key) = storage.key(index).map_err(js_error)? {
                keys.push(key);
            }
        }
        (self.wake)();
        Ok(keys)
    }
}

fn js_error(error: wasm_bindgen::JsValue) -> ServiceError {
    ServiceError(
        error
            .as_string()
            .unwrap_or_else(|| format!("localStorage exception: {error:?}")),
    )
}
