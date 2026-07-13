use jian_core::action::services::{ClipboardService, ServiceError};
use std::rc::Rc;

use super::await_with_wake;

pub struct WebClipboard {
    wake: Rc<dyn Fn()>,
}

impl WebClipboard {
    pub fn new(wake: Rc<dyn Fn()>) -> Self {
        Self { wake }
    }
}

#[async_trait::async_trait(?Send)]
impl ClipboardService for WebClipboard {
    async fn read_text(&self) -> Result<String, ServiceError> {
        let clipboard = web_sys::window()
            .ok_or_else(|| ServiceError("window unavailable".into()))?
            .navigator()
            .clipboard();
        let result = await_with_wake(clipboard.read_text(), &self.wake)
            .await
            .map_err(js_error)
            .and_then(|value| {
                value
                    .as_string()
                    .ok_or_else(|| ServiceError("clipboard returned non-text data".into()))
            });
        (self.wake)();
        result
    }

    async fn write_text(&self, text: &str) -> Result<(), ServiceError> {
        let clipboard = web_sys::window()
            .ok_or_else(|| ServiceError("window unavailable".into()))?
            .navigator()
            .clipboard();
        let result = await_with_wake(clipboard.write_text(text), &self.wake)
            .await
            .map(|_| ())
            .map_err(js_error);
        (self.wake)();
        result
    }
}

fn js_error(error: wasm_bindgen::JsValue) -> ServiceError {
    ServiceError(
        error
            .as_string()
            .unwrap_or_else(|| format!("clipboard operation rejected: {error:?}")),
    )
}
