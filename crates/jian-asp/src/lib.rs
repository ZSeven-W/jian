//! Jian Agent Shell Protocol (ASP) — dev-only debugging / automation channel.
//!
//! Spec: `2026-04-17-jian-runtime-design.md` C18.
//! Plan: `2026-04-17-jian-plan-18-agent-shell-protocol.md`.
//!
//! # Status
//!
//! This is the **Phase 0 skeleton** — an empty crate that exists solely to
//! own the `dev-asp` feature gate. Plan 18 Task 0 explicitly designates
//! this crate as the single owner of dev/prod isolation; subsequent tasks
//! (Task 1 protocol, Task 2 selectors, Task 3 verbs, …) fill in the
//! `protocol` / `selector` / `verb_impls` / `transport` / `server` modules
//! behind `#[cfg(feature = "dev-asp")]`.
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

#[cfg(feature = "dev-asp")]
pub mod protocol {
    //! Phase 1 placeholder — Plan 18 Task 1 fills in `Request` / `Response`
    //! types + the verb enum (`tap` / `type` / `find` / `wait_for` / etc.).
}

#[cfg(feature = "dev-asp")]
pub mod selector {
    //! Phase 1 placeholder — Plan 18 Task 2 fills in the structured-JSON
    //! selector language (`id` / `alias` / `role` / `text_contains` /
    //! `child_of` / `all_of` / etc.) and `resolve(&Doc, &Spatial) -> Vec<NodeKey>`.
}

#[cfg(feature = "dev-asp")]
pub mod verb_impls {
    //! Phase 1 placeholder — Plan 18 Task 3+4 fill in the verb dispatch
    //! table and the individual handler bodies.
}

#[cfg(feature = "dev-asp")]
pub mod transport {
    //! Phase 1 placeholder — Plan 18 Task 5 fills in the
    //! Unix-socket / Named-Pipe / WebSocket / stdio transports.
}

#[cfg(feature = "dev-asp")]
pub mod session {
    //! Phase 1 placeholder — Plan 18 Task 6 fills in token bootstrap +
    //! per-session permission tier (`Observe` / `Act` / `Full`).
}

#[cfg(feature = "dev-asp")]
pub mod server {
    //! Phase 1 placeholder — Plan 18 Task 7 fills in the main loop:
    //! transport accept → handshake → verb dispatch → audit ring buffer.
}

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
