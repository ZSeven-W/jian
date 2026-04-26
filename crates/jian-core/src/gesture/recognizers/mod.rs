//! Recognizer roster.
//!
//! - **Single-pointer (live)**: Tap / DoubleTap / LongPress / Pan /
//!   Hover.
//! - **Wheel (live)**: handled outside the arena via
//!   `Runtime::dispatch_wheel`.
//! - **Multi-pointer (live)**: `ScaleRecognizer` and
//!   `RotateRecognizer` track the first two pointers in arrival
//!   order, claim past 5% / 5° respectively, and emit
//!   Start/Update/End. They're owned by `PointerRouter::multi`
//!   (cross-arena pool, Plan 5 §B.2) — when one Claims, the
//!   participating per-pointer arenas are cancelled so single-
//!   finger Tap / Pan / LongPress lose to the multi gesture.

pub mod hover;
pub mod long_press;
pub mod pan;
pub mod rotate;
pub mod scale;
pub mod tap;

pub use hover::HoverRecognizer;
pub use long_press::LongPressRecognizer;
pub use pan::PanRecognizer;
pub use rotate::RotateRecognizer;
pub use scale::ScaleRecognizer;
pub use tap::{DoubleTapRecognizer, TapRecognizer};
