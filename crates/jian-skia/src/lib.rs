//! `jian-skia` — Skia-backed `RenderBackend` for the Jian runtime.
//!
//! The crate is backend-agnostic by default: the raster surface (created
//! with [`SkiaBackend::new_raster`]) lets tests render without a GPU
//! context. Host adapters (desktop Plan 8, web Plan 12) plug in a
//! platform-specific GPU surface via [`SkiaBackend::new_with_surface`].
//!
//! ```no_run
//! use jian_core::geometry::{rect, size};
//! use jian_core::render::{DrawOp, Paint, RenderBackend};
//! use jian_core::scene::Color;
//! use jian_skia::SkiaBackend;
//!
//! let mut backend = SkiaBackend::new();
//! let mut surface = backend.new_surface(size(100.0, 100.0));
//! backend.begin_frame(&mut surface, 0xffffffff);
//! // Trait calls are buffered; `end_frame` replays them onto the canvas.
//! backend.draw(&DrawOp::Rect {
//!     rect: rect(10.0, 10.0, 80.0, 80.0),
//!     paint: Paint::solid(Color::rgb(0x1e, 0x88, 0xe5)),
//! });
//! backend.end_frame(&mut surface);
//! let png = surface.encode_png().unwrap();
//! ```

pub mod backend;
pub mod color;
pub mod convert;
pub mod path;
pub mod surface;

pub use backend::SkiaBackend;
pub use surface::SkiaSurface;
