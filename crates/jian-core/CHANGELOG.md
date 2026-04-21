# Changelog

## [0.2.0] — Unreleased (Plan 3)

### Added

- Tier 1 expression language:
  - Lexer, recursive-descent parser, AST, bytecode, stack-machine VM.
  - Scope references (`$app / $page / $self / $route / $storage / $vars`,
    contextual `$state`, local `$item / $index / $acc`).
  - Template literals (`` `text ${expr}` ``).
  - Builtins: math (10), string (11), array + HOF (filter/map/sort/reduce),
    object (4), date (3 MVP), type ops (5).
  - `Expression` facade + `ExpressionCache`.
  - `BindingEffect` for reactive scene-property updates.
  - Fine-grained Signal subscription: static member chains fold into a single
    `PushScopeRef` so only the referenced variable's Signal is subscribed.
  - Proptest fuzz (512 cases) + criterion `expr_eval` benches.

### Changed

- `Runtime::state` is now `Rc<StateGraph>` (was `StateGraph` by value) so
  bindings can capture shared state into effect closures.

## [0.1.0] — Unreleased

### Added

- Runtime composition root (`Runtime`).
- Document runtime (SlotMap-backed tree + ID index).
- Fine-grained reactive primitives: `Signal<T>`, `Scheduler`, `Effect`.
- State graph with six scopes: `$app`, `$page`, `$self`, `$route`, `$storage`, `$vars`.
- Layout engine via `taffy` 0.5 (basic flexbox mapping).
- Spatial index via `rstar` (hit + rect queries).
- Viewport math with screen↔scene transforms.
- `RenderBackend` trait + `CaptureBackend` for dry-run / tests.
- `LogicProvider` trait (Tier 3, L4 reserved).
- End-to-end pipeline smoke test (`counter.op` fixture).
- Signal update microbenchmark (10/100/1000 subscribers).
