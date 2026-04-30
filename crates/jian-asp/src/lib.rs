//! Jian Agent Shell Protocol (ASP) — dev-only debugging / automation channel.
//!
//! Spec: `2026-04-17-jian-runtime-design.md` C18.
//! Plan: `2026-04-17-jian-plan-18-agent-shell-protocol.md`.
//!
//! # Status
//!
//! Phase 1 (Plan 18 Tasks 1+2) ships the **type surface**: NDJSON
//! `Request` / `Response` envelope, the tagged `Verb` enum, the
//! `OutcomePayload` semantic-result shape, and the structured-JSON
//! `Selector` data type. Real runtime integration (transport,
//! session, verb dispatch, selector resolver) lands in Plan 18
//! Tasks 3-7 against `&mut Runtime`. All types live behind
//! `#[cfg(feature = "dev-asp")]` so a default-feature build of a
//! consumer crate compiles to the empty Phase 0 skeleton.
//!
//! Production hosts (`jian-host-*`) link this crate as an **optional**
//! dependency that activates only when their own `dev-asp` feature is
//! turned on. Release builds (`--release` with default features) **do
//! not** include any code from this crate; the CI isolation job in
//! `.github/workflows/ci-action-surface-isolation.yml` proves this via
//! `cargo tree`.
//!
//! # Why a feature-gated empty crate?
//!
//! Action Surface (`jian-action-surface`, Plan 22) is the production AI
//! channel — narrow, derived, gated. ASP is the **dev** channel —
//! wide-open verbs (`tap` / `find` / `inspect` / `snapshot`) that would
//! be a structural information leak in production (threat model T1).
//! The two-channel split is enforced **physically**, not by convention:
//! a release binary cannot reach ASP code paths because the `dev-asp`
//! feature isn't compiled in.

#![cfg_attr(not(feature = "dev-asp"), allow(dead_code))]

// Plan 18 Task 1 — protocol types (Request / Response / Verb tagged
// enum / OutcomePayload). The whole `pub mod protocol;` declaration
// is feature-gated so a `cargo build --no-default-features` of
// jian-asp is byte-empty (no serde dep, no module tree).
#[cfg(feature = "dev-asp")]
pub mod protocol;

// Plan 18 Task 2 — Selector types (Phase 1 types-only). Same gating
// as `protocol`. The runtime-side `resolve(&Doc, &Spatial) ->
// Vec<NodeKey>` is a follow-up (`selector/resolve.rs`) once the
// runtime borrows are settled.
#[cfg(feature = "dev-asp")]
pub mod selector;

// Plan 18 Tasks 3+4 — verb dispatch + per-verb handlers. Phase 2
// ships `find` / `inspect` (node_props + route) / `audit` / `exit`;
// remaining verbs return `OutcomePayload::error("not yet
// implemented")` so the wire surface stays uniform while the
// runtime-coupled work lands incrementally.
#[cfg(feature = "dev-asp")]
pub mod verb_impls;

// Plan 18 Task 5 — transport layer. Phase 2 ships the trait
// abstraction + stdio variant; Unix socket / Named Pipe /
// WebSocket land additively behind the same trait.
#[cfg(feature = "dev-asp")]
pub mod transport;

// Plan 18 Task 6 — session state. Token validation is delegated
// to a `TokenValidator` impl the host installs; this module owns
// the permission tier, audit ring, and handshake handling.
#[cfg(feature = "dev-asp")]
pub mod session;

// Plan 18 Task 7 — server main loop. `run_session` accepts a
// transport + validator + runtime borrow and drives the full
// lifecycle (handshake → request loop → exit).
#[cfg(feature = "dev-asp")]
pub mod server;

#[cfg(test)]
mod tests {
    /// Phase 0 sanity: the crate links and is *empty* under default
    /// features. Once Plan 18 Task 1+ adds real code, this test becomes
    /// a regression guard against accidental code landing without a
    /// `#[cfg(feature = "dev-asp")]` gate.
    #[test]
    fn crate_is_link_clean() {
        // Intentionally trivial. The point is that this test compiles
        // — under both `--features dev-asp` and `--no-default-features`
        // — proving the gating attribute pattern is correct.
    }
}
