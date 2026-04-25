//! `jian-skia` — Skia-backed `RenderBackend` for the Jian runtime.
//!
//! The crate is backend-agnostic by default: the raster surface (created
//! via [`RenderBackend::new_surface`]) lets tests render without a GPU
//! context. Host adapters (desktop Plan 8, web Plan 12) drive Skia
//! through their own GPU-backed [`SkiaSurface`].
//!
//! Under the `textlayout` cargo feature this crate also exposes
//! [`measure::SkiaMeasure`] — a `jian_core::layout::measure::MeasureBackend`
//! that defers to `skia_safe::textlayout::Paragraph`. Hosts wire
//! it via `Runtime::build_layout_with` to align layout metrics
//! with what the renderer paints.
//!
//! [`RenderBackend::new_surface`]: jian_core::render::RenderBackend::new_surface
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
pub mod icons;
pub(crate) mod image;
#[cfg(feature = "textlayout")]
pub mod measure;
pub mod path;
pub mod surface;

pub use backend::SkiaBackend;
pub use surface::SkiaSurface;
#[cfg(feature = "textlayout")]
pub use measure::SkiaMeasure;
