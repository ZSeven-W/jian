# jian-core

Core of the Jian UI framework: Document runtime, fine-grained Signals, Scene
Graph, Layout (via taffy), Spatial Index (via rstar), Viewport, Tier 1
expression language, Tier 2 Action DSL, **Gesture Arena**, and the
`RenderBackend` / `LogicProvider` extension traits.

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

## Gesture Arena

`Runtime` owns a `PointerRouter` that turns low-level `PointerEvent`s into
high-level `SemanticEvent`s (`Tap`, `Pan*`, `LongPress`, `Hover*`, …) and
fires the node's matching `events.*` ActionList through the Tier 2
interpreter.

```rust
use jian_core::{Runtime, gesture::{PointerEvent, PointerPhase}};

let mut rt = Runtime::new();
rt.load_str(&src)?;
rt.build_layout((800.0, 600.0))?;
rt.rebuild_spatial();

rt.dispatch_pointer(PointerEvent::simple(
    1, PointerPhase::Down, jian_core::geometry::point(100.0, 50.0)));
rt.dispatch_pointer(PointerEvent::simple(
    1, PointerPhase::Up,   jian_core::geometry::point(100.0, 50.0)));
// onTap fired; $app.count bumped.

rt.tick(std::time::Instant::now()); // drive LongPress timers each frame
```

Arena arbitration follows Flutter's convention: recognizers start `Possible`;
`Pan` claims on drag past its slop; `LongPress` claims on a held timer;
`Tap` only claims on the Up event if no other recognizer has moved first.
A node (or any ancestor on the hit path) can opt out with
`"gestures": { "rawPointer": true }` — its subtree then receives
`SemanticEvent::RawPointer` directly with no arena arbitration.

Scale / Rotate multi-pointer recognizers are deferred to Plan 9.

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
- `v0.3.0-core` — Tier 2 Action DSL
- **`v0.4.0-core`** — Gesture Arena (this release)
- Next: Skia render backend (Plan 7), host-desktop multi-pointer (Plan 9)

## License

MIT
