//! `SkiaSurface` — owns a Skia `Surface` + provides PNG encoding for tests.
//!
//! Three creation modes are supported:
//! 1. [`SkiaSurface::new_raster`] — CPU raster; default for tests and
//!    headless rendering. Works everywhere without a GPU context.
//! 2. `SkiaSurface::from_backend_surface` (future, Plan 8) — wraps a
//!    host-provided `SkSurface` backed by Metal / D3D12 / GL / Vulkan.
//! 3. WASM path (Plan 12) — same as above, backed by WebGPU/WebGL.

use skia_safe::{EncodedImageFormat, Surface};

pub struct SkiaSurface {
    pub(crate) inner: Surface,
}

impl SkiaSurface {
    /// Build a CPU raster surface at `width × height` logical pixels.
    pub fn new_raster(width: i32, height: i32) -> Self {
        let inner = skia_safe::surfaces::raster_n32_premul((width, height))
            .expect("skia raster surface allocation failed");
        Self { inner }
    }

    /// Snapshot the surface into a PNG byte vector. Useful for golden
    /// tests and for debugging.
    pub fn encode_png(&mut self) -> Option<Vec<u8>> {
        let image = self.inner.image_snapshot();
        let mut ctx = self.inner.direct_context();
        let data = image.encode(ctx.as_mut(), EncodedImageFormat::PNG, None)?;
        Some(data.as_bytes().to_vec())
    }

    /// Access the underlying canvas. Re-borrow on every frame.
    pub fn canvas(&mut self) -> &skia_safe::Canvas {
        self.inner.canvas()
    }

    pub fn width(&self) -> i32 {
        self.inner.width()
    }
    pub fn height(&self) -> i32 {
        self.inner.height()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_surface_has_requested_size() {
        let s = SkiaSurface::new_raster(200, 100);
        assert_eq!(s.width(), 200);
        assert_eq!(s.height(), 100);
    }

    #[test]
    fn blank_surface_encodes_to_png() {
        let mut s = SkiaSurface::new_raster(16, 16);
        let png = s.encode_png().expect("encode");
        // Minimum PNG signature + IHDR is 8 + 25 bytes.
        assert!(png.len() > 33);
        assert_eq!(&png[0..8], b"\x89PNG\r\n\x1a\n");
    }
}
