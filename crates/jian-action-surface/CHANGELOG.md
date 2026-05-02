# Changelog

All entries roll up into the workspace's `0.0.1` development release;
sections within tag the originating Plan for traceability.

## [0.0.1] - Unreleased

### Added

**Plan 22 — AI Action Surface (production AI channel):**

- `ActionSurface` projection that derives **business actions** from a
  loaded document's events / bindings / routes (e.g.
  `home.sign_in_a3f7`, `checkout.set_email_b012`,
  `home.swipe_left_b9d2`) rather than exposing node IDs / scene tree.
- Threat-model T1 enforcement: AI sees what a human user can do with
  the UI, **not** the document structure — no node IDs, no
  `$state.*` paths, no pixel coordinates.
- Action availability gate: dispatching against a stale snapshot
  whose target action no longer exists fails safely with a typed
  refusal rather than mis-applying.
- Build-salt (`BUILD_SALT`, injected by `jian-core/build.rs`) seeds
  the deterministic action ID so an action ID generated against
  binary `A` cannot be re-used against binary `B`. Documented in
  `crates/jian-action-surface/README.md` so embedders know to
  regenerate actions on update.
- `mcp` feature: stdio MCP server via `rmcp`; pairs with `jian dev
  --mcp` for editor tooling.

**Plan 22 Phase 2 isolation:**

- Cargo-feature + CI gate
  (`.github/workflows/ci-action-surface-isolation.yml`) fails any PR
  whose release host's `cargo tree` pulls `jian-asp` via `dev-asp`.
  Production builds that ship `jian-action-surface` MUST NOT link
  Agent Shell Protocol's structural verbs.

### Documented

- Embedding guide + threat-model anchors in `README.md`.
- Client-side guide at
  `openpencil-docs/superpowers/notes/2026-04-24-ai-action-surface-client-guide.md`
  (Claude Desktop / raw-stdio Python clients, error-handling policy,
  build-salt awareness).
