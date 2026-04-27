//! Compile-time `BUILD_SALT` constant for `derive_actions`.
//!
//! Spec: `2026-04-24-ai-action-surface.md` §3.1 — the salt fed to
//! `short_hash` must be a build-time constant so derivation is
//! bitwise-stable within one build. The actual bytes are produced by
//! `crates/jian-core/build.rs` from one of (in priority order):
//!
//!   1. `JIAN_BUILD_SALT` env var — explicit override.
//!   2. `git rev-parse HEAD` + Cargo semver — default.
//!   3. Cargo semver alone — fallback.
//!
//! Hosts must use this constant when constructing `ActionSurface` so
//! the same source `.op` produces the same action names across binary
//! invocations of the same build (and predictable churn across builds).
//! The placeholder `[0u8; 16]` previously used in tests / examples is
//! a footgun — dropping a real binary that uses `[0u8; 16]` collides
//! with every other `[0u8; 16]`-keyed surface.

include!(concat!(env!("OUT_DIR"), "/build_salt.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_salt_is_sixteen_bytes() {
        // Trivial but pins the include macro: a botched `build.rs` that
        // generates the wrong `pub const` shape fails to compile rather
        // than silently shipping a degenerate salt.
        let bytes: &[u8; 16] = &BUILD_SALT;
        assert_eq!(bytes.len(), 16);
    }

    #[test]
    fn build_salt_is_not_all_zero() {
        // The whole point of Task 0 was to stop hosts from shipping
        // `[0u8; 16]` placeholders. If this trips, build.rs failed to
        // resolve any source string — investigate before shipping.
        assert!(
            BUILD_SALT.iter().any(|b| *b != 0),
            "BUILD_SALT is all zero — build.rs source resolution broke"
        );
    }

    #[test]
    fn build_salt_source_is_recorded() {
        // Audit trail: the source string lives in the binary so a Codex
        // / forensic reader can see what fed the salt without a rebuild.
        assert!(!BUILD_SALT_SOURCE.is_empty());
    }

    #[test]
    fn build_salt_is_stable_within_one_build() {
        // Plan Task 0 Step 4. Trivial — `BUILD_SALT` is a `const` so
        // two reads can't differ — but pinning it as a test means a
        // botched `build.rs` rewrite that promotes the constant to a
        // `static` (or, worse, a `fn`) trips here rather than at the
        // first place that compares two derivations against each other.
        let a = BUILD_SALT;
        let b = BUILD_SALT;
        assert_eq!(a, b);
    }
}
