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

pub mod clip;
pub mod curve;
pub mod declarations;
pub mod in_view;
pub mod interpolate;
pub mod scene_view;
#[cfg(test)]
mod test_support;
pub mod timeline;
pub mod triggers;

pub use clip::{Clip, ClipKeyframe, ClipTrigger, MotionOverrides};
pub use curve::{as_cubic, ease, ease_in_out_cubic, ease_out_cubic, inverse, CubicPiece};
pub use in_view::InViewWindow;
pub use scene_view::MotionSceneView;
pub use timeline::{invalidation_for, sample_request, MotionObserver, MotionTimeline};
pub use triggers::{MotionControl, MotionControlMode, MotionTriggers};
