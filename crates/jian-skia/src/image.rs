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
//! Bounded LRU + soft byte cap. The cache evicts the least-recently
//! used entry once `decoded_byte_total > BYTE_CAP`. The cap defaults
//! to 128 MB (matching the Plan 7 / Plan 12 budget); hosts with
//! tighter memory targets construct a custom-sized cache via
//! `ImageCache::with_byte_cap`.
//!
//! Decoding failures are cached too (as `None`) so the same broken
//! data: URL doesn't burn CPU on every redraw — but `None` entries
//! count as zero bytes for the eviction calculation.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use jian_core::render::ImageSource;
use skia_safe::Image as SkImage;
use std::collections::HashMap;

/// Default soft byte cap — 128 MB. Plan 7 §C.1 budget.
pub(crate) const DEFAULT_BYTE_CAP: usize = 128 * 1024 * 1024;

struct CacheEntry {
    image: Option<SkImage>,
    /// Decoded RGBA byte estimate (0 on decode failure). Used by the
    /// eviction loop; doesn't have to be exact, just monotonic.
    bytes: usize,
    /// Monotonically increasing access timestamp — incremented on
    /// each `get_or_decode` hit to drive LRU.
    last_used: u64,
}

pub(crate) struct ImageCache {
    decoded: HashMap<String, CacheEntry>,
    byte_total: usize,
    byte_cap: usize,
    tick: u64,
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::with_byte_cap(DEFAULT_BYTE_CAP)
    }
}

impl ImageCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with a custom soft byte cap. Capacity is interpreted
    /// as decoded RGBA bytes; backing native objects (skia raster
    /// surfaces, mipmaps) typically add 10-20% on top.
    pub fn with_byte_cap(cap: usize) -> Self {
        Self {
            decoded: HashMap::new(),
            byte_total: 0,
            byte_cap: cap,
            tick: 0,
        }
    }

    /// Look up the source's cached image, decoding on first miss.
    /// Returns `None` if decoding failed or the source is a remote URL
    /// the cache cannot resolve — callers fall back to a placeholder.
    pub fn get_or_decode(&mut self, source: &ImageSource) -> Option<&SkImage> {
        let key = source.cache_key();
        if !self.decoded.contains_key(&key) {
            let decoded = decode(source);
            let bytes = estimate_bytes(&decoded);
            self.byte_total = self.byte_total.saturating_add(bytes);
            let tick = self.next_tick();
            self.decoded.insert(
                key.clone(),
                CacheEntry {
                    image: decoded,
                    bytes,
                    last_used: tick,
                },
            );
            self.evict_if_over_budget(&key);
        }
        let tick = self.next_tick();
        let entry = self.decoded.get_mut(&key)?;
        entry.last_used = tick;
        entry.image.as_ref()
    }

    /// Number of cached entries (decoded *and* failed). Test surface.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.decoded.len()
    }

    /// Total cached bytes — `0` for entries whose decode failed.
    #[cfg(test)]
    pub fn byte_total(&self) -> usize {
        self.byte_total
    }

    fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }

    /// Drop least-recently-used entries until `byte_total <= byte_cap`.
    /// `protect` is the just-inserted key — protected from same-call
    /// eviction *unless* it alone exceeds the cap, in which case
    /// caching it indefinitely would silently bypass the budget.
    /// Oversize protected entries are dropped immediately.
    fn evict_if_over_budget(&mut self, protect: &str) {
        // Drop other LRU victims first.
        loop {
            if self.byte_total <= self.byte_cap {
                return;
            }
            let victim = self
                .decoded
                .iter()
                .filter(|(k, _)| *k != protect)
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone());
            let Some(key) = victim else {
                break;
            };
            if let Some(removed) = self.decoded.remove(&key) {
                self.byte_total = self.byte_total.saturating_sub(removed.bytes);
            }
        }
        // If we're still over budget, the protected entry itself is
        // larger than the cap — refusing to evict means it lives
        // forever over budget. Drop it; callers fall back to the
        // grey placeholder which is cheaper than caching a
        // budget-busting image.
        if self.byte_total > self.byte_cap {
            if let Some(removed) = self.decoded.remove(protect) {
                self.byte_total = self.byte_total.saturating_sub(removed.bytes);
            }
        }
    }
}

/// Conservative RGBA-byte estimate. `width * height * 4` matches the
/// uncompressed cost of decoded image data in Skia.
fn estimate_bytes(image: &Option<SkImage>) -> usize {
    image
        .as_ref()
        .map(|img| (img.width() as usize) * (img.height() as usize) * 4)
        .unwrap_or(0)
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
            .get_or_decode(&ImageSource::DataUrl("data:image/png,not-base64".into()))
            .is_none());
    }

    #[test]
    fn bytes_source_decodes() {
        let bytes = STANDARD.decode(TINY_PNG_B64).unwrap();
        let mut cache = ImageCache::new();
        let img = cache.get_or_decode(&ImageSource::Bytes(Arc::new(bytes)));
        assert!(img.is_some());
    }

    #[test]
    fn lru_evicts_oldest_on_overflow() {
        // Two distinct decode-able sources whose cache_keys are stable
        // string literals (DataUrl) so eviction is testable without
        // worrying about Arc-pointer reuse. Each decodes to a 1×1
        // PNG = 4 RGBA bytes; cap = 4 → second insert tips over and
        // evicts the older entry.
        //
        // We synthesise the second payload by using a distinct
        // (still-valid) base64 PNG. `TINY_PNG_B64` is a 1×1 transparent
        // PNG; `TINY_PNG_B64_RED` is a 1×1 red PNG. Both decode fine.
        let mut cache = ImageCache::with_byte_cap(4);
        let src_a = ImageSource::DataUrl(format!("data:image/png;base64,{}", TINY_PNG_B64));
        let src_b = ImageSource::DataUrl(format!("data:image/png;base64,{}", TINY_PNG_B64_RED));
        assert!(cache.get_or_decode(&src_a).is_some());
        assert_eq!(cache.len(), 1);
        // Second insert tips the cap → older entry evicts.
        assert!(cache.get_or_decode(&src_b).is_some());
        assert_eq!(cache.len(), 1, "LRU should have evicted the oldest");
        // Re-insert the first — second now becomes the older entry
        // and gets evicted in turn.
        assert!(cache.get_or_decode(&src_a).is_some());
        assert_eq!(cache.len(), 1);
    }

    /// 1×1 red PNG, distinct from `TINY_PNG_B64`.
    const TINY_PNG_B64_RED: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

    #[test]
    fn under_budget_keeps_all_entries() {
        let mut cache = ImageCache::with_byte_cap(1024 * 1024);
        // Three distinct cache keys via three different Url strings;
        // they all decode to None but each takes its own cache slot.
        // Url decode-failure entries count as 0 bytes, so all three
        // sit comfortably under any cap.
        for n in 0..3 {
            cache.get_or_decode(&ImageSource::Url(format!("u-{}", n)));
        }
        assert_eq!(cache.len(), 3);
        assert!(cache.byte_total() <= 1024 * 1024);
    }
}
