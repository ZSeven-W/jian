//! Recognizer roster.
//!
//! - **Single-pointer (live)**: Tap / DoubleTap / LongPress / Pan /
//!   Hover.
//! - **Wheel (live)**: handled outside the arena via
//!   `Runtime::dispatch_wheel`.
//! - **Multi-pointer placeholders**: `ScaleRecognizer` and
//!   `RotateRecognizer` are skeletons that hold the registration
//!   slot. The arena's `priority.rs` already reserves their
//!   priority and the action surface already derives Scale/Rotate
//!   actions when authored. Bodies stay no-ops until the host
//!   driver routes two simultaneous PointerEvents through them
//!   (Plan 8 follow-on).

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
