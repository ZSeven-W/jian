//! Gesture Arena — Flutter-style pointer event pipeline and recognizer arbitration.

pub mod arena;
pub mod dispatcher;
pub mod focus;
pub mod hit;
pub mod ime;
pub mod keyboard;
pub mod pointer;
pub mod priority;
pub mod raw;
pub mod recognizer;
pub mod recognizers;
pub mod router;
pub mod semantic;

pub use dispatcher::dispatch_event;
pub use focus::{collect_focus_chain, FocusChange, FocusEvent, FocusManager};
pub use raw::find_raw_root;
pub use router::PointerRouter;

pub use arena::Arena;
pub use hit::{hit_test, HitPath};
pub use ime::{ImeEvent, ImeKind};
pub use keyboard::{KeyCode, KeyEvent, KeyLocation, KeyState, KeyValue, NamedKey};
pub use pointer::{
    Modifiers, MouseButtons, PointerEvent, PointerId, PointerKind, PointerPhase, ScrollMode,
    WheelEvent,
};
pub use recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
pub use semantic::SemanticEvent;
