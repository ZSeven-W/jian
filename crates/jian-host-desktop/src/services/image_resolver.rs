use jian_core::render::image_store::ImageResolver;

pub struct DesktopImageResolver {
    client: reqwest::Client,
}

impl Default for DesktopImageResolver {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ImageResolver for DesktopImageResolver {
    async fn resolve(&self, url: &str) -> Result<Vec<u8>, String> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        let bytes = response.bytes().await.map_err(|error| error.to_string())?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("image response exceeds 64 MiB".into());
        }
        Ok(bytes.to_vec())
    }
}
