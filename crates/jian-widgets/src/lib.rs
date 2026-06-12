//! `jian-widgets` — cross-platform component painting primitives.
//!
//! `jian-core` owns platform-free state machines, layout, and document
//! runtime types. This crate owns the immediate-mode paint contract and
//! reusable themed components. Platform hosts implement [`Painter`];
//! widget code never sees platform-specific canvas or GPU types.

pub mod components;
pub mod geometry;
pub mod painter;
pub mod tokens;

#[cfg(test)]
pub(crate) mod test_support;

pub use geometry::{Color, Point2D, Rect};
pub use painter::{ImageAdjustments, ImageDrawMode, Painter, TextLayout};
pub use tokens::{Density, Tokens};
