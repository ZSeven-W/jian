//! `SkiaSurface` plus per-platform GPU surface factories (Plan 8 Task 2).
//!
//! Three creation paths exist:
//!
//! - **`raster`** (default, always available): CPU raster surface
//!   used by tests, the headless CLI fast path, and the current
//!   softbuffer presenter in `jian-host-desktop::run`. Constructed
//!   via [`SkiaSurface::new_raster`]; no GPU context needed.
//!
//! - **Platform GPU paths** (feature-gated): Metal on macOS / iOS,
//!   Direct3D 12 on Windows, OpenGL on Linux. Each ships under its
//!   own cargo feature (`metal` / `d3d` / `gl`) and is conditionally
//!   compiled by `target_os` so a default `cargo build` skips the
//!   per-platform pulldown. Hosts that opt into a GPU surface
//!   construct it with `surface::<platform>::from_window(...)`,
//!   passing the appropriate `raw-window-handle` view of the live
//!   winit window.
//!
//! - **WASM CanvasKit** (Plan 12 follow-up): future
//!   `surface/canvas_kit.rs`. Not in this skeleton.
//!
//! ## Why Plan 8 Task 2 ships skeleton-first
//!
//! The platform GPU code is not just a lookup of the right skia API:
//! it has to coordinate with the winit Window's lifecycle (the
//! drawable / swapchain owns a reference to the OS surface), the
//! resize handler (a backend render target sized for an old surface
//! draws garbage on the next frame), and the present cadence (Metal
//! drawables come from the layer queue; D3D12 swapchains have a
//! `Present()` call). Doing all three correctly per-platform needs
//! a session with the actual hardware in front of the developer.
//!
//! The skeleton keeps the API surface stable so:
//! - `jian-host-desktop::run` can pick a backend at startup and the
//!   subsequent code is platform-agnostic.
//! - Per-platform implementations land independently in follow-up
//!   commits without rippling through the host's render path.
//! - The raster fallback continues to work everywhere when the GPU
//!   path is not enabled.

mod raster;

#[cfg(all(target_os = "windows", feature = "d3d"))]
pub mod d3d;
#[cfg(all(target_os = "linux", feature = "gl"))]
pub mod gl;
#[cfg(all(target_os = "macos", feature = "metal"))]
pub mod metal;

pub use raster::SkiaSurface;
