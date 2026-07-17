//! Android JNI marshalling layer for the jian engine (spec §6.7).
//!
//! `engine_thread` is the host-testable queue core; the JNI bindings,
//! callback trampolines, window ownership, and handle registry are
//! Android-only modules (M4 plan Task 5).

pub mod engine_thread;

pub use engine_thread::{Dispatch, EngineThread, STATUS_CLOSING};
