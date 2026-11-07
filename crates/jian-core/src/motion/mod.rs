//! Document motion runtime: easing curves, value interpolation, the bounded
//! animation timeline, lifecycle / in-view / transition triggers, time-based
//! clip sampling, and the curve compiler that exports the runtime's curves
//! as per-segment cubic-beziers.
//!
//! The runtime came from OpenPencil's `op-preview-core`, which wrote it
//! against these same `jian-core` types, so it moved here unchanged instead
//! of being re-implemented: OpenPencil's Preview, its canvas timeline and
//! exporter, and the Jian hosts now share one implementation. Golden vectors
//! recorded from the original implementation live in
//! `tests/golden/motion/` and pin its behaviour, quirks included.

pub mod curve;

pub use curve::{ease, ease_in_out_cubic, ease_out_cubic};
