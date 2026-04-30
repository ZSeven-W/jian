//! Linux OpenGL-backed `SkiaSurface` factory (Plan 8 Task 2,
//! `gl` feature).
//!
//! ## Status
//!
//! **Skeleton only.** [`from_window`] currently panics — the real
//! implementation needs a GLX/EGL context plus a Skia GL backend
//! that co-operates with `jian-host-desktop::run`'s frame loop.
//! Until then `jian-host-desktop` keeps the softbuffer raster
//! presenter as the default.
//!
//! ## Implementation outline (for the follow-up)
//!
//! 1. Add `glutin = "..."` (or `glutin-winit` for direct
//!    integration) under
//!    `[target."cfg(target_os = \"linux\")".dependencies]` of
//!    jian-skia. Wayland and X11 each have their own GL context
//!    creation path; `glutin` papers over both.
//! 2. From the host's `raw_window_handle::WaylandWindowHandle` or
//!    `XlibWindowHandle`, create a `glutin::GlContext` and
//!    `glutin::Surface`. Make the context current on the host
//!    thread.
//! 3. Build `skia_safe::gpu::gl::Interface::new_native()` (loads
//!    GL function pointers from the active context) and
//!    `gpu::direct_contexts::make_gl(interface, ...)`.
//! 4. Each frame: query the framebuffer's
//!    `(width, height, sample_count, stencil_bits)`, build a
//!    `gpu::BackendRenderTarget::new_gl(...)` and
//!    `surfaces::wrap_backend_render_target(...)`. After the
//!    frame flushes, swap buffers via `glutin::Surface::swap_buffers`.
//! 5. Resize: drop the cached BRT, rebuild on the next frame.
//!    `glutin::Surface::resize` follows.

use crate::SkiaSurface;
use raw_window_handle::RawWindowHandle;

pub fn from_window(
    _raw_handle: RawWindowHandle,
    _width: i32,
    _height: i32,
) -> Result<SkiaSurface, &'static str> {
    Err(
        "jian-skia: OpenGL GPU surface not yet implemented; \
         `--features gl` enables the API surface but the platform \
         glue is a Plan 8 Task 2 follow-up. Hosts that hit this \
         error should fall back to `SkiaSurface::new_raster`.",
    )
}
