//! One monotonic clock epoch shared by every adapter in a mounted host.

use wasm_bindgen::JsValue;
#[cfg(target_arch = "wasm32")]
use web_sys::Performance;

#[derive(Clone)]
pub(crate) struct HostClock {
    #[cfg(target_arch = "wasm32")]
    performance: Performance,
    #[cfg(target_arch = "wasm32")]
    epoch: f64,
}

impl HostClock {
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new() -> Result<Self, JsValue> {
        let performance = web_sys::window()
            .and_then(|window| window.performance())
            .ok_or_else(|| JsValue::from_str("performance clock unavailable"))?;
        let epoch = performance.now();
        Ok(Self { performance, epoch })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn new() -> Result<Self, JsValue> {
        Ok(Self {})
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn now_ms(&self) -> u64 {
        (self.performance.now() - self.epoch).max(0.0).round() as u64
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn now_ms(&self) -> u64 {
        0
    }
}
