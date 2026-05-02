# Changelog

All entries roll up into the workspace's `0.0.1` development release;
sections within tag the originating Plan for traceability.

## [0.0.1] - Unreleased

### Added

**Plan 8 §T8 macOS `kAEGetURL` Apple-Event receiver:**

- `apple_event_receiver` module (macOS-only) — registers a
  `JianAppleEventReceiver` `NSObject` subclass with
  `[NSAppleEventManager sharedAppleEventManager]
  setEventHandler:andSelector:forEventClass:andEventID:` keyed on
  `'GURL'` / `'GURL'`. The receiver's `handleURLEvent:withReplyEvent:`
  IMP pulls the URL out of the event's `keyDirectObject` parameter and
  forwards into `app_delegate::dispatch_url`, so `open jian://...`
  routes through the running host instead of the OS' default handler.
- Sits alongside winit's `NSApp.delegate` (NSAppleEventManager is a
  separate dispatch path keyed on event class+id), so neither delegate
  contention nor a winit fork is required.
- Class registration runs inside `OnceLock::get_or_init` so concurrent
  first-time callers can't both reach `ClassBuilder::register` for the
  same class name. Receiver instance parked in a `thread_local!` so
  `Retained` keeps it alive past the FFI registration call.
- Install / uninstall both assert main-thread via `pthread_main_np`
  before touching `NSAppleEventManager`'s registration table.
- The `extern "C"` IMP wraps user `DeepLinkHandler::handle` in
  `catch_unwind` + `mem::forget(payload)` + panic-safe
  `writeln!(io::stderr(), …)` so a panic in the handler never escapes
  the Rust→Cocoa frame as undefined behaviour.
- `deeplink::install_deeplink_handler` / `take_deeplink_handler` —
  cross-platform shim that delegates per-platform. macOS path chains
  `app_delegate::install_handler` then
  `apple_event_receiver::install_apple_event_handler` in the order
  required for `[NSApp finishLaunching]` synchronous URL delivery.
- `objc2-foundation` features extended to include `NSAppleEventManager`
  + `NSAppleEventDescriptor`. `paramDescriptorForKeyword:` and
  `removeEventHandlerForEventClass:andEventID:` are not in
  objc2-foundation 0.2.2's generated bindings yet; both routed via raw
  `msg_send!` against documented signatures.
- Codex-reviewed across 6 rounds; final pass clean. End-to-end runtime
  validation needs a real macOS GUI session (`open jian://...` against
  a `cargo bundle`-built `.app`).

**Plan 8 §T8 deep-link foundation:**

- `deeplink::JianUrl::parse(url) → Result<JianUrl, DeepLinkError>` for
  the canonical `jian://<app-id>/<path>?<query>` shape; thread-local
  registry + `dispatch_url` in `app_delegate` (macOS) and
  `win_deeplink` (Windows) wait for OS-side delivery to land.

**Plan 8 §T7 native menu bar:**

- `menus::MenuSpec` declarative spec compatible with `muda` 0.13;
  hooks into `DesktopHost::run` when the `menus` feature is on.

**Plan 8 §T9 updater:**

- `updater::Updater` trait + `selfupdate` feature backed by
  `self_update` 0.41; portable GitHub-Releases pipeline with rustls
  + zip/tar decoders. Sparkle (macOS) and AppImageUpdate (Linux)
  remain trait-level deferred.

**Plan 8 — `DesktopHost::run` real event loop:**

- `DesktopHost::run(self) -> Result<(), EventLoopError>` (under the
  `run` feature) — blocking winit 0.30 event loop that wires every
  `WindowEvent` through `PointerTranslator::translate` into
  `Runtime::dispatch_pointer`, runs `tick()` in `about_to_wait` for
  timer-based recognisers, and requests a redraw when state changes.
- `softbuffer` 0.4 CPU presenter: rasterize via `SkiaBackend` +
  `SkiaSurface::new_raster`, snapshot RGBA8888 through the new
  `SkiaSurface::read_rgba8` helper, pack to `0x00RRGGBB` and present.
  Keeps the host platform-agnostic — Metal / D3D12 / GL surfaces
  stay deferred behind their existing feature flags.
- `scene::collect_draws(document, layout) -> Vec<DrawOp>` — schema-
  agnostic walker that reads `fill[]` via JSON round-trip and emits a
  `DrawOp::Rect` with the first solid-fill colour per node.
- Re-exports `scene::collect_draws` from the crate root.

**Plan 8 — `jian-host-desktop` MVP:**

- `DesktopHost` composition root: owns `Runtime` + `SkiaBackend` +
  `HostConfig` (title + initial size).
- `pointer::PointerTranslator`: stateful winit → `PointerEvent`
  translator. Caches cursor position between events so that
  `MouseInput` (which carries no position) can fire a complete Down /
  Up. `CursorMoved` emits `Hover` when no button is held, `Move`
  otherwise. `Touch` events pass through with phase + finger id. 6
  unit tests covering phase transitions + modifier propagation.
- `keyboard::translate_key` + `modifiers_from_winit`: winit key →
  `(key_string, Modifiers)` with the web-ish naming convention
  (`Enter`, `ArrowLeft`, `Space`, …).
- `services::HistoryRouter` — in-process route stack implementing
  `jian_core::action::services::Router`.
- `services::InMemoryStorage` — BTreeMap-backed `StorageBackend` good
  enough for the MVP; real `rusqlite` lands under a future flag.
- Feature-gated `services::clipboard::DesktopClipboard` — `arboard`
  wrapper. Opt-in via the `clipboard` feature so headless CI skips it.
- `bin/jian-player PATH` — loads a `.op`, runs `Runtime::new_from_document`,
  builds layout, prints node count + initial size.

**Plan 19 visual + background bootstrap (B2/B3):**

- `startup_visual` — runs Splash / FirstFrame / Present /
  EventPumpReady inside the first `RedrawRequested` after `resumed()`.
- Background-stage hooks into `BuildFullSpatial` /
  `LoadRemainingFonts` / `DecodeImages`.

**Plan 19 §C19 D2 — first-frame font plan:**

- `BootstrapHandles::take_core_font_plan` exposes a typed
  per-family codepoint plan a host's font provider uses to request
  first-paint subsets.

### Deferred

- Per-platform Skia surface factories for Metal / D3D12 / OpenGL /
  Vulkan / WebGL — each warrants its own session against real
  hardware. The `jian-skia` skeleton stubs return a typed
  `Err("not yet implemented")`.
- Windows `WM_COPYDATA` hidden message-only window + named-mutex
  single-instance forwarding — paired Windows leg of the macOS
  Apple-Event receiver above.
- `reqwest` network client + SQLite storage backend.
