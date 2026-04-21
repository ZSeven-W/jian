# Changelog

## [0.4.0] — Plan 5 — Gesture Arena

### Added

- Flutter-style gesture pipeline under `gesture/`:
  - `PointerEvent` with unified `PointerKind` / `PointerPhase` /
    `MouseButtons` / `Modifiers`.
  - `hit_test` over `SpatialIndex` returning a z-ordered `HitPath` that walks
    parent ancestors for bubbling.
  - `Recognizer` trait (`handle_pointer` / `accept` / `reject` / `tick`) and
    `RecognizerState` state machine (Possible / Eager / Defer / Claimed /
    Rejected).
  - Per-pointer `Arena` with priority-based arbitration on Up.
  - MVP recognizers: `TapRecognizer`, `DoubleTapRecognizer`,
    `LongPressRecognizer`, `PanRecognizer`, `HoverRecognizer`.
    (Scale/Rotate deferred to Plan 9 when host-desktop multi-pointer lands.)
  - `SemanticEvent` enum with `handler_key()` mapping to schema
    `events.*` names (camelCase: `onTap`, `onPanUpdate`, …).
  - `PointerRouter` top-level dispatcher: creates arenas on Down, routes
    Move/Up into the winning recognizer, separates Hover state-tracking, and
    exposes `tick(now)` for timer-driven recognizers (LongPress).
  - `rawPointer` escape hatch: any ancestor declaring
    `gestures.rawPointer: true` bypasses arena arbitration and receives
    `SemanticEvent::RawPointer` directly.
  - `FocusManager` MVP (request/clear) — full Tab-tree traversal lands with
    host-desktop in Plan 9.
  - `EventDispatcher` (`dispatch_event`) resolves the node's `events.<key>`
    ActionList and runs it through Plan 4's `execute_list_shared`.
- `Runtime` wiring:
  - New fields: `gestures: PointerRouter`, `actions: SharedRegistry`,
    `expr_cache`, and injected services (network/storage/nav/feedback/
    async_feedback/clipboard/capabilities) with Null defaults.
  - `dispatch_pointer(event)` and `tick(now)` drive the gesture pipeline and
    fire action handlers end-to-end.
- Integration tests (`tests/gesture_tap_counter.rs`): Tap increments
  `$app.count`; drag past slop rejects Tap; miss outside node fires nothing.

## [0.3.0] — Unreleased (Plan 4)

### Added

- Tier 2 Action DSL interpreter with async execution:
  - `ActionImpl` (`async_trait(?Send)`) + `ActionChain::run_serial` driver.
  - `ActionRegistry` + `SharedRegistry` (Rc<RefCell<...>>) for nested
    re-parse of control-flow action bodies.
  - `execute_list` facade powered by `futures::executor::block_on`.
- Action catalogue:
  - **State**: `set` (shorthand + target/value), `delete`, `reset`.
  - **Control flow**: `if` (then/else), `abort`, `delay` (MVP passthrough),
    `for_each` (`$item`/`$index` locals), `parallel`, `race`.
  - **Navigation**: `push`, `replace`, `pop`, `reset` (string→nav, scope→state),
    `open_url`.
  - **Network**: `fetch` with `loading` / `into` / `on_error` chain + explicit
    `Capability::Network` gate.
  - **Storage**: `storage_set`, `storage_clear`, `storage_wipe` with
    `Capability::Storage` gate.
  - **UI feedback**: `toast`, `alert`, `confirm` (async confirm branches on
    `on_confirm` / `on_cancel`).
  - **L4 platform stubs**: `vibrate`, `haptic`, `share`, `notify` (emit
    warnings until real adapters land).
  - **Tier 3**: `call` dispatches through `LogicProvider`; Null provider
    errors flow to `on_error`.
- Platform service traits + Null implementations in `services/`:
  `NetworkClient`, `StorageBackend`, `Router`, `FeedbackSink`,
  `AsyncFeedback`, `ClipboardService`, `WebSocketSession`.
- `CapabilityGate` trait with `DummyCapabilityGate` (allow-all) +
  `DeclaredCapabilityGate` (whitelist) implementations.
- `CancellationToken` honoured by every async action between awaits.
- `Expression::eval_with_locals` enables `for_each` / HOF-style lambdas
  to pass `$item` / `$index` / `$acc` overrides into sub-expressions.

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
