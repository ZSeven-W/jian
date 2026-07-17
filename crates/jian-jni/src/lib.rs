//! Android JNI marshalling layer for the jian engine (spec §6.7).
//!
//! `engine_thread` is the host-testable queue core and `registry` is the
//! host-testable handle table; the JNI bindings, callback trampolines, and
//! window ownership are Android-only modules (M4 plan Task 5).

pub mod engine_thread;
pub mod registry;

pub use engine_thread::{Dispatch, EngineThread, STATUS_CLOSING};
pub use registry::Registry;
