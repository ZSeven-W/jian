# Changelog

The crate has not been released yet — this log only contains
additions targeted at the workspace's `0.0.1` development release.

## [0.0.1] - Unreleased

### Added

- Runtime composition root (`Runtime`) tying together state,
  document tree, layout, spatial index, viewport, scene graph,
  gesture pipeline, action registry, expression cache, and the
  injected service traits.
- Document runtime (SlotMap-backed tree + ID index) and a
  `RuntimeDocument` produced by `loader::build` /
  `loader::build_with`.
- Fine-grained reactive primitives: `Signal<T>`, `Scheduler`,
  `Effect`. Static member chains fold into a single
  `PushScopeRef` so only the referenced variable's Signal is
  subscribed.
- State graph with six scopes (`$app`, `$page`, `$self`, `$route`,
  `$storage`, `$vars`) and matching slot-reusing setters
  (`app_set`, `page_set`, `self_set`, `route_set`, `storage_set`,
  `vars_set`); `restore_default_state` and `dump_default_state`
  for AOT round-trip.
- Tier 1 expression language: lexer, recursive-descent parser, AST,
  bytecode, stack-machine VM. Scope refs, template literals,
  builtins (math 10, string 11, array+HOF, object 4, date 3 MVP,
  type ops 5), `Expression` facade, `ExpressionCache`,
  `BindingEffect`, `eval_with_locals` for `$item` / `$index` /
  `$acc`. Proptest fuzz (512 cases) + criterion `expr_eval` benches.
- Tier 2 Action DSL interpreter with async execution: `ActionImpl`
  (`async_trait(?Send)`) + `ActionChain::run_serial`,
  `ActionRegistry` + `SharedRegistry`, `execute_list` powered by
  `futures::executor::block_on`. Action catalogue spans state
  (`set` / `delete` / `reset`), control flow (`if` / `abort` /
  `delay` / `for_each` / `parallel` / `race`), navigation (`push`
  / `replace` / `pop` / `reset` / `open_url`), network (`fetch`
  with `loading` / `into` / `on_error` chain), storage
  (`storage_set` / `storage_clear` / `storage_wipe`), UI feedback
  (`toast` / `alert` / `confirm`), L4 platform stubs (`vibrate` /
  `haptic` / `share` / `notify`), Tier 3 (`call` via
  `LogicProvider`).
- Platform service traits + Null implementations in `services/`:
  `NetworkClient`, `StorageBackend`, `Router`, `FeedbackSink`,
  `AsyncFeedback`, `ClipboardService`, `WebSocketSession`.
- `CancellationToken` honoured by every async action between
  awaits.
- Flutter-style gesture pipeline under `gesture/`: `PointerEvent`
  with unified `PointerKind` / `PointerPhase` / `MouseButtons` /
  `Modifiers`; `hit_test` over `SpatialIndex` returning a z-ordered
  `HitPath`; `Recognizer` trait + `RecognizerState` state machine;
  per-pointer `Arena` with priority-based arbitration on Up;
  recognizers Tap / DoubleTap / LongPress / Pan / Hover; multi-
  pointer Pinch + Rotate; `SemanticEvent` enum mapping to
  `events.<key>` (camelCase); `PointerRouter` top-level
  dispatcher; `tick(now)` for timer-based recognizers;
  `rawPointer` escape hatch; `FocusManager` MVP; `EventDispatcher`
  resolves and runs `events.<key>` through `execute_list_shared`.
- Top-level `crate::capability` module: `CapabilityGate::check
  (needed, action)`, `DeclaredCapabilityGate::new(caps,
  Some(audit))` writing `AuditEntry` per check, `AuditLog` ring
  buffer + accessors, `map::required_capabilities(action_name)`,
  `AutomationLevel { Observe, Act, Full }` lattice,
  `PermissionBroker` trait + `NullPermissionBroker` stub.
  `Runtime::new_from_document(schema)` reads `app.capabilities`,
  builds a `DeclaredCapabilityGate` with a 1000-entry `AuditLog`,
  stores both on the runtime. Plan 4 IO action call sites pass
  the action name so audit entries identify the call.
- `Runtime::make_action_ctx()` is public so embedders can build an
  `ActionContext` that shares the runtime's services.
- `Runtime::state` is `Rc<StateGraph>` so bindings can capture
  shared state into effect closures.
- Layout engine via `taffy` 0.5 (basic flexbox mapping); spatial
  index via `rstar` (hit + rect queries); viewport math with
  screen↔scene transforms; `RenderBackend` trait + `CaptureBackend`
  for dry-run / tests; `LogicProvider` trait (Tier 3, L4 reserved).
- `LayoutEngine::preload_initial(snapshot, doc_tree)` — installs an
  `aot/initial_layout.bin` snapshot into a `SecondaryMap<NodeKey,
  Rect>` cache so the first paint can serve scene-coord rects
  without running `compute_layout_with_measure`. `node_rect()`
  short-circuits on the cache; the next `build()` clears it so a
  resize-driven relayout falls back to taffy compute. Companion
  APIs: `has_preload()`, `preload_len()`, `preload_covers
  (doc_tree)`, `drop_preload()`. `Runtime::preload_initial_layout
  (snapshot)` delegates; `Runtime::replace_document` calls
  `drop_preload()` so a hot-reload to a fresh doc never serves
  stale rects against new slot keys.
- Typed three-stage `StartupDriver` (`DataPath` pre-window /
  `Visual` first-redraw / `Background` post-paint) with per-phase
  dependency graph and `StartupReport` / `StartupConfig` typed
  surface.
- `HostAgnosticBootstrap::install_data_path` registers DataPath
  phases (`ReadFile` → `ParseSchema` → `SeedStateGraph` →
  `BuildNodeTree` → `InitGpuContext` → `LoadCoreFonts` →
  `ComputeFirstLayout` → `BuildVisibleSpatial`).
  `install_data_path_with_aot(driver, source, viewport, snap)`
  preloads the snapshot from `SeedStateGraph` (only when
  `snap.viewport` bit-matches the bootstrap viewport) and short-
  circuits `ComputeFirstLayout` only when the preload covers every
  doc node — partial coverage drops the cache and runs a real
  compute so new nodes never go rect-less.
- `Runtime::rebuild_spatial_for_first_frame(viewport)` cold-start
  variant that bulk-loads only nodes intersecting the viewport;
  `SpatialIndex::fill_rest` folds in the remainder from the
  Background stage.
- `LazyBinding` / `DeferredBindingQueue` so off-viewport bindings
  evaluate after the first paint.
- End-to-end pipeline smoke test (`counter.op` fixture); Signal
  update microbenchmark (10 / 100 / 1000 subscribers). Integration
  suite `tests/capability_enforcement.rs` (10 tests) and
  `tests/gesture_tap_counter.rs`. 399 lib tests pass.
