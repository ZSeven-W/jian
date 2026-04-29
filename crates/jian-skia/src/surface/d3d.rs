//! Windows Direct3D 12-backed `SkiaSurface` factory (Plan 8 Task 2,
//! `d3d` feature).
//!
//! ## Status
//!
//! **Skeleton only.** [`from_window`] currently panics — the real
//! implementation needs a DXGI swapchain + ID3D12Device + ID3D12CommandQueue
//! that co-operates with `jian-host-desktop::run`'s frame loop.
//! Until then `jian-host-desktop` keeps the softbuffer raster
//! presenter as the default.
//!
//! ## Implementation outline (for the follow-up)
//!
//! 1. Add `windows = "..."` (the official Microsoft binding crate)
//!    under `[target."cfg(target_os = \"windows\")".dependencies]`
//!    of jian-skia, with the `Win32_Graphics_Direct3D12` /
//!    `Win32_Graphics_Dxgi` / `Win32_Graphics_Direct3D` features.
//! 2. From the host's `raw_window_handle::Win32WindowHandle`:
//!    - Create an `IDXGIFactory4`.
//!    - Create an `ID3D12Device` (warp-fallback if no hardware
//!      adapter is acceptable).
//!    - Create a direct `ID3D12CommandQueue`.
//!    - Create a flip-discard `IDXGISwapChain3` over the HWND.
//! 3. Build `skia_safe::gpu::d3d::BackendContext { adapter, device,
//!    queue, ... }` and feed it to
//!    `gpu::direct_contexts::make_direct3d`.
//! 4. Each frame: get the swapchain's current back-buffer texture,
//!    wrap as `gpu::BackendRenderTarget::new_d3d` and
//!    `surfaces::wrap_backend_render_target(...)`. After the frame
//!    flushes, call `swapchain.Present(1, 0)` for vsync (or `(0, 0)`
//!    for immediate).
//! 5. Resize: call `swapchain.ResizeBuffers(...)` + drop the cached
//!    `BackendRenderTarget`s; rebuild on the next frame.

use crate::SkiaSurface;
use raw_window_handle::RawWindowHandle;

pub fn from_window(_raw_handle: RawWindowHandle, _width: i32, _height: i32) -> SkiaSurface {
    panic!(
        "jian-skia: D3D12 GPU surface not yet implemented; \
         `--features d3d` enables the API surface but the platform \
         glue is a Plan 8 Task 2 follow-up. Fall back to \
         `SkiaSurface::new_raster` for now."
    );
}
