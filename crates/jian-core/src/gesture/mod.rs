//! Gesture Arena — Flutter-style pointer event pipeline and recognizer arbitration.

pub mod arena;
pub mod dispatcher;
pub mod focus;
pub mod hit;
pub mod pointer;
pub mod priority;
pub mod raw;
pub mod recognizer;
pub mod recognizers;
pub mod router;
pub mod semantic;

pub use dispatcher::dispatch_event;
pub use focus::FocusManager;
pub use raw::find_raw_root;
pub use router::PointerRouter;

pub use arena::Arena;
pub use hit::{hit_test, HitPath};
pub use pointer::{Modifiers, MouseButtons, PointerEvent, PointerId, PointerKind, PointerPhase};
pub use recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
pub use semantic::SemanticEvent;
