# Changelog

## [0.1.0] — Unreleased

### Added
- Full Rust representation of v0.x `.op` file format (PenDocument/PenNode/styles/variables/pages).
- Jian v1 extension types: AppConfig, RoutesConfig, StateSchema, EventHandlers, Bindings,
  GestureOverrides, NavigationRoute, Lifecycle hooks (app/page/node), SemanticsMeta, LogicModuleRef.
- `load_str` compat loader with warnings for unknown fields and skipped logic modules.
- JSON Schema Draft 2020-12 export via `schemars`.
- TypeScript type export via `ts-rs` (feature-gated `export-ts`).
- Backward compat test suite (v0.x real corpus roundtrip).
- Forward compat test suite (future-field tolerance + v2 rejection).
- Schema drift test guarding `bindings/ops.schema.json` freshness.
- Real-world fixture `pencil-demo.op` (629 KB) validates non-trivial documents;
  surfaced a per-side object form of stroke thickness and optional `d` on path nodes.
