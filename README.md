# Jian (简)

A Rust-native cross-platform UI framework. An `.op` file is an app.

See `docs/` for the design spec (pending publication).

## Crates

- **`jian-ops-schema`** (`v0.1.0-schema`) — canonical schema + types for `.op` files.
  Byte-level roundtrip for v0.x legacy files; additive v1.0 extensions for Jian apps.
  Generates `bindings/ops.schema.json` (JSON Schema Draft 2020-12) and
  `bindings/ops.ts` (TypeScript type declarations) consumed by OpenPencil and other
  editors.

- **`jian-core`** (`v0.1.0-core`) — runtime kernel. Provides:
  - `Runtime` composition root — load `.op` → `RuntimeDocument` → layout → spatial index.
  - `Signal<T>` + `Effect` fine-grained reactivity (Leptos/SolidJS-style, single-threaded).
  - `StateGraph` with six scopes (`$app` / `$page` / `$self` / `$route` / `$storage` / `$vars`).
  - Flexbox layout via `taffy`, R-tree hit testing via `rstar`, viewport math.
  - `RenderBackend` trait (with a test-only `CaptureBackend`) and
    `LogicProvider` trait (Tier 3 WASM reserved for later stages).

## Quickstart

```rust
use jian_core::Runtime;

let src = std::fs::read_to_string("app.op")?;
let mut rt = Runtime::new();
rt.load_str(&src)?;
rt.build_layout((800.0, 600.0))?;
rt.rebuild_spatial();

let hits = rt.spatial.hit(jian_core::geometry::point(50.0, 50.0));
```

## Development

```bash
cargo test --all-features          # 238 tests across both crates
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo run -p jian-ops-schema --bin export_schema
cargo run -p jian-ops-schema --features export-ts --bin export_ts
```

## Roadmap

- Stage A (this release): schema + runtime skeleton — ✅ `v0.1.0-schema`, `v0.1.0-core`
- Plan 3: Tier 1 expression interpreter (bindings → signals)
- Plan 4: Tier 2 Action DSL (event handlers, effects)
- Plan 5: gesture arena (pointer → action dispatch)
- Plan 7: Skia render backend (`jian-skia`)
- Stage G: Tier 3 WASM logic modules

## License

MIT
