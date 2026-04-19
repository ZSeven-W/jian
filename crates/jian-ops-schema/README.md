# jian-ops-schema

Canonical Rust types + JSON Schema + TypeScript bindings for the `.op` file format.

## Usage

```rust
use jian_ops_schema::load_str;

let src = std::fs::read_to_string("app.op")?;
let result = load_str(&src)?;
let doc = result.value;
for warning in result.warnings {
    eprintln!("load warning: {:?}", warning);
}
```

## Artifacts

Running `cargo run -p jian-ops-schema --bin export_schema` regenerates
`bindings/ops.schema.json` (JSON Schema Draft 2020-12).

Running `cargo run -p jian-ops-schema --features export-ts --bin export_ts`
regenerates `bindings/ops.ts` (TypeScript type declarations).

Both files are tracked in git and consumed by `openpencil` via submodule.

## Versioning

- v0.x format files (`"version": "0.8.0"` etc.) — full backward compat.
- v1.0 format files (`"formatVersion": "1.0"`) — Jian extensions supported.
- v2.0+ — rejected at load time.

## License

Apache-2.0
