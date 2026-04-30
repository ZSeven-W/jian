//! macOS / iOS Metal-backed `SkiaSurface` factory (Plan 8 Task 2,
//! `metal` feature).
//!
//! ## Status
//!
//! **Skeleton only.** [`from_window`] currently panics with a clear
//! message — the real implementation needs a focused session against
//! a real CAMetalLayer + MTLDevice + MTLCommandQueue lifecycle that
//! co-operates with `jian-host-desktop::run`'s frame loop. Until
//! then `jian-host-desktop` keeps the existing softbuffer raster
//! presenter as the default; hosts that flip the `metal` feature on
//! see this panic and know to gate their startup on a real impl.
//!
//! ## Implementation outline (for the follow-up)
//!
//! 1. Pull `metal = "..."` (RustCrypto-style crate that wraps
//!    Foundation's MTL types) under `[target."cfg(target_os =
//!    \"macos\")".dependencies]` of jian-skia.
//! 2. Convert the host-supplied `raw_window_handle::AppKitWindowHandle`
//!    into the `NSView`'s `CAMetalLayer` (creating one if the view
//!    doesn't already have a Metal-backed layer; on macOS winit
//!    leaves layer creation to the host).
//! 3. Build a `skia_safe::gpu::mtl::BackendContext` from
//!    `(MTLDevice, MTLCommandQueue)` and feed it to
//!    `gpu::direct_contexts::make_metal`.
//! 4. Each frame: pull the next `CAMetalDrawable` from the layer,
//!    wrap its `MTLTexture` in
//!    `gpu::BackendRenderTarget::new_metal((w, h), texture)` and
//!    `surfaces::wrap_backend_render_target(...)`. The returned
//!    `SkSurface` is what `SkiaSurface` carries.
//! 5. After `end_frame` flushes, ask the drawable to `present()` and
//!    drop it before pulling the next one.
//! 6. Resize: drop the cached `BackendRenderTarget`s and rebuild on
//!    the next frame; the layer's drawableSize follows the window's
//!    physical size automatically.
//!
//! See `crates/jian-host-desktop/src/run.rs::redraw` for the call
//! site that today drives `SkiaSurface::new_raster`. The eventual
//! GPU-aware redraw will pick this factory based on `HostConfig`.

use crate::SkiaSurface;
use raw_window_handle::RawWindowHandle;

/// Build a Metal-backed `SkiaSurface` of `width × height` physical
/// pixels for the supplied window handle. Returns an `Err` while
/// the platform glue is unimplemented so a host can fall back to
/// the raster surface gracefully — earlier draft `panic!`-ed,
/// which would crash any host that flipped on `--features metal`
/// before the real impl lands.
///
/// `raw_handle` is taken by value because the eventual
/// implementation needs to retain Foundation references for the
/// layer's lifetime; the caller passes a handle from
/// `winit::window::Window::window_handle()` and surrenders
/// ownership of the bridge.
pub fn from_window(
    _raw_handle: RawWindowHandle,
    _width: i32,
    _height: i32,
) -> Result<SkiaSurface, &'static str> {
    Err(
        "jian-skia: Metal GPU surface not yet implemented; \
         `--features metal` enables the API surface but the platform \
         glue is a Plan 8 Task 2 follow-up. Hosts that hit this \
         error should fall back to `SkiaSurface::new_raster`.",
    )
}
