//! `jian-skia` — Skia-backed `RenderBackend` for the Jian runtime.
//!
//! The crate is backend-agnostic by default: the raster surface (created
//! via [`RenderBackend::new_surface`]) lets tests render without a GPU
//! context. Host adapters (desktop Plan 8, web Plan 12) drive Skia
//! through their own GPU-backed [`SkiaSurface`].
//!
//! Under the `textlayout` cargo feature this crate also exposes
//! `measure::SkiaMeasure` — a `jian_core::layout::measure::MeasureBackend`
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
#[cfg(feature = "textlayout")]
pub mod bundled_fonts;
pub mod color;
pub mod convert;
// Always compiled (not `textlayout`-gated): the non-textlayout paint path
// (`backend::draw_text`) also creates a `FontMgr` and must serialize with it.
pub mod font_lock;
#[cfg(feature = "textlayout")]
pub mod font_resolve;
pub mod icons;
pub(crate) mod image;
pub mod image_registry;
#[cfg(feature = "textlayout")]
pub mod measure;
pub mod path;
pub mod shader_cache;
pub mod startup;
pub mod surface;

pub use backend::SkiaBackend;
#[cfg(feature = "textlayout")]
pub use bundled_fonts::{
    generation as font_generation, list_families, parse_imported_font_meta, register_bundled_fonts,
    register_imported_font, remove_imported_font, FamilyMeta, FontBlob, FontSource,
    ImportedFontMeta,
};
pub use font_lock::with_font_lock;
#[cfg(feature = "textlayout")]
pub use font_resolve::{
    FontResolver, FontSegment, ResolvedTypeface, SYNTHETIC_BOLD_WIDTH_FACTOR, SYNTHETIC_ITALIC_SKEW,
};
pub use image_registry::{InstanceImageRegistry, RegisteredBackend};
#[cfg(feature = "textlayout")]
pub use measure::SkiaMeasure;
pub use shader_cache::ShaderCache;
pub use surface::SkiaSurface;
