# jian-action-surface

The Jian runtime's **AI Action Surface** — the production-side MCP-compatible
protocol an external AI agent (Claude, GPT, local LLM, …) calls to drive
a running `.op` app. Spec:
[`2026-04-24-ai-action-surface.md`](../../../openpencil-docs/superpowers/specs/2026-04-24-ai-action-surface.md).

## Why this exists (and what it isn't)

Jian had two candidate paths for "let an AI agent drive a UI":

- **Agent Shell Protocol (`jian-asp`, Plan 18)** — generic UI verbs:
  `tap` / `find` / `inspect` / `snapshot`. Powerful but exposes node IDs
  and the raw scene tree, which is a structural information leak per
  threat model T1. After 2026-04-24 ASP is **dev-only** (`#[cfg(feature
  = "dev-asp")]`) and does not link into production hosts.
- **Action Surface (this crate)** — the **production** path. Exposes
  only **derived business actions** (e.g. `home.sign_in_a3f7`,
  `checkout.set_email_b012`, `home.swipe_left_b9d2`) projected from the
  author's events / bindings / routes. AI sees what a human user can do
  with the UI, **not the document structure**. No node IDs, no
  `$state.*` paths, no pixels.

The split is enforced physically by Cargo features + CI (Plan 22 Task 10).

## What's in the box

| Module                | Role                                                          |
|-----------------------|---------------------------------------------------------------|
| `lib.rs`              | `ActionSurface` — list / execute / dynamic gate, session state |
| `mcp/`                | `rmcp` MCP server (`tools/list` + `tools/call`) over stdio    |
| `transport.rs`        | NDJSON / JSON-RPC 2.0 fallback (sync, no tokio)               |
| `runtime_dispatch.rs` | Hands an executed action off to the live `Runtime`. Phase 1 wires Tap (`PointerEvent`), Confirm / Dismiss (Enter / Esc `KeyEvent`), SetValue (`state.app_set` direct write), OpenRoute (`runtime.nav.push/replace`). DoubleTap / LongPress / Submit / Swipe* / LoadMore land as `ExecutionFailed(handler_error)` until their host-driver paths exist. |
| `rate_limit.rs`       | Token bucket — 10 execute / sec / session                     |
| `concurrency.rs`      | `Busy(already_running)` for re-entrant calls on same action   |
| `swipe_throttle.rs`   | 400 ms same-direction window for `swipe_*` actions            |
| `audit.rs`            | `ActionAuditLog` ring buffer + `ActionSurfaceAuditEntry`      |
| `error.rs`            | 4-tier wire taxonomy (ValidationFailed / NotAvailable / Busy / ExecutionFailed) |

The actual derivation algorithm — the pure function that turns a
`PenDocument` into `Vec<ActionDefinition>` — lives next door in
[`jian_core::action_surface`](../jian-core/src/action_surface). It is
linked into every production runtime regardless of whether MCP is
attached.

## The two MCP tools

```
tools/list  → list_available_actions
tools/call  → execute_action
```

### `list_available_actions`

```jsonc
{
  "page_scope": "current",         // or "all"
  "include_confirm_gated": false   // dev-only, see spec §4.1
}
```

Returns:

```jsonc
{
  "actions": [
    {
      "name":           "home.sign_in_a3f7",
      "description":    "Sign in with email and password",
      "params_schema":  { "type": "object", "properties": {} },
      "returns_schema": { "type": "object", "properties": { "ok": { "type": "boolean" } } }
    }
  ],
  "page":  "home",
  "total": 12
}
```

Filtered to `Available` actions by default. Hidden / state-gated /
confirm-gated actions never appear in production.

### `execute_action`

```jsonc
{ "name": "home.set_email_b012", "params": { "value": "alice@example.com" } }
```

Success: `{ "ok": true }`.

Failure (always 4-tier, never leaks `$state.*` paths or handler names):

```jsonc
{ "ok": false, "error": { "kind": "NotAvailable", "reason": "state_gated" } }
```

| `kind`            | `reason` enum                                                 |
|-------------------|---------------------------------------------------------------|
| `ValidationFailed`| `missing_required` / `type_mismatch` / `out_of_range` / `schema_violation` |
| `NotAvailable`    | `unknown_action` / `static_hidden` / `state_gated` / `confirm_gated` / `rate_limited` |
| `Busy`            | `already_running`                                             |
| `ExecutionFailed` | `capability_denied` / `handler_error` / `timeout` / `unknown` |

## Author opt-in: `SemanticsMeta`

The `.op` schema's optional `SemanticsMeta` block tunes how each action
appears to the AI. Author writes nothing → defaults apply.

```jsonc
{
  "type": "rectangle",
  "id": "checkout-btn",
  "events": { "onTap": [ ... ] },
  "semantics": {
    "aiName":        "checkout",                   // stable cross-build name
    "aiDescription": "Submit the cart and pay",    // help string for the AI
    "aiHidden":      false,                        // hide from list_available_actions
    "aiAliases":     ["pay", "home.checkout_b9d2"] // honour old AI prompts
  }
}
```

| Field           | Effect                                                                      |
|-----------------|-----------------------------------------------------------------------------|
| `aiName`        | Replaces the auto-derived `<slug>_<hash4>`. Stable across builds.           |
| `aiDescription` | Human-readable hint shown in `list_available_actions` `description`.        |
| `aiHidden`      | `true` → permanently absent. `false` → unlocks `ConfirmGated` actions.      |
| `aiAliases`     | Old action names that still resolve via `execute_action` (audit `alias_used`). |

## Cargo features

| Feature         | Default | What it pulls in                                              |
|-----------------|---------|---------------------------------------------------------------|
| `agent-surface` | **on**  | The default production wire. `jian-host-*` link this.         |
| `mcp`           | off     | `rmcp` + `tokio` for the MCP stdio server (`mcp/` module).    |

`jian-cli`'s `dev` subcommand activates `mcp` via `--mcp`; production
builds typically leave `mcp` off and bridge through NDJSON / Unix
sockets, which is sync and avoids the tokio link.

## Embedding (host author)

```rust
use jian_action_surface::{ActionSurface, ActionAuditLog};
use std::rc::Rc;

let salt   = jian_core::action_surface::BUILD_SALT; // see jian-core/build.rs
let audit  = Rc::new(ActionAuditLog::new(10_000));
let surface = ActionSurface::from_document(&doc.schema, &salt)
    .with_audit(audit.clone());

// In-process API:
let actions = surface.list(jian_action_surface::ListOptions::default());
let outcome = surface.execute_with_gate(
    "home.checkout",
    Some(&json!({})),
    &mut dispatcher,
    &gate,
);
```

For an MCP stdio server (under `--features mcp`):

```rust
let (drain, handle) = jian_action_surface::mcp::spawn_stdio_server()?;
host.with_mcp(drain, salt);     // host pumps `drain` each frame in `about_to_wait`
// `handle` keeps the rmcp server alive; drop it to terminate.
```

## Determinism

`derive_actions(doc, build_salt)` is **bitwise stable** for the same
`(doc, build_salt)` pair. The salt is a compile-time constant produced
by [`jian-core/build.rs`](../jian-core/build.rs) (priority:
`JIAN_BUILD_SALT` env → `git HEAD` + Cargo semver → semver alone).
This means action names like `home.sign_in_a3f7` are stable across
restarts of the same build but predictably shift across builds — the
churn is by design, not a bug. Authors who want cross-build-stable
names set `semantics.aiName`, which drops the `_<hash4>` suffix.

## Threat-model anchors

| Concern (threat model 4.x)         | Mitigation                                            |
|-----------------------------------|-------------------------------------------------------|
| 4.1 structural leak                | No node IDs / scene tree exposed. Names are derived. |
| 4.2 state-path leak                | Errors are 4-tier enum, no `$state.*` strings.        |
| 4.3 destructive action by AI       | `confirm:` / `fetch DELETE|POST` / `storage_clear` → `ConfirmGated`. Author opts in via `aiHidden: false`. |
| 4.4 abusive call rate              | Token bucket 10/s/session.                            |
| 4.5 same-action stampede           | `Busy(already_running)` re-entrancy guard.            |
| 4.6 swipe spam                     | 400 ms same-direction window.                         |

## Tests

```bash
cargo test -p jian-action-surface                  # 63 unit tests
cargo test -p jian-action-surface --features mcp   # +2 mcp_smoke round-trips
```

The smoke test (`tests/mcp_smoke.rs`) drives a full
`initialize → tools/list → tools/call → error shapes` round-trip over
an in-process `tokio::io::duplex` pipe — same code path the real stdio
listener uses, no real stdin/stdout needed.

## Status

Phase 1 (this crate). Phase 2 ideas:

- `execute_chain` — author-whitelisted action sequences
- Author-declared return shapes beyond `{ok: bool}`
- Cross-session rate-limit aggregation
- `pen-ai-skills` `SkillBackend::ActionSurface` consumer (Plan 15)

## License

MIT
