//! `jian-widgets` — cross-platform component painting primitives.
//!
//! `jian-core` owns platform-free state machines, layout, and document
//! runtime types. This crate owns the immediate-mode paint contract and
//! reusable themed components. Platform hosts implement [`Painter`];
//! widget code never sees platform-specific canvas or GPU types.

pub mod components;
pub mod geometry;
pub mod painter;
pub mod text_box;
pub mod tokens;

#[cfg(test)]
pub(crate) mod test_support;

pub use geometry::{Color, Point2D, Rect};
pub use painter::{ImageAdjustments, ImageDrawMode, Painter, TextLayout};
pub use text_box::{HorizontalAlign, TextBox, VerticalAlign};
pub use tokens::{Density, Tokens};

/// The baseline `y` that vertically centers `font_size` text in the box
/// `rect`. **`Painter::draw_text` positions text by its BASELINE** — every host
/// backend (op-host-native, op-host-web) draws at `origin.y` directly (Skia
/// `draw_str` is baseline-relative), so centering is `center + ~cap-height/2`,
/// not the `(height - font_size)/2` top-left form. Components MUST route every
/// label's `y` through this so jian text lines up with the host chrome instead
/// of riding ~`font_size` too high.
pub fn centered_text_baseline_y(rect: Rect, font_size: f32) -> f32 {
    rect.origin.y + rect.size.y / 2.0 + font_size * 0.35
}
