//! Per-host registered-image adapter.
//!
//! Bytes are owned by the runtime/host pair, never by process-global state.
//! The wrapped `SkiaBackend` remains per-host and therefore its decoded-image
//! cache is isolated to the same runtime. Keyed draws are translated to byte
//! draws before reaching that cache.

use crate::SkiaBackend;
use jian_core::geometry::{Affine2, Rect, Size};
use jian_core::render::{DecodeError, DrawOp, ImageSource, RenderBackend, ShadowSpec};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct InstanceImageRegistry {
    images: HashMap<String, Arc<Vec<u8>>>,
    pinned_bytes: usize,
}

impl InstanceImageRegistry {
    pub fn get(&self, key: &str) -> Option<Arc<Vec<u8>>> {
        self.images.get(key).cloned()
    }

    pub fn pinned_bytes(&self) -> usize {
        self.pinned_bytes
    }

    fn insert(&mut self, key: &str, bytes: &[u8]) {
        let bytes = Arc::new(bytes.to_vec());
        if let Some(old) = self.images.insert(key.to_owned(), bytes.clone()) {
            self.pinned_bytes = self.pinned_bytes.saturating_sub(old.len());
        }
        self.pinned_bytes = self.pinned_bytes.saturating_add(bytes.len());
    }

    fn remove(&mut self, key: &str) {
        if let Some(old) = self.images.remove(key) {
            self.pinned_bytes = self.pinned_bytes.saturating_sub(old.len());
        }
    }
}

pub struct RegisteredBackend<'a> {
    pub inner: &'a mut SkiaBackend,
    pub images: &'a mut InstanceImageRegistry,
}

impl RenderBackend for RegisteredBackend<'_> {
    type Surface = <SkiaBackend as RenderBackend>::Surface;

    fn new_surface(&mut self, size: Size) -> Self::Surface {
        self.inner.new_surface(size)
    }
    fn begin_frame(&mut self, surface: &mut Self::Surface, clear: u32) {
        self.inner.begin_frame(surface, clear);
    }
    fn end_frame(&mut self, surface: &mut Self::Surface) {
        self.inner.end_frame(surface);
    }
    fn push_clip(&mut self, rect: Rect) {
        self.inner.push_clip(rect);
    }
    fn push_transform(&mut self, m: &Affine2) {
        self.inner.push_transform(m);
    }
    fn pop(&mut self) {
        self.inner.pop();
    }
    fn push_layer(&mut self, bounds: Rect) {
        self.inner.push_layer(bounds);
    }
    fn pop_layer(&mut self) {
        self.inner.pop_layer();
    }
    fn apply_blur(&mut self, sigma: f32) {
        self.inner.apply_blur(sigma);
    }
    fn apply_shadow(&mut self, shadow: &ShadowSpec) {
        self.inner.apply_shadow(shadow);
    }

    fn draw(&mut self, op: &DrawOp) {
        if let DrawOp::Image {
            source: ImageSource::Url(key),
            dst,
            opacity,
        } = op
        {
            if let Some(bytes) = self.images.get(key) {
                self.inner.draw(&DrawOp::Image {
                    source: ImageSource::Bytes(bytes),
                    dst: *dst,
                    opacity: *opacity,
                });
                return;
            }
        }
        self.inner.draw(op);
    }

    fn register_image(&mut self, key: &str, bytes: &[u8]) -> Result<(), DecodeError> {
        jian_core::render::probe_image_bounds(bytes)?;
        self.images.insert(key, bytes);
        Ok(())
    }

    fn release_image(&mut self, key: &str) {
        self.images.remove(key);
    }
}
