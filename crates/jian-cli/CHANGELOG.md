# Changelog

The crate has not been released yet — this log only contains
additions targeted at the workspace's `0.0.1` development release.

## [0.0.1] - Unreleased

### Added

- `jian check PATH [--json]` — load + parse via `jian_ops_schema::
  load_str`, emit human or NDJSON diagnostics. Exit 0 clean, 1
  warnings, 2 parse error.
- `jian pack INPUT OUTPUT [--include-fonts] [--include-images]
  [--aot] [--aot-viewport WxH]` — deflate-compressed `.op.pack`
  containing `manifest.json` + `app.op` + assets. With `--aot` the
  archive carries both `aot/initial_layout.bin` and
  `aot/default_state.bin`, dumped from a single probe runtime so
  the layout↔state pair stays internally consistent. Manifest's
  `aot` block records `default_viewport`, `initial_layout`,
  `default_state`, and the `measurement_backend` tag (currently
  `"estimate"`) so a runtime preload reader can refuse a
  mismatched-shaping snapshot. Console summary prints both file
  sizes (`AOT layout 800×600 (N rect(s), M bytes), AOT state (K
  app key(s), J bytes)`).
- `jian unpack INPUT OUT_DIR` — extract every entry; zip-slip
  guard on entry names.
- `jian new NAME [--template counter|form] [--path DIR]` —
  scaffold a fresh project from embedded templates with
  `{{APP_NAME}}` / `{{APP_ID}}` substitution. `slugify` helper
  produces kebab-case `APP_ID`s.
- `jian player PATH [--size WxH] [--title ...]` — opens a `.op`
  in a real desktop window backed by `jian_host_desktop::
  DesktopHost::run` (winit 0.30 + softbuffer CPU presenter).
  Scene-walk driven by `jian_host_desktop::scene::collect_draws`.
  `--asp <path|auto>` opens an Agent Shell Protocol Unix socket /
  Windows Named Pipe with explicit user-SID DACL,
  `BCryptGenRandom` token, file-based revoke + rotate via
  `FileTokenValidator`; `--asp-permission <observe|act|full>`
  picks the permission tier.
- `jian dev PATH` — hot-reload variant of `player` with `notify`-
  driven file watch; `--mcp` flag bundles the
  `jian-action-surface` MCP server.
- `jian perf startup [--runs N] [--format json|table]` — drives
  `StartupDriver` end-to-end and reports per-phase timings.
- `jian perf compare baseline.json current.json --threshold 0.15
  --noise-floor-ms 1.0 --label macos-aarch64` — regression bot,
  single rolling PR comment with the diff table.
- Cargo features: `player` (default; opt out via
  `--no-default-features` for headless-only build), `mcp` (MCP
  surface), `prod-asp` (Plan 18 Agent Shell Protocol prod mode).
- AspBridge mpsc rendezvous bridge between the listener thread
  and the runtime; AspSession Drop calls `CancelSynchronousIo`
  on Windows via OS thread id reported through
  `sync_channel(1)`.
- Packaging configs: `cargo bundle` macOS `.app`, `cargo wix`
  Windows MSI, `cargo deb` + AppImage Linux, `.icns` generator,
  Sparkle appcast template, AppImageUpdate metadata.
  `osx_url_schemes` (cargo-bundle) intentionally OMITTED until
  the Plan 8 §T8 NSApplicationDelegate URL receiver lands across
  all three platforms.
- 7+ integration tests via `assert_cmd` covering check / pack /
  unpack / new / player CLI surfaces. Pack-specific tests assert
  presence / absence of both AOT binaries and decode-round-trip
  the empty fixture's state snapshot.
- `.op.pack` archive reader in the player path is **not yet**
  shipped — runtime AOT preload hooks are wired and tested but
  the player still loads raw `.op` only.
- `cargo dist`, Homebrew, winget distribution configs are **not
  yet** shipped.
