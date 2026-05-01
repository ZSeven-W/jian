//! Jian Agent Shell Protocol (ASP) — agent driving channel for both
//! dev tooling and production AI clients (Plan 18).
//!
//! Spec: `2026-04-17-jian-runtime-design.md` C18.
//! Plan: `2026-04-17-jian-plan-18-agent-shell-protocol.md`,
//! Plan 18 ASP prod mode follow-up: `2026-05-01-jian-plan-18-asp-production-mode.md`.
//!
//! # Two surfaces, one crate
//!
//! ASP exposes **two opt-in surfaces** sharing the same NDJSON
//! protocol envelope, selector resolver, session / token validator,
//! transport, and operation verbs. Hosts pick one (or both, for
//! tests) at compile time:
//!
//! - **`dev-asp` — full debug surface.** Adds `find` / `inspect` /
//!   `snapshot` / `audit` / `set_state` / expression evaluator on
//!   top of the operation verbs. Wide-open by design — meant for
//!   UI testing, debugging, accessibility automation. Production
//!   hosts must NOT enable this; `.github/workflows/ci-action-surface-isolation.yml`
//!   fails any PR whose `cargo tree` for a release host's MCP /
//!   player feature combo includes `jian-asp` via `dev-asp`.
//! - **`prod-asp` — lean production surface.** Only `handshake` /
//!   `list_actions` / `tap` / `type` / `scroll` / `swipe` / `exit`.
//!   No structural readers, no expression evaluator, no direct
//!   state writes. Built for token-efficient AI driving against
//!   shipping apps; the spec's `list_actions` flat projection
//!   (`<id, events>` rows) makes it ~4-8× cheaper per session than
//!   MCP's `tools/list` preamble + per-call envelope.
//!
//! Both opt-in features are off by default. `--no-default-features`
//! still produces the empty Phase 0 skeleton — no serde, no
//! modules, just the gating attribute pattern (regression guard
//! against accidental code landing without an ASP feature gate).
//!
//! # Why two surfaces, not one
//!
//! Action Surface (`jian-action-surface`, Plan 22) remains the
//! production AI channel for *business* actions — narrow, derived,
//! gated. ASP **prod** is the production *automation* channel — wide
//! verbs (tap / type / scroll / swipe) targeting `list_actions` ids
//! that match Action Surface's `<scope>.<verb>_<slug>_<hash4>`
//! derivation. ASP **dev** stays as the structural-introspection
//! channel agents would be a leak vector in production (threat
//! model T1).
//!
//! Build-time isolation (the `dev-asp` cargo feature bound) is
//! enforced **physically**, not by convention: a release binary
//! that opts into `prod-asp` literally cannot reach `find` /
//! `inspect` / `snapshot` / etc. because those modules aren't
//! compiled in. Codex round 1 (C3) verified the per-handler
//! gating + dispatch stub make this airtight.

#![cfg_attr(not(any(feature = "dev-asp", feature = "prod-asp")), allow(dead_code))]

// Plan 18 Task 1 — protocol types (Request / Response / Verb tagged
// enum / OutcomePayload). The whole `pub mod protocol;` declaration
// is feature-gated so a `cargo build --no-default-features` of
// jian-asp is byte-empty (no serde dep, no module tree).
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod protocol;

// Plan 18 Task 2 — Selector types (Phase 1 types-only). Same gating
// as `protocol`. The runtime-side `resolve(&Doc, &Spatial) ->
// Vec<NodeKey>` is a follow-up (`selector/resolve.rs`) once the
// runtime borrows are settled.
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod selector;

// Plan 18 Tasks 3+4 — verb dispatch + per-verb handlers. Phase 2
// ships `find` / `inspect` (node_props + route) / `audit` / `exit`;
// remaining verbs return `OutcomePayload::error("not yet
// implemented")` so the wire surface stays uniform while the
// runtime-coupled work lands incrementally.
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod verb_impls;

// Plan 18 Task 5 — transport layer. Phase 2 ships the trait
// abstraction + stdio variant; Unix socket / Named Pipe /
// WebSocket land additively behind the same trait.
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod transport;

// Plan 18 Task 6 — session state. Token validation is delegated
// to a `TokenValidator` impl the host installs; this module owns
// the permission tier, audit ring, and handshake handling.
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod session;

// Plan 18 Task 7 — server main loop. `run_session` accepts a
// transport + validator + runtime borrow and drives the full
// lifecycle (handshake → request loop → exit).
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
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
