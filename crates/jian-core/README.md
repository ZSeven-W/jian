# jian-core

Core of the Jian UI framework: Document runtime, fine-grained Signals, Scene
Graph, Layout (via taffy), Spatial Index (via rstar), Viewport, Tier 1
expression language, and the `RenderBackend` / `LogicProvider` extension
traits.

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

## Tier 1 Expression Language

Compile a source expression into bytecode and evaluate it against a
`StateGraph`. Fine-grained subscriptions happen automatically: when the
Expression reads `$app.count`, only that Signal is subscribed.

```rust
use jian_core::expression::Expression;

let expr = Expression::compile("`Count: ${$app.count * 2}`")?;
let (v, warnings) = expr.eval(&state_graph, None, None);
```

Supported:

- Literals: number, string, bool, null, array `[...]`, object `{k:v}`.
- Operators: `+ - * / %`, `== != === !== < > <= >=`, `&& || ??`, `? :`,
  unary `! + -`, postfix `.` / `[...]` / call.
- Scope refs: `$app`, `$page`, `$self`, `$route`, `$storage`, `$vars`,
  `$state` (contextual), `$item`/`$index`/`$acc` (in HOFs), `$event`.
- Template strings: `` `text ${expr} more` ``.
- Builtin catalog: math / string / array (incl. HOF filter/map/sort/reduce
  with string-body lambdas) / object / date / type ops.
- Soft failure: type errors, division by zero, unknown vars, unknown
  functions return `null` + push a `Diagnostic` to warnings; evaluation
  never panics.

### Binding

`BindingEffect` attaches an Expression to a mutation callback:

```rust
use jian_core::{BindingEffect, expression::Expression};

let _b = BindingEffect::new(
    &rt.effects, expr, rt.state.clone(), None, None,
    |v, _warnings| { /* write into SceneNode property */ },
);
```

Changes to any referenced Signal trigger re-evaluation on the next
`Scheduler::flush()`.

## Status

- Stage A (this release): schema + runtime skeleton + Tier 1 expressions — ✅
  `v0.1.0-core`, `v0.2.0-core`
- Tier 2 Action DSL: Plan 4
- Gesture Arena: Plan 5
- Skia render backend: Plan 7

## License

MIT
