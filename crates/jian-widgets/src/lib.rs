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
pub use painter::{ImageAdjustments, ImageDrawMode, Painter, TextLayout, TextMetrics};
pub use text_box::{HorizontalAlign, TextBox, VerticalAlign};
pub use tokens::{Density, Tokens};

/// Legacy baseline-like helper retained temporarily for downstream callers.
///
/// `Painter::draw_text` now has a top-left contract. New and migrated controls
/// must use [`TextBox`] so host font metrics determine the visible ink center.
#[deprecated(note = "use TextBox with VerticalAlign::Center")]
pub fn centered_text_baseline_y(rect: Rect, font_size: f32) -> f32 {
    rect.origin.y + rect.size.y / 2.0 + font_size * 0.35
}
