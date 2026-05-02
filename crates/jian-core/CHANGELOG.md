# Changelog

All entries roll up into the workspace's `0.0.1` development release;
sections within tag the originating Plan for traceability.

## [0.0.1] - Unreleased

### Added

**Plan 19 D1 cold-start runtime preload:**

- `LayoutEngine::preload_initial(snapshot, doc_tree)` — installs an
  `aot/initial_layout.bin` snapshot into a `SecondaryMap<NodeKey, Rect>`
  cache so the first paint can serve scene-coord rects without running
  `compute_layout_with_measure`. `node_rect()` short-circuits on the
  cache; the next `build()` clears it so a resize-driven relayout falls
  back to taffy compute. Companion APIs: `has_preload()`,
  `preload_len()`, `preload_covers(doc_tree)`, `drop_preload()`.
- `Runtime::preload_initial_layout(snapshot)` delegates to the layout
  engine; `Runtime::replace_document` now calls `drop_preload()` so a
  hot-reload to a fresh doc never serves stale rects against new slot
  keys.
- `HostAgnosticBootstrap::install_data_path_with_aot(driver, source,
  viewport, Some(snapshot))` — bootstrap variant that preloads the
  snapshot from `SeedStateGraph` (only when `snap.viewport` bit-matches
  the bootstrap viewport) and short-circuits `ComputeFirstLayout` only
  when the preload covers every doc node. Partial coverage drops the
  cache and runs a real compute so new nodes never go rect-less.
- `StateGraph::dump_default_state()` / `restore_default_state(&snap)` —
  capture / re-seed the six scopes for `aot/default_state.bin`. Restore
  reuses existing Signal slots (via `app_set` / `page_set` / `self_set`
  / `route_set` / `storage_set` / `vars_set`) so binding subscribers
  survive the AOT seed.
- New `route_set` / `storage_set` setters mirroring `app_set`'s
  slot-reuse pattern; previously these scopes had no public setter.
- Tests: 4 layout-level preload + 4 bootstrap-level (full-coverage skip,
  partial-coverage fallback, viewport-mismatch fallback, resize clears)
  + 3 state-graph dump/restore. 399 jian-core lib tests pass.
- Codex-reviewed across 3 rounds; final pass clean.

**Plan 19 cold-start capstone (B-series):**

- Typed three-stage `StartupDriver` (`DataPath` pre-window · `Visual`
  first-redraw · `Background` post-paint) with per-phase dependency
  graph and `StartupReport`/`StartupConfig` typed surface.
- `HostAgnosticBootstrap::install_data_path` registers DataPath phases:
  `ReadFile` (`std::fs::read_to_string`), `ParseSchema`
  (`jian_ops_schema::load_str`), `SeedStateGraph` (`Runtime::
  new_from_document`), `BuildNodeTree` (no-op marker),
  `InitGpuContext` (host-overrides), `LoadCoreFonts`
  (`FontPlan::scan_subtrees`), `ComputeFirstLayout`
  (`Runtime::build_layout`), `BuildVisibleSpatial`
  (`rebuild_spatial_for_first_frame`).
- `Runtime::rebuild_spatial_for_first_frame(viewport)` cold-start
  variant that bulk-loads only nodes intersecting the viewport;
  `SpatialIndex::fill_rest` folds in the remainder from the
  Background stage.
- `LazyBinding` / `DeferredBindingQueue` so off-viewport bindings
  evaluate after the first paint.

**Plan 6 — Capability gate:**

- Top-level `crate::capability` module:
  `CapabilityGate::check(needed, action)`,
  `DeclaredCapabilityGate::new(caps, Some(audit))` writes
  `AuditEntry` per check, `AuditLog` ring buffer + accessors,
  `map::required_capabilities(action_name)` single source of truth,
  `AutomationLevel { Observe, Act, Full }` lattice,
  `PermissionBroker` trait + `NullPermissionBroker` stub.
- `Runtime::new_from_document(schema)` factory: reads
  `schema.app.capabilities`, builds a `DeclaredCapabilityGate` with a
  1000-entry `AuditLog`, stores both on the runtime.
- `Runtime::make_action_ctx()` is now public.
- Plan 4 IO action call sites (`fetch`, `storage_set`, `storage_clear`,
  `storage_wipe`, platform stubs) updated to pass the action name.
- Integration suite `tests/capability_enforcement.rs` (10 tests).

**Plan 5 — Gesture arena:**

- Flutter-style gesture pipeline under `gesture/`: `PointerEvent`
  with unified `PointerKind` / `PointerPhase` / `MouseButtons` /
  `Modifiers`; `hit_test` over `SpatialIndex` returning a z-ordered
  `HitPath`; `Recognizer` trait + `RecognizerState` state machine;
  per-pointer `Arena` with priority-based arbitration on Up.
- MVP recognizers: `TapRecognizer`, `DoubleTapRecognizer`,
  `LongPressRecognizer`, `PanRecognizer`, `HoverRecognizer`. (Scale /
  Rotate landed alongside multi-pointer host-desktop in Plan 9.)
- `SemanticEvent` enum with `handler_key()` mapping to schema
  `events.*` names (camelCase).
- `PointerRouter` top-level dispatcher; `tick(now)` drives timer-
  based recognizers (LongPress).
- `rawPointer` escape hatch.
- `FocusManager` MVP.
- `EventDispatcher` (`dispatch_event`) resolves `events.<key>` and
  runs through Plan 4's `execute_list_shared`.
- Runtime wiring: `gestures: PointerRouter`, `actions:
  SharedRegistry`, `expr_cache`, injected services with Null defaults.
- `dispatch_pointer(event)` / `tick(now)` end-to-end.

**Plan 4 — Tier 2 Action DSL:**

- `ActionImpl` (`async_trait(?Send)`) + `ActionChain::run_serial`
  driver. `ActionRegistry` + `SharedRegistry` (`Rc<RefCell<...>>`)
  for nested re-parse of control-flow action bodies. `execute_list`
  facade powered by `futures::executor::block_on`.
- Action catalogue: state (`set` / `delete` / `reset`); control flow
  (`if` / `abort` / `delay` / `for_each` / `parallel` / `race`);
  navigation (`push` / `replace` / `pop` / `reset` / `open_url`);
  network (`fetch` with `loading` / `into` / `on_error` chain +
  `Capability::Network` gate); storage (`storage_set` /
  `storage_clear` / `storage_wipe`); UI feedback (`toast` / `alert`
  / `confirm`); L4 platform stubs (`vibrate` / `haptic` / `share` /
  `notify`); Tier 3 (`call` via `LogicProvider`).
- Platform service traits + Null impls in `services/`.
- `CancellationToken` honoured between awaits.
- `Expression::eval_with_locals` for `for_each` HOF locals.

**Plan 3 — Tier 1 expressions:**

- Lexer, recursive-descent parser, AST, bytecode, stack-machine VM.
- Scope references (`$app / $page / $self / $route / $storage /
  $vars`, contextual `$state`, local `$item / $index / $acc`).
- Template literals (`` `text ${expr}` ``).
- Builtins: math (10), string (11), array + HOF
  (filter/map/sort/reduce), object (4), date (3 MVP), type ops (5).
- `Expression` facade + `ExpressionCache`.
- `BindingEffect` for reactive scene-property updates.
- Fine-grained Signal subscription: static member chains fold into
  a single `PushScopeRef`.
- Proptest fuzz (512 cases) + criterion `expr_eval` benches.

**Plan 2 — Runtime baseline:**

- Runtime composition root (`Runtime`).
- Document runtime (SlotMap-backed tree + ID index).
- Fine-grained reactive primitives: `Signal<T>`, `Scheduler`,
  `Effect`.
- State graph with six scopes: `$app`, `$page`, `$self`, `$route`,
  `$storage`, `$vars`.
- Layout engine via `taffy` 0.5 (basic flexbox mapping).
- Spatial index via `rstar` (hit + rect queries).
- Viewport math with screen↔scene transforms.
- `RenderBackend` trait + `CaptureBackend` for dry-run / tests.
- `LogicProvider` trait (Tier 3, L4 reserved).
- End-to-end pipeline smoke test (`counter.op` fixture).
- Signal update microbenchmark (10/100/1000 subscribers).

### Changed

- `Runtime::state` is `Rc<StateGraph>` (Plan 3) so bindings can
  capture shared state into effect closures.

### Fixed

**Post-Codex review (Plan 5/6):**

- `LongPressRecognizer` claiming via `tick()` now resolves the arena
  so a subsequent Up event cannot also let `TapRecognizer` claim on
  the same pointer sequence. `Arena::tick(now)` is the new timer-
  aware variant that mirrors the event-driven `dispatch` path.
- `open_url` consults the capability gate and records to the audit
  log. Undeclared `network` → `CapabilityDenied`.
- `DoubleTap` is detected at the `PointerRouter` level by pairing
  consecutive `Tap` emissions per node (≤ 300 ms, ≤ 16 px). The
  in-arena `DoubleTapRecognizer` couldn't see across arenas; this
  wiring makes `onDoubleTap` handlers fire end-to-end.
