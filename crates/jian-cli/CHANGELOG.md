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
  archive carries `aot/initial_layout.bin`, `aot/default_state.bin`,
  AND `aot/expressions.bin`, dumped from a single probe runtime so
  the layout ↔ state ↔ expression-cache trio stays internally
  consistent. Manifest's `aot` block records `default_viewport`,
  `initial_layout`, `default_state`, `expressions`, and the
  `measurement_backend` tag (currently `"estimate"`) so a runtime
  preload reader can refuse a mismatched-shaping snapshot. The
  probe runtime calls `Runtime::warm_expression_cache` after
  `build_layout` so every queued binding source compiles into the
  cache before the dump, not just whatever first-frame layout
  incidentally fired. Console summary prints all three file sizes
  (`AOT layout 800×600 (N rect(s), M bytes), AOT state (K app
  key(s), J bytes), AOT exprs (E cached, F bytes)`).
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
- `pack_reader` module — opens a `.op.pack` zip, validates the
  typed `AotManifest` (`format == "op.pack"` + version match +
  `entries` includes `app.op`), parses `app.op` into `PenDocument`,
  and decodes all three AOT entries (`initial_layout`,
  `default_state`, `expressions`) with their own `read_bytes`
  validators. Garbled AOT entries warn to stderr and drop to `None`
  so the runtime falls back to `ComputeFirstLayout` /
  `SeedStateGraph` / JIT compile rather than misparse. Defensive
  guards: per-entry decompressed byte ceilings (manifest 1 MiB,
  app.op 32 MiB, layout/state 8 MiB, expressions 16 MiB) via
  `entry.take(limit + 1)` to refuse decompression bombs;
  `READER_EXPECTED_BACKEND = "estimate"` requires the manifest's
  `aot.measurement_backend` to match before the layout snapshot
  drives preload (mismatched-shaper rects would diverge from the
  live render); orphan AOT entries inventoried only in the zip but
  not in `manifest.entries` are silently ignored; duplicate
  canonical names rejected. `looks_like_op_pack` extension hint
  routes the `jian player` entry between raw-`.op` and pack-archive
  load paths.
- `jian player path/to/foo.op.pack` — pack archive load path. Reads
  the schema + all three AOT entries, threads the initial-layout
  snapshot AND the expressions snapshot through
  `install_data_path_with_aot_full` (which gates the
  `ComputeFirstLayout` short-circuit on viewport bit-match + total
  coverage in the bootstrap layer, AND runs `verify_all` on the
  expressions snapshot before installing — verify failure drops the
  whole snapshot to JIT compile), then overlays the default-state
  snapshot via `restore_default_state`. The state overlay is gated
  by `snapshot_extra_keys` against a fresh `dump_default_state()`
  baseline — recursively type-compatible at every leaf
  (Null/Bool/Number/String outer match; Array length + element-
  wise; Object exact key parity + recursive value match) and no
  extra keys vs the schema-fresh seed. Mismatch → warn + skip the
  whole restore, keeping the schema-default seed intact.
- 20 `pack_reader` unit tests covering: extension routing
  (case-insensitive, rejects `.op.pack.bak`); zip-no-manifest
  rejection; wrong-format / unsupported-version / missing-app-op
  manifest rejection; AOT round-trip happy path; uninventoried-AOT
  silent-drop (per-entry, including expressions); backend-mismatch
  layout drop; garbled-AOT-snapshot fall-through (layout AND
  expressions); expressions round-trip; snapshot extras (basic +
  nested type drift + nested baseline-extra-key + array length
  mismatch + subset).
- `cargo dist`, Homebrew, winget distribution configs are **not
  yet** shipped.
