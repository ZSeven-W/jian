//! `jian-core` — Jian UI framework core.
//!
//! This crate provides the three primary abstractions the rest of Jian builds on:
//!
//! - [`document`] — the runtime representation of a loaded `.op` file.
//! - [`signal`] — fine-grained reactivity primitives (Signals + Effects).
//! - [`scene`] — a resolved render-ready view of the document.
//!
//! It also defines two extension traits that host crates implement:
//!
//! - [`render::RenderBackend`] — how to turn the scene graph into pixels.
//! - [`logic::LogicProvider`] — how to execute Tier 3 logic modules (reserved for L4).

pub mod error;

pub use error::{CoreError, CoreResult};

#[cfg(test)]
mod sanity {
    #[test]
    fn smoke() {
        assert_eq!(2 + 2, 4);
    }
}
