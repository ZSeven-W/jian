//! MVP recognizers. Multi-pointer Scale + Rotate are stubbed pending
//! Plan 8 host-desktop multi-pointer driver. Scroll waits on the
//! `WheelEvent` wire-up (no `delta` field on `PointerEvent` yet —
//! adding that touches every host adapter and is parked alongside the
//! multi-pointer work).

pub mod hover;
pub mod long_press;
pub mod pan;
pub mod tap;

pub use hover::HoverRecognizer;
pub use long_press::LongPressRecognizer;
pub use pan::PanRecognizer;
pub use tap::{DoubleTapRecognizer, TapRecognizer};
