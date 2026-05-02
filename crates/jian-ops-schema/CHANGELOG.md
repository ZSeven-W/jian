# Changelog

All entries roll up into the workspace's `0.0.1` development release;
sections within tag the originating Plan for traceability.

## [0.0.1] - Unreleased

### Added

**Plan 19 D1 cold-start AOT (`aot/default_state.bin`):**

- `pack::default_state::DefaultStateSnapshot` — six-scope `BTreeMap`-
  backed capture of `app` / `page` / `self_node` / `route` / `storage`
  / `vars` initial values.
- Wire format (`OPS1` magic + `u16` version + `u32` payload_len +
  canonicalised JSON payload), `write_bytes` / `read_bytes`, and
  structural rejections (TooShort, BadMagic, UnsupportedVersion,
  PayloadTruncated, TrailingBytes, InvalidJson, DepthExceeded).
- `MAX_CANONICALIZE_DEPTH = 64` recursive object-key sort so the wire
  bytes stay deterministic under the workspace's `serde_json`
  `preserve_order` feature (`Value::Object` is `IndexMap`); scope-
  uniform depth counter prevents pathological programmatic snapshots
  from blowing the writer's stack ahead of serde-json's own 128-level
  guard.
- 15 unit tests covering round-trip, deterministic byte order (flat +
  nested + array-of-objects), all rejection variants, depth-limit
  boundary across scopes.
- Codex-reviewed across 4 rounds; final pass clean.

**Plan 19 D1 cold-start AOT (`aot/initial_layout.bin`):**

- `pack::initial_layout::InitialLayoutSnapshot` — `BTreeMap<String,
  PackedRect>` keyed by node id, with the document's authored
  `DefaultViewport`. `OPL1` little-endian SoA wire format: 18-byte
  header + per-rect `(u16 id_len, utf8 id, [f32; 4] xywh)`.
- `write_bytes` / `read_bytes` reject NonFiniteRect / InvalidViewport
  / TooShort / BadMagic / UnsupportedVersion / Truncated /
  InvalidIdUtf8 / IdTooLong / TrailingBytes / DuplicateId so a
  garbled snapshot falls back to a real `ComputeFirstLayout` rather
  than misparse.
- 30+ unit tests pinning the wire constants and every rejection path.

**Plan 19 D2 — font subsetter:**

- `font_plan::FontPlan` — per-family codepoint scanner returned by
  `BootstrapHandles::take_core_font_plan`. Hosts iterate
  `plan.families()` to request first-paint subsets.
- `pack::manifest` types (`AotManifest`, `AotInventory`,
  `DefaultViewport`, `FontEntry`, `ManifestAppMetadata`) — typed
  representation of a `.op.pack` `manifest.json`. `entries` always
  lists `app.op` plus every non-manifest entry.

**Plan 1 baseline:**

- Full Rust representation of v0.x `.op` file format
  (PenDocument/PenNode/styles/variables/pages).
- Jian v1 extension types: AppConfig, RoutesConfig, StateSchema,
  EventHandlers, Bindings, GestureOverrides, NavigationRoute,
  Lifecycle hooks (app/page/node), SemanticsMeta, LogicModuleRef.
- `load_str` compat loader with warnings for unknown fields and
  skipped logic modules.
- JSON Schema Draft 2020-12 export via `schemars`.
- TypeScript type export via `ts-rs` (feature-gated `export-ts`).
- Backward compat test suite (v0.x real corpus roundtrip).
- Forward compat test suite (future-field tolerance + v2 rejection).
- Schema drift test guarding `bindings/ops.schema.json` freshness.
- Real-world fixture `pencil-demo.op` (629 KB) validates non-trivial
  documents; surfaced a per-side object form of stroke thickness and
  optional `d` on path nodes.

### Deferred

- `aot/expressions.bin` precompiled bytecode — needs `jian_core::
  expression::Chunk: Serialize` refactor first.
