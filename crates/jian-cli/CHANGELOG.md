# Changelog

All entries roll up into the workspace's `0.0.1` development release;
sections within tag the originating Plan for traceability.

## [0.0.1] - Unreleased

### Added

**Plan 9 — `jian` CLI MVP (8 subcommands):**

- `jian check PATH [--json]`: load + parse via `jian_ops_schema::
  load_str`, emit human or NDJSON diagnostics. Exit 0 clean, 1
  warnings, 2 parse error.
- `jian pack INPUT OUTPUT [--include-fonts] [--include-images]
  [--aot] [--aot-viewport WxH]`: deflate-compressed `.op.pack`
  containing `manifest.json` + `app.op` + assets + AOT entries.
- `jian unpack INPUT OUT_DIR`: extract every entry; zip-slip guard
  on entry names.
- `jian new NAME [--template counter|form] [--path DIR]`: scaffold
  a fresh project from embedded templates with `{{APP_NAME}}` /
  `{{APP_ID}}` substitution.
- `jian player PATH [--size WxH] [--title ...]`: opens a `.op` in a
  real desktop window backed by `jian_host_desktop::DesktopHost::
  run` (winit 0.30 + softbuffer CPU presenter). Scene-walk driven
  by `jian_host_desktop::scene::collect_draws`.
- `jian dev PATH`: hot-reload variant of `player` with `notify`-
  driven file watch; `--mcp` flag bundles the
  `jian-action-surface` MCP server.
- `jian perf startup [--runs N] [--format json|table]`: drives
  `StartupDriver` end-to-end and reports per-phase timings.
- `jian perf compare baseline.json current.json --threshold 0.15
  --noise-floor-ms 1.0 --label macos-aarch64`: regression bot,
  single rolling PR comment with the diff table.
- `slugify` helper: kebab-case `APP_ID` generation.
- `player` cargo feature (on by default). `--no-default-features`
  produces a headless-only build; `mcp` opts into MCP surface;
  `prod-asp` opts into the Plan 18 Agent Shell Protocol prod mode.
- 7+ integration tests via `assert_cmd` covering check / pack /
  unpack / new / player CLI surfaces.

**Plan 19 D1 cold-start AOT (default_state.bin):**

- `jian pack --aot` now emits **two** AOT entries instead of one:
  `aot/initial_layout.bin` plus the new `aot/default_state.bin`
  (six-scope StateGraph initial values via
  `DefaultStateSnapshot::write_bytes`). Both come from the same
  probe runtime so the layout↔state pair is internally consistent.
- The manifest's `aot` block gains a `default_state` field pointing
  at the new path.
- `compute_initial_layout` renamed to `compute_aot_payload`,
  returning `(InitialLayoutSnapshot, DefaultStateSnapshot)` so the
  pack writer walks the schema once.
- Console summary expanded: `AOT layout 800×600 (N rect(s), M
  bytes), AOT state (K app key(s), J bytes)`.
- 2 cli_subcommands integration tests extended to assert presence /
  absence of the new binary and decode-round-trip the empty
  fixture's state snapshot.

**Plan 18 Agent Shell Protocol prod mode (`prod-asp` feature):**

- `jian player --asp <path|auto>` opens a Unix socket / Windows
  Named Pipe with explicit user-SID DACL, `BCryptGenRandom` token
  generation, file-based revoke + rotate via `FileTokenValidator`.
- `--asp-permission <observe|act|full>` permission tier flag.
- AspBridge mpsc rendezvous bridge between the listener thread and
  the runtime; AspSession Drop calls `CancelSynchronousIo` on
  Windows via OS thread id reported through `sync_channel(1)`.

**Plan 8 packaging configs:**

- `cargo bundle` macOS `.app`, `cargo wix` Windows MSI, `cargo deb`
  + AppImage Linux, `.icns` generator, Sparkle appcast template,
  AppImageUpdate metadata.
- `osx_url_schemes` (cargo-bundle) intentionally OMITTED until the
  Plan 8 §T8 NSApplicationDelegate URL receiver lands.

### Deferred

- `.op.pack` archive reader in the player path so a published pack
  actually drives `install_data_path_with_aot` end-to-end (today
  the runtime hooks are wired and tested but the player still
  loads raw `.op` only).
- `cargo dist`, Homebrew, winget distribution configs.
