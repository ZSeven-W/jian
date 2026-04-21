//! Gesture Arena — Flutter-style pointer event pipeline and recognizer arbitration.

pub mod arena;
pub mod hit;
pub mod pointer;
pub mod priority;
pub mod recognizer;
pub mod semantic;

pub use arena::Arena;
pub use hit::{hit_test, HitPath};
pub use pointer::{Modifiers, MouseButtons, PointerEvent, PointerId, PointerKind, PointerPhase};
pub use recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
pub use semantic::SemanticEvent;
