# Changelog

The crate has not been released yet — this log only contains
additions targeted at the workspace's `0.0.1` development release.

## [0.0.1] - Unreleased

### Added

- NDJSON-over-byte-stream protocol for AI agents driving a running
  Jian runtime. Two opt-in surfaces share the same envelope,
  transport, and session machinery; hosts pick one (or both, for
  tests) at compile time:
  - `dev-asp`: full debug / accessibility / UI testing verbs
    (`handshake`, `list_actions`, `find`, `inspect`, `snapshot`,
    `audit`, `tap`, `type`, `scroll`, `swipe`, `wait_for`,
    `assert`, `navigate`, `set_state`, `exit`).
  - `prod-asp`: token-efficient AI driving against shipping apps
    (`handshake`, `list_actions`, `tap`, `type`, `scroll`,
    `swipe`, `exit`). Rows are flat `{id, events}` pairs — ~3.65×
    smaller than MCP's `tools/list` on a 50-action screen
    (`cargo test -p jian-asp --features prod-asp byte_budget --
    --nocapture` prints the live byte budget).
- `--no-default-features` produces an empty crate (no serde, no
  module tree).
- Unix socket transport with atomic 0700 parent dir + 0600 socket
  + `UnixStream::connect` probe before unlink (narrowed to
  `ConnectionRefused | NotFound` so an active listener can't be
  unlinked as stale).
- Windows Named Pipe transport via `CreateNamedPipeW
  (PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
  PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT |
  PIPE_REJECT_REMOTE_CLIENTS, nMaxInstances=1)` with explicit
  user-SID DACL resolved at runtime via `OpenProcessToken` +
  `GetTokenInformation(TokenUser)` + `ConvertSidToStringSidW`.
  `read_unaligned` for `TOKEN_USER` cast alignment safety;
  `GetLastError` captured before `LocalFree`.
- `socket_path::resolve(arg) -> PathBuf` whitelist: `auto`,
  absolute path, `./` or `../` prefix (Unix), `\\.\pipe\`
  (Windows).
- `AspBridge` / `AspDrain` mpsc rendezvous (`sync_channel(0)`)
  between the listener thread and the runtime so accidental
  pipelining surfaces as deadlock not silent reorder.
- `FileTokenValidator { path, grant }` re-reads the file on every
  `validate()`; full `String::trim()` (not just `\n` / `\r`) so
  whitespace-only revocation reads as empty and refuses with
  `"token unavailable"`. Constant-time byte compare.
- `prod_op_guard::rewrite_op_verb_for_prod` rewrites selectors
  from action ids to source-node-ids; `is_action_id_only` uses
  exhaustive destructuring (no `..`) so future Selector fields
  force a compile error. `validate_prod_op_target` checks
  Available + !aiHidden, validates source_kind matches verb's
  expected event, resolves rewritten selector to verify exactly
  one node match (duplicate-id defense).
- `dispatch_with_mode` Mode::Prod path:
  `rewrite_op_verb_for_prod` then dispatches;
  `sanitize_prod_op_payload` overwrites `target` with action_id
  and replaces `narrative` with `"action <id> dispatched"` so
  the response surface doesn't leak schema node IDs or layout
  coordinates.
- `run_prod_session_via_bridge(transport, validator, bridge,
  _start)` mirrors `run_prod_session` but ferries verbs through
  the bridge.
- Five acceptance tests in `tests/prod_acceptance.rs` covering
  `list_actions` row shape, structural verb refusal +
  state-canary, dev `list_actions`, prod tap leak refusal
  (`target=action_id`, no `digit.digit` coords), per-op-verb
  leak.
- 50-action byte-budget benchmark asserts MCP/ASP-prod ≥ 3×
  (current 3.65×).
- GitHub Actions matrix CI: `asp-features × {dev-asp, prod-asp}
  × {linux, macos, windows}` runs on every push.
