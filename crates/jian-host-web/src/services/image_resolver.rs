use super::network::fetch_bytes;
use super::AbortRegistry;
use jian_core::render::image_store::{ImageAdmission, ImageResolver};
use std::rc::Rc;
use url::Url;

#[derive(Debug, Clone)]
pub(crate) struct AssetPolicy {
    base: Url,
    base_segments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedAsset {
    pub url: String,
    pub trusted_bundle: bool,
}

impl AssetPolicy {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        let base = Url::parse(value).map_err(|error| format!("invalid assetBase: {error}"))?;
        if !matches!(base.scheme(), "http" | "https")
            || base.cannot_be_a_base()
            || !base.path().ends_with('/')
            || base.query().is_some()
            || base.fragment().is_some()
            || !base.username().is_empty()
            || base.password().is_some()
        {
            return Err(
                "assetBase must be a hierarchical HTTP(S) directory URL without credentials, query, or fragment"
                    .into(),
            );
        }
        let base_segments = segments(&base);
        Ok(Self {
            base,
            base_segments,
        })
    }

    pub(crate) fn resolve(&self, source: &str) -> Result<ResolvedAsset, String> {
        let scheme_relative = source.starts_with("//");
        let absolute = Url::parse(source).is_ok();
        let relative = !scheme_relative && !absolute;
        let url = if relative || scheme_relative {
            self.base
                .join(source)
                .map_err(|error| format!("invalid image URL: {error}"))?
        } else {
            Url::parse(source).map_err(|error| format!("invalid image URL: {error}"))?
        };
        let same_origin = url.origin() == self.base.origin();
        let candidate = segments(&url);
        let contained = candidate.starts_with(&self.base_segments);
        Ok(ResolvedAsset {
            url: url.to_string(),
            trusted_bundle: relative && same_origin && contained,
        })
    }
}

pub(crate) struct WebImageResolver {
    policy: Option<AssetPolicy>,
    aborts: AbortRegistry,
    wake: Rc<dyn Fn()>,
}

impl WebImageResolver {
    pub(crate) fn new(
        policy: Option<AssetPolicy>,
        aborts: AbortRegistry,
        wake: Rc<dyn Fn()>,
    ) -> Self {
        Self {
            policy,
            aborts,
            wake,
        }
    }

    fn resolve_url(&self, source: &str) -> Result<ResolvedAsset, String> {
        match &self.policy {
            Some(policy) => policy.resolve(source),
            None => {
                let url = if source.starts_with("//") {
                    Url::parse(&format!("{}{source}", page_protocol()))
                } else {
                    Url::parse(source)
                }
                .map_err(|_| "relative image requires opts.assetBase".to_owned())?;
                Ok(ResolvedAsset {
                    url: url.to_string(),
                    trusted_bundle: false,
                })
            }
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ImageResolver for WebImageResolver {
    fn admission(&self, source: &str) -> Result<Option<ImageAdmission>, String> {
        let resolved = self.resolve_url(source)?;
        let parsed =
            Url::parse(&resolved.url).map_err(|error| format!("invalid image URL: {error}"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("web image URL must use HTTP(S)".into());
        }
        Ok(Some(ImageAdmission {
            key: resolved.url,
            request_source: source.to_owned(),
            requires_network: !resolved.trusted_bundle,
        }))
    }

    async fn resolve(&self, source: &str) -> Result<Vec<u8>, String> {
        let resolved = self.resolve_url(source)?;
        fetch_bytes(&resolved.url, &self.aborts, &self.wake).await
    }
}

fn segments(url: &Url) -> Vec<String> {
    url.path_segments()
        .into_iter()
        .flatten()
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(target_arch = "wasm32")]
fn page_protocol() -> String {
    web_sys::window()
        .and_then(|window| window.location().protocol().ok())
        .filter(|protocol| matches!(protocol.as_str(), "http:" | "https:"))
        .unwrap_or_else(|| "https:".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn page_protocol() -> String {
    "https:".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_base_trust_requires_relative_origin_and_segment_containment() {
        let policy = AssetPolicy::parse("https://example.test/assets/").unwrap();
        assert!(policy.resolve("icons/a.png").unwrap().trusted_bundle);
        assert!(
            !policy
                .resolve("../assets-evil/a.png")
                .unwrap()
                .trusted_bundle
        );
        assert!(
            !policy
                .resolve("https://other.test/assets/a.png")
                .unwrap()
                .trusted_bundle
        );
        assert!(
            !policy
                .resolve("//example.test/assets/a.png")
                .unwrap()
                .trusted_bundle
        );
        assert!(
            !policy
                .resolve("https://example.test/assets/a.png")
                .unwrap()
                .trusted_bundle
        );
    }

    #[test]
    fn asset_base_must_be_http_directory() {
        assert!(AssetPolicy::parse("https://example.test/assets/").is_ok());
        assert!(AssetPolicy::parse("https://example.test/assets").is_err());
        assert!(AssetPolicy::parse("data:text/plain,no").is_err());
    }

    #[test]
    fn resolver_admission_preserves_relative_provenance_and_network_gate() {
        let resolver = WebImageResolver::new(
            Some(AssetPolicy::parse("https://example.test/assets/").unwrap()),
            AbortRegistry::default(),
            Rc::new(|| {}),
        );
        let trusted = resolver.admission("icons/a.png").unwrap().unwrap();
        assert_eq!(trusted.key, "https://example.test/assets/icons/a.png");
        assert_eq!(trusted.request_source, "icons/a.png");
        assert!(!trusted.requires_network);

        let equivalent = resolver.admission("./icons/a.png").unwrap().unwrap();
        assert_eq!(equivalent.key, trusted.key);
        assert!(!equivalent.requires_network);

        let absolute = resolver
            .admission("https://example.test/assets/icons/a.png")
            .unwrap()
            .unwrap();
        assert_eq!(absolute.key, "https://example.test/assets/icons/a.png");
        assert!(absolute.requires_network);

        let scheme_relative = resolver
            .admission("//example.test/assets/icons/a.png")
            .unwrap()
            .unwrap();
        assert!(scheme_relative.requires_network);

        let no_base = WebImageResolver::new(None, AbortRegistry::default(), Rc::new(|| {}));
        let scheme_relative = no_base
            .admission("//cdn.example.test/icons/a.png")
            .unwrap()
            .unwrap();
        assert!(scheme_relative.requires_network);
        assert!(scheme_relative
            .key
            .ends_with("//cdn.example.test/icons/a.png"));
    }
}
