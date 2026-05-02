# jian-asp — Agent Shell Protocol

NDJSON-over-byte-stream protocol for AI agents driving a running Jian
runtime. Two opt-in surfaces share the same envelope, transport, and
session machinery; hosts pick one (or both, for tests) at compile
time:

| Feature   | Verbs                                                                                          | Use                                              |
|-----------|------------------------------------------------------------------------------------------------|--------------------------------------------------|
| `dev-asp` | `handshake`, `list_actions`, `find`, `inspect`, `snapshot`, `audit`, `tap`, `type`, `scroll`, `swipe`, `wait_for`, `assert`, `navigate`, `set_state`, `exit` | Debug / accessibility / UI testing               |
| `prod-asp`| `handshake`, `list_actions`, `tap`, `type`, `scroll`, `swipe`, `exit`                          | Token-efficient AI driving against shipping apps |

`prod-asp` is the production AI channel: ~4-8× cheaper per session
than MCP's `tools/list` because rows are flat `{id, events}` pairs
with no descriptions, schemas, hierarchy, or labels (`cargo test
-p jian-asp --features prod-asp byte_budget -- --nocapture` prints
the live byte budget on a 50-action screen).

`--no-default-features` produces an empty crate (no serde, no module
tree). The CI gate
[`ci-action-surface-isolation.yml`](../../.github/workflows/ci-action-surface-isolation.yml)
fails any PR whose release host's `cargo tree` pulls `jian-asp` via
`dev-asp`.

## Portable client behavior

Spec §7's migration plan is "always discover with `list_actions`
first" — agents should treat `list_actions` as the canonical
discovery API and reach for `find` / `inspect` / `snapshot` only on
**dev** sessions where they have explicit permission.

The portable flow is the same in dev and prod:

```text
1. handshake     → server confirms permission tier
2. list_actions  → flat [{id, events}] projection of every actionable element
3. tap / type / scroll / swipe { selector: { id: "<list_actions id>" } }
4. (loop 2-3 as the agent's plan unfolds)
5. exit          → clean session teardown
```

The action ids are the same in dev and prod
(`<scope>.<verb>_<slug>_<hash4>`, derived by
`jian-action-surface`). An agent can switch between MCP, ASP dev,
and ASP prod without re-learning ids.

In **prod** mode, the operation verbs (`tap` / `type` / `scroll` /
`swipe`) accept *only* `{"id": "<list_actions id>"}` selectors —
arbitrary structural selectors (`role`, `text`, `near`, …) are
refused with the `Invalid` error tag. In **dev** mode the same
verbs additionally accept the full structural selector vocabulary.

A misbehaving agent that calls a dev-only verb in prod gets
`OutcomePayload { error: Some("UnsupportedVerbInProd"), … }` and
the session stays open — the client can self-correct without a
re-handshake.

## Transport

`Transport` is a synchronous read-line / write-line trait. Four
implementations are planned; current state:

| Transport                | Status          | Path                                  |
|--------------------------|-----------------|---------------------------------------|
| `StdioTransport`         | Shipped         | dev CLI agent driving                 |
| `UnixSocketTransport`    | Shipped         | macOS / Linux prod ASP listener       |
| `NamedPipeListener`      | Stub on Windows | landing in a follow-up commit         |
| WebSocket                | Not implemented | future remote-control profile (separate threat model) |

`socket_path::resolve_bind_arg` rejects TCP / `host:port` / URL
shapes; prod ASP is local-only by spec §6.

## Spec / plan references

- Protocol design: `2026-04-17-jian-runtime-design.md` C18.
- Plan tasks: `2026-04-17-jian-plan-18-agent-shell-protocol.md`,
  `2026-05-01-jian-plan-18-asp-production-mode.md`.

## Acceptance gate (spec §10)

`tests/prod_acceptance.rs` pins these spec §10 bullets:

- **Bullet 1**: prod `list_actions` rows have *only* `id` and
  `events` (no leaked structural fields).
- **Bullet 2**: prod op response bodies don't carry `.op` tree
  structure — `target` is the action id (not the schema node id),
  and `narrative` doesn't include layout-rect coordinates.
- **Bullet 3**: prod rejects `find` / `inspect` / `snapshot` /
  `audit` / `set_state` (and the other dev-only verbs) with the
  stable `UnsupportedVerbInProd` error tag *and* never runs the
  handler's side-effects.
- **Bullet 9**: dev mode dispatches `list_actions` for portable
  clients.

The other §10 bullets are pinned at the layer they belong to:

- **Bullets 4 / 5**: prod-op selector narrowing + `aiHidden`
  filtering — `verb_impls/prod_op_guard.rs` + `verb_impls/list_actions.rs`.
- **Bullet 6**: prod refuses startup without app capabilities —
  `server.rs` (run_prod_session preconditions).
- **Bullet 7**: token validation is delegated to a host-installed
  `TokenValidator`. The crate cannot structurally prove the host
  isn't a no-op — that's a host contract documented at
  `session.rs`.
- **Bullet 8**: TCP / network bind refusal — `transport/socket_path.rs`
  + `crates/jian-cli/tests/cli_subcommands.rs`.
- **Bullet 10**: three-tier token comparison —
  `tests/byte_budget.rs` prints MCP vs ASP-dev vs ASP-prod and
  asserts the prod ratio ≥ 3× MCP on a 50-action screen
  (currently 3.65×).
