# jian-core

Core of the Jian UI framework: Document runtime, fine-grained Signals, Scene
Graph, Layout (via taffy), Spatial Index (via rstar), Viewport, and the
`RenderBackend` / `LogicProvider` extension traits.

## Usage

```rust
use jian_core::Runtime;

let src = std::fs::read_to_string("app.op")?;
let mut rt = Runtime::new();
rt.load_str(&src)?;
rt.build_layout((800.0, 600.0))?;
rt.rebuild_spatial();

// Hit-test:
let hits = rt.spatial.hit(jian_core::geometry::point(50.0, 50.0));
```

## Status

- Stage A skeleton (this crate)
- Tier 1 expressions: Plan 3 (next crate-level plan)
- Tier 2 Action DSL: Plan 4
- Gesture Arena: Plan 5

## License

MIT
