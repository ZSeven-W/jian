//! AI Action Surface — derivation algorithm (Phase 1).
//!
//! Implements the pure, build-time-deterministic action derivation
//! defined by `2026-04-24-ai-action-surface.md` §3-§4. This module is
//! linked into every production runtime; the matching MCP server +
//! transport (Phase 2) lives in the `jian-action-surface` crate.
//!
//! ```text
//!  PenDocument (parsed)
//!         │
//!         ▼
//!   derive_actions(doc, build_salt) ──→  Vec<ActionDefinition>
//!                                              │
//!                                              │ (Phase 2)
//!                                              ▼
//!                                   list_available_actions / execute_action
//! ```
//!
//! The derivation is bitwise-stable for the same `(doc, build_salt)`
//! pair — the test suite asserts this invariant (`derive_is_deterministic`).

pub mod availability;
pub mod derive;
pub mod naming;
pub mod state_gate;
pub mod types;

pub use derive::{derive_actions, derive_actions_with_warnings, DeriveWarning};
pub use naming::{compute_slug, normalize_slug, short_hash};
pub use state_gate::RuntimeStateGate;
pub use types::{
    ActionDefinition, ActionName, AvailabilityStatic, ParamSpec, ParamTy, Scope, SourceKind,
};
