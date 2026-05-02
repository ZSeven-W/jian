# Changelog

The crate has not been released yet — this log only contains
additions targeted at the workspace's `0.0.1` development release.

## [0.0.1] - Unreleased

### Added

- `DesktopHost` composition root: owns `Runtime` + `SkiaBackend` +
  `HostConfig` (title + initial size).
- `DesktopHost::run(self) -> Result<(), EventLoopError>` (under the
  `run` feature) — blocking winit 0.30 event loop that wires every
  `WindowEvent` through `PointerTranslator::translate` into
  `Runtime::dispatch_pointer`, runs `tick()` in `about_to_wait` for
  timer-based recognisers, and requests a redraw when state changes.
- `softbuffer` 0.4 CPU presenter: rasterize via `SkiaBackend` +
  `SkiaSurface::new_raster`, snapshot RGBA8888 through
  `SkiaSurface::read_rgba8`, pack to `0x00RRGGBB` and present.
  Keeps the host platform-agnostic — Metal / D3D12 / GL surface
  factories ship as feature-gated skeletons.
- `pointer::PointerTranslator`: stateful winit → `PointerEvent`
  translator. Caches cursor position so `MouseInput` (carrying no
  position) can fire complete Down / Up. `CursorMoved` emits
  `Hover` when no button is held, `Move` otherwise. `Touch` events
  pass through with phase + finger id. 6 unit tests cover phase
  transitions + modifier propagation.
- `keyboard::translate_key` + `modifiers_from_winit`: winit key →
  `(key_string, Modifiers)` with the web-ish naming convention
  (`Enter`, `ArrowLeft`, `Space`, …).
- `services::HistoryRouter` (in-process route stack implementing
  `jian_core::action::services::Router`); `services::InMemoryStorage`
  (BTreeMap-backed `StorageBackend` good enough for the MVP);
  feature-gated `services::clipboard::DesktopClipboard` (`arboard`
  wrapper, `clipboard` feature opt-in).
- `scene::collect_draws(document, layout) -> Vec<DrawOp>` schema-
  agnostic walker that reads `fill[]` via JSON round-trip and emits
  a `DrawOp::Rect` with the first solid-fill colour per node.
  Re-exported from the crate root.
- `bin/jian-player PATH` smoke binary that loads a `.op`, runs
  `Runtime::new_from_document`, builds layout, prints node count +
  initial size.
- `startup_visual` runner — drives Splash / FirstFrame / Present /
  EventPumpReady inside the first `RedrawRequested` after
  `resumed()`. Background-stage hooks for `BuildFullSpatial` /
  `LoadRemainingFonts` / `DecodeImages`.
- `BootstrapHandles::take_core_font_plan` exposes a typed per-
  family codepoint plan a host's font provider uses to request
  first-paint subsets.
- `menus::MenuSpec` declarative spec compatible with `muda` 0.13;
  hooks into `DesktopHost::run` when the `menus` feature is on.
- `updater::Updater` trait + `selfupdate` feature backed by
  `self_update` 0.41; portable GitHub-Releases pipeline with
  rustls + zip/tar decoders.
- `deeplink::JianUrl::parse(url)` for the canonical
  `jian://<app-id>/<path>?<query>` shape; thread-local registry +
  `dispatch_url` in `app_delegate` (macOS) and `win_deeplink`
  (Windows). `deeplink::install_deeplink_handler` /
  `take_deeplink_handler` cross-platform shim that delegates per-
  platform.
- `apple_event_receiver` module (macOS-only) — registers a
  `JianAppleEventReceiver` `NSObject` subclass with
  `[NSAppleEventManager sharedAppleEventManager]
  setEventHandler:andSelector:forEventClass:andEventID:` keyed on
  `'GURL'` / `'GURL'`. The receiver's
  `handleURLEvent:withReplyEvent:` IMP pulls the URL out of the
  event's `keyDirectObject` parameter and forwards into
  `app_delegate::dispatch_url`, so `open jian://...` routes
  through the running host instead of the OS' default handler.
  Sits alongside winit's `NSApp.delegate` (NSAppleEventManager is
  a separate dispatch path keyed on event class+id), so neither
  delegate contention nor a winit fork is required. Class
  registration runs inside `OnceLock::get_or_init` so concurrent
  first-time callers can't both reach `ClassBuilder::register`
  for the same class name; receiver instance parked in a
  `thread_local!` so `Retained` keeps it alive past the FFI
  registration call. Install / uninstall both assert main-thread
  via `pthread_main_np` before touching `NSAppleEventManager`'s
  registration table. The `extern "C"` IMP wraps user
  `DeepLinkHandler::handle` in `catch_unwind` +
  `mem::forget(payload)` + panic-safe `writeln!(io::stderr(), …)`
  so a panic in the handler never escapes the Rust→Cocoa frame as
  undefined behaviour. macOS path of `install_deeplink_handler`
  chains `app_delegate::install_handler` then
  `apple_event_receiver::install_apple_event_handler` in the
  order required for `[NSApp finishLaunching]` synchronous URL
  delivery. `paramDescriptorForKeyword:` and
  `removeEventHandlerForEventClass:andEventID:` are not in
  objc2-foundation 0.2.2's generated bindings yet; both routed
  via raw `msg_send!` against documented signatures.
- 85 lib tests pass; one new four_cc-pinning unit test ensures
  the AppleEvent class / id / keyword constants match the
  C-header values (`'GURL' == 0x4755524C`, `'----' == 0x2D2D2D2D`).
  Runtime end-to-end validation of the Apple-Event path needs a
  real macOS GUI session (`open jian://...` against a `cargo
  bundle`-built `.app`).
- Windows `WM_COPYDATA` hidden message-only window + named-mutex
  single-instance forwarding is **not yet** shipped — paired
  Windows leg of the macOS Apple-Event receiver above.
- Per-platform Skia surface factories for Metal / D3D12 / OpenGL /
  Vulkan / WebGL are **not yet** shipped — each warrants its own
  session against real hardware. The `jian-skia` skeleton stubs
  return a typed `Err("not yet implemented")`.
- `reqwest` network client + SQLite storage backend are **not
  yet** shipped.
