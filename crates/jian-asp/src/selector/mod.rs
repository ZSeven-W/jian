//! ASP structured-JSON selector language (Plan 18 Task 2).
//!
//! Phase 1 (this module) ships the **types** — the tagged enum / struct
//! shape that an LLM-driven agent serialises into NDJSON line frames.
//! The runtime-side `resolve(&Doc, &Spatial) -> Vec<NodeKey>` evaluator
//! is a Plan 18 Task 2 follow-up (`resolve.rs`) that needs the live
//! `jian-core` document graph + spatial index.
//!
//! See the spec at `2026-04-17-jian-runtime-design.md` C18 for the
//! field semantics; see `2026-04-17-jian-plan-18-agent-shell-protocol.md`
//! Task 2 for the Phase 1 / Phase 2 split rationale.

#[cfg(feature = "dev-asp")]
pub mod types;

#[cfg(feature = "dev-asp")]
pub use types::{Combinator, Selector};
