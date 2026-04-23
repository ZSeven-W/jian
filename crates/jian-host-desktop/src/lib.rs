//! `jian-host-desktop` — winit + Skia desktop host for the Jian runtime.
//!
//! This crate wires [`jian_core::Runtime`] into a real OS window:
//!
//! - [`mod@pointer`] translates `winit::event::WindowEvent` mouse / touch
//!   input into `jian_core::gesture::PointerEvent`.
//! - [`keyboard`] translates `winit::event::KeyEvent` into a neutral
//!   key-string + `Modifiers` pair.
//! - [`services`] ships host-agnostic implementations of the Plan 4
//!   platform service traits (clipboard / storage / router) suitable
//!   for the desktop environment.
//! - [`host`] owns the end-to-end loop: surface creation, event
//!   dispatch, frame scheduling. The loop is feature-gated (`run`)
//!   because headless CI can't open a window.
//!
//! ## Minimum viable loop
//!
//! ```no_run
//! use jian_core::Runtime;
//! use jian_host_desktop::DesktopHost;
//!
//! let mut rt = Runtime::new();
//! rt.load_str(include_str!("../../jian-ops-schema/tests/corpus/minimal.op")).unwrap();
//! rt.build_layout((800.0, 600.0)).unwrap();
//! rt.rebuild_spatial();
//!
//! let host = DesktopHost::new(rt, "Jian");
//! // host.run();  // real window — only available with the `run` feature.
//! drop(host);
//! ```

pub mod host;
pub mod keyboard;
pub mod pointer;
pub mod scene;
pub mod services;

#[cfg(feature = "run")]
mod run;

pub use host::DesktopHost;
pub use scene::collect_draws;
