//! MVP recognizers. Scale + Rotate (multi-pointer) are stubbed out pending
//! Plan 9 host-desktop work.

pub mod hover;
pub mod long_press;
pub mod pan;
pub mod tap;

pub use hover::HoverRecognizer;
pub use long_press::LongPressRecognizer;
pub use pan::PanRecognizer;
pub use tap::{DoubleTapRecognizer, TapRecognizer};
