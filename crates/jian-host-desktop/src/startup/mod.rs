//! Cold-start helpers for the desktop host (Plan 19 Task 7).
//!
//! Today this module exposes [`splash`] — the renderer + timer pair
//! that paints an `app.splash` config onto a `SkiaSurface` and tracks
//! the configured `min_duration_ms`. Plan 19 Task 7 also envisioned a
//! cross-fade animation from splash → real first frame; that lands as
//! a follow-up because it touches the winit run loop's frame-scheduling
//! invariants. The renderer + timer are testable building blocks the
//! cross-fade work can build on.

pub mod splash;

pub use splash::{SplashRenderer, SplashTimer};
