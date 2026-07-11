use super::ServiceError;

pub trait PlatformService {
    fn open_url(&self, url: &str) -> Result<(), ServiceError>;
}

pub struct NullPlatform;

impl PlatformService for NullPlatform {
    fn open_url(&self, _url: &str) -> Result<(), ServiceError> {
        Err(ServiceError("open_url is unavailable".into()))
    }
}
