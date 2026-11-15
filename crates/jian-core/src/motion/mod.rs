//! Document motion runtime: easing curves, value interpolation, the bounded
//! animation timeline, lifecycle / in-view / transition triggers, time-based
//! clip sampling, screen transitions, and the curve compiler that exports the
//! runtime's curves as per-segment cubic-beziers.
//!
//! The runtime came from OpenPencil's `op-preview-core`, which wrote it
//! against these same `jian-core` types, so it moved here unchanged instead
//! of being re-implemented: OpenPencil's Preview, its canvas timeline and
//! exporter, and the Jian hosts now share one implementation. Golden vectors
//! recorded from the original implementation live in
//! `tests/golden/motion/` and pin its behaviour, quirks included.

pub mod clip;
pub mod compile;
pub mod curve;
pub mod declarations;
pub mod in_view;
pub mod interpolate;
pub mod scene_view;
pub mod screen_transition;
#[cfg(test)]
mod test_support;
pub mod timed_transition;
pub mod timeline;
pub mod triggers;

pub use clip::{Clip, ClipKeyframe, ClipTrigger, MotionOverrides};
pub use compile::{compile, CompiledAnimation, CompiledSegment};
pub use curve::{as_cubic, ease, ease_in_out_cubic, ease_out_cubic, inverse, CubicPiece};
pub use in_view::InViewWindow;
pub use scene_view::MotionSceneView;
pub use screen_transition::{
    NavigationKind, ResolvedScreenTransition, ScreenTransitionKind, ScreenTransitionSpec,
    TransitionLayer, TransitionScreen,
};
pub use timed_transition::TimedTransition;
pub use timeline::{invalidation_for, sample_request, MotionObserver, MotionTimeline};
pub use triggers::{MotionControl, MotionControlMode, MotionTriggers};
