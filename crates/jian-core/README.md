# jian-core

Core of the Jian UI framework: Document runtime, fine-grained Signals, Scene
Graph, Layout (via taffy), Spatial Index (via rstar), Viewport, Tier 1
expression language, **Tier 2 Action DSL**, and the `RenderBackend` /
`LogicProvider` extension traits.

## Usage

```rust
use jian_core::Runtime;

let src = std::fs::read_to_string("app.op")?;
let mut rt = Runtime::new();
rt.load_str(&src)?;
rt.build_layout((800.0, 600.0))?;
rt.rebuild_spatial();
```

## Tier 1 Expression Language

Compile a source expression into bytecode and evaluate it against a
`StateGraph`. Fine-grained subscriptions happen automatically.

```rust
use jian_core::expression::Expression;

let expr = Expression::compile("`Count: ${$app.count * 2}`")?;
let (v, warnings) = expr.eval(&state_graph, None, None);
```

## Tier 2 Action DSL

Parse and execute event-driven side-effect chains. Every action is a
single-key JSON object; `execute_list` drives them through an async
pipeline over `futures::executor::block_on`.

```rust
use jian_core::action::{default_registry, execute_list_shared, ActionContext};
use serde_json::json;

let reg = default_registry();
let list = json!([
    { "set": { "$app.count": "$app.count + 1" } },
    { "if": {
        "expr": "$app.count >= $app.target",
        "then": [
            { "toast": "\"Done!\"" },
            { "push": "\"/stats\"" }
        ]
    }}
]);
let out = execute_list_shared(&reg, &list, &ctx);
```

Registered actions:

- **State**: `set` (shorthand + target/value), `delete`, `reset` (scope or nav)
- **Control flow**: `if`, `abort`, `delay`, `for_each`, `parallel`, `race`
- **Navigation**: `push`, `replace`, `pop`, `reset` (router), `open_url`
- **Network**: `fetch` with `loading` / `into` / `on_error` + `Capability::Network`
- **Storage**: `storage_set`, `storage_clear`, `storage_wipe` + `Capability::Storage`
- **UI feedback**: `toast`, `alert`, `confirm` (async branches on user choice)
- **Platform stubs (L4)**: `vibrate`, `haptic`, `share`, `notify`
- **Tier 3**: `call` dispatches through `LogicProvider`

Platform service traits (`NetworkClient` / `StorageBackend` / `Router` /
`FeedbackSink` / `AsyncFeedback` / `ClipboardService`) have `Null*`
implementations in `services::null_impls`; host adapters supply real
backends.

### Binding

`BindingEffect` attaches an Expression to a mutation callback:

```rust
use jian_core::{BindingEffect, expression::Expression};

let _b = BindingEffect::new(
    &rt.effects, expr, rt.state.clone(), None, None,
    |v, _warnings| { /* write into SceneNode property */ },
);
```

## Status

- `v0.1.0-core` — runtime kernel skeleton
- `v0.2.0-core` — Tier 1 expressions + bindings
- **`v0.3.0-core`** — Tier 2 Action DSL (this release)
- Next: Gesture Arena (Plan 5), Skia render backend (Plan 7)

## License

MIT
