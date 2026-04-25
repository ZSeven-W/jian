//! Image decode + cache.
//!
//! `DrawOp::Image` carries an `ImageSource`. The skia backend caches
//! decoded `sk_Image` instances keyed by [`ImageSource::cache_key`] —
//! re-drawing the same image (a list cell repainting on scroll, the
//! same `image` URL reused across frames) skips the decode.
//!
//! Decoding paths:
//! - **`DataUrl`** — strip `data:image/...;base64,` prefix, base64-decode
//!   the payload, hand the bytes to `skia_safe::Image::from_encoded`.
//! - **`Bytes`** — feed straight into `from_encoded`.
//! - **`Url`** — host-resolution path; without an injected resolver
//!   we return `None` and the backend draws a placeholder.
//!
//! The cache is unbounded for now — Plan 12 will add LRU eviction with
//! a 128 MB cap once a real-world corpus exists.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use jian_core::render::ImageSource;
use skia_safe::Image as SkImage;
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct ImageCache {
    decoded: HashMap<String, Option<SkImage>>,
}

impl ImageCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up the source's cached image, decoding on first miss.
    /// Returns `None` if decoding failed or the source is a remote URL
    /// the cache cannot resolve — callers fall back to a placeholder.
    pub fn get_or_decode(&mut self, source: &ImageSource) -> Option<&SkImage> {
        let key = source.cache_key();
        if !self.decoded.contains_key(&key) {
            let decoded = decode(source);
            self.decoded.insert(key.clone(), decoded);
        }
        self.decoded.get(&key).and_then(|opt| opt.as_ref())
    }
}

fn decode(source: &ImageSource) -> Option<SkImage> {
    match source {
        ImageSource::DataUrl(s) => {
            let bytes = decode_data_url(s)?;
            decode_bytes(&bytes)
        }
        ImageSource::Bytes(b) => decode_bytes(b.as_slice()),
        ImageSource::Url(_) => None,
    }
}

/// Parse `data:image/...;base64,<payload>`. We accept any media type so
/// PNG / JPG / GIF / WebP all flow through the same path. Non-base64
/// data URLs (rare) are not supported — they're a mis-fit for embedded
/// binary assets.
fn decode_data_url(s: &str) -> Option<Vec<u8>> {
    let rest = s.strip_prefix("data:")?;
    let (header, payload) = rest.split_once(',')?;
    if !header.split(';').any(|p| p == "base64") {
        return None;
    }
    STANDARD.decode(payload.trim()).ok()
}

fn decode_bytes(bytes: &[u8]) -> Option<SkImage> {
    let data = skia_safe::Data::new_copy(bytes);
    SkImage::from_encoded(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Tiny 1x1 transparent PNG — the smallest valid PNG, used as a
    /// happy-path decode test that doesn't need a real fixture file.
    const TINY_PNG_B64: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";

    #[test]
    fn data_url_decodes_to_skia_image() {
        let url = format!("data:image/png;base64,{}", TINY_PNG_B64);
        let mut cache = ImageCache::new();
        let img = cache.get_or_decode(&ImageSource::DataUrl(url));
        assert!(img.is_some(), "expected the 1x1 PNG to decode");
        let img = img.unwrap();
        assert_eq!(img.width(), 1);
        assert_eq!(img.height(), 1);
    }

    #[test]
    fn cache_returns_same_image_on_second_lookup() {
        let url = format!("data:image/png;base64,{}", TINY_PNG_B64);
        let mut cache = ImageCache::new();
        let first = cache.get_or_decode(&ImageSource::DataUrl(url.clone()));
        assert!(first.is_some());
        let dims = (first.unwrap().width(), first.unwrap().height());
        let second = cache.get_or_decode(&ImageSource::DataUrl(url));
        assert!(second.is_some());
        assert_eq!((second.unwrap().width(), second.unwrap().height()), dims);
    }

    #[test]
    fn url_source_returns_none() {
        let mut cache = ImageCache::new();
        assert!(cache
            .get_or_decode(&ImageSource::Url("https://example.com/x.png".into()))
            .is_none());
    }

    #[test]
    fn malformed_data_url_returns_none() {
        let mut cache = ImageCache::new();
        assert!(cache
            .get_or_decode(&ImageSource::DataUrl("not a data url".into()))
            .is_none());
        assert!(cache
            .get_or_decode(&ImageSource::DataUrl(
                "data:image/png,not-base64".into()
            ))
            .is_none());
    }

    #[test]
    fn bytes_source_decodes() {
        let bytes = STANDARD.decode(TINY_PNG_B64).unwrap();
        let mut cache = ImageCache::new();
        let img = cache.get_or_decode(&ImageSource::Bytes(Arc::new(bytes)));
        assert!(img.is_some());
    }
}
