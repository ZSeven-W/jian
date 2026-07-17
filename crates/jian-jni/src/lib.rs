//! Android JNI marshalling layer for the jian engine (spec §6.7).
//!
//! `engine_thread` is the host-testable queue core and `registry` is the
//! host-testable handle table; the JNI bindings, callback trampolines, and
//! window ownership are Android-only modules (M4 plan Task 5).

pub mod engine_thread;
pub mod marshal;
pub mod registry;

#[cfg(target_os = "android")]
pub mod alog;
#[cfg(target_os = "android")]
pub mod callbacks;
#[cfg(target_os = "android")]
pub mod window;

pub use engine_thread::{Dispatch, EngineThread, STATUS_CLOSING};
pub use registry::Registry;
