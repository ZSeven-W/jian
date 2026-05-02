# Changelog

The crate has not been released yet — this log only contains
additions targeted at the workspace's `0.0.1` development release.

## [0.0.1] - Unreleased

### Added

- Full Rust representation of v0.x `.op` file format
  (PenDocument/PenNode/styles/variables/pages).
- Jian v1 extension types: AppConfig, RoutesConfig, StateSchema,
  EventHandlers, Bindings, GestureOverrides, NavigationRoute,
  Lifecycle hooks (app/page/node), SemanticsMeta, LogicModuleRef.
- `load_str` compat loader with warnings for unknown fields and
  skipped logic modules.
- JSON Schema Draft 2020-12 export via `schemars`; TypeScript type
  export via `ts-rs` (feature-gated `export-ts`).
- Backward-compat test suite (v0.x real corpus roundtrip), forward-
  compat suite (future-field tolerance + v2 rejection), schema-drift
  test guarding `bindings/ops.schema.json` freshness, real-world
  fixture `pencil-demo.op` (629 KB).
- `pack::manifest` types: `AotManifest`, `AotInventory`,
  `DefaultViewport`, `FontEntry`, `ManifestAppMetadata`. `entries`
  always lists `app.op` plus every non-manifest entry.
- `font_plan::FontPlan` per-family codepoint scanner returned by
  `BootstrapHandles::take_core_font_plan`; hosts iterate
  `plan.families()` to request first-paint subsets.
- `pack::initial_layout::InitialLayoutSnapshot` — `BTreeMap<String,
  PackedRect>` keyed by node id with the document's authored
  `DefaultViewport`. `OPL1` little-endian SoA wire format (18-byte
  header + per-rect `(u16 id_len, utf8 id, [f32; 4] xywh)`).
  `write_bytes` / `read_bytes` reject NonFiniteRect / InvalidViewport
  / TooShort / BadMagic / UnsupportedVersion / Truncated /
  InvalidIdUtf8 / IdTooLong / TrailingBytes / DuplicateId so a
  garbled snapshot falls back to a real `ComputeFirstLayout` rather
  than misparse. 30+ unit tests pin every rejection path.
- `pack::default_state::DefaultStateSnapshot` — six-scope `BTreeMap`-
  backed capture of `app` / `page` / `self_node` / `route` /
  `storage` / `vars` initial values. `OPS1` framed wire format
  (`OPS1` magic + `u16` version + `u32` payload_len + canonicalised
  JSON payload). Recursive object-key sort with `MAX_CANONICALIZE_
  DEPTH = 64` so the wire bytes stay deterministic under the
  workspace's `serde_json` `preserve_order` feature; scope-uniform
  depth counter prevents pathological programmatic snapshots from
  blowing the writer's stack ahead of serde-json's own 128-level
  guard. Typed rejections for TooShort / BadMagic / Unsupported
  Version / PayloadTruncated / TrailingBytes / InvalidJson /
  DepthExceeded. 15 unit tests covering round-trip, deterministic
  byte order (flat / nested / array-of-objects), every rejection
  variant, and the depth-limit boundary across scopes.
- `aot/expressions.bin` precompiled bytecode is **not yet** shipped
  — needs `jian_core::expression::Chunk: Serialize` refactor first.
