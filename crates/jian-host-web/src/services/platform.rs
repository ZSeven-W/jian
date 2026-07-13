use jian_core::action::services::{PlatformService, ServiceError};

pub struct WebPlatform;

impl PlatformService for WebPlatform {
    fn open_url(&self, url: &str) -> Result<(), ServiceError> {
        let window = web_sys::window().ok_or_else(|| ServiceError("window unavailable".into()))?;
        let opened = window
            .open_with_url_and_target(url, "_blank")
            .map_err(|error| ServiceError(format!("window.open failed: {error:?}")))?;
        opened
            .map(|_| ())
            .ok_or_else(|| ServiceError("popup blocked".into()))
    }
}
