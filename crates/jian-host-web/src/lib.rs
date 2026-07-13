//! CanvasKit-backed Jian web host.

mod backend;
mod canvaskit;
mod clock;
mod event;
mod fonts;
mod ime_input;
mod measure;
mod mount;
mod raf_pump;
mod runtime_slot;
pub mod services;
mod viewport;

pub use backend::CanvasKitBackend;
pub use fonts::FontRegistry;
pub use measure::CkMeasure;
pub use mount::{mount_jian, JianHandle};

#[cfg(all(test, target_arch = "wasm32"))]
#[path = "browser_tests/callback_reentry.rs"]
mod callback_reentry;

#[cfg(all(test, target_arch = "wasm32"))]
#[path = "browser_tests/canvaskit_semantics.rs"]
mod canvaskit_semantics;

#[cfg(all(test, target_arch = "wasm32"))]
#[path = "browser_tests/image_safety_reload.rs"]
mod image_safety_reload;

#[cfg(all(test, target_arch = "wasm32"))]
#[path = "browser_tests/production_bridges.rs"]
mod production_bridges;

#[cfg(all(test, target_arch = "wasm32"))]
#[path = "browser_tests.rs"]
mod tests;
