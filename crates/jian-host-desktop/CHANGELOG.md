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
- `win_deeplink_receiver` module (Windows-only) — pairs with the
  macOS Apple-Event receiver to land Plan 8 §T8 across both
  desktop platforms. `try_acquire_singleton()` returns
  `Singleton::{Primary(SingletonGuard) | Secondary}` via a named-
  mutex (`Local\JianHostDesktop-Singleton`). Primary path also
  pre-creates the named ready event
  (`Local\JianHostDesktop-ReceiverReady`, manual-reset, initially
  unsignaled) at mutex-acquire time so secondaries arriving
  during cold-start always find a live event to wait on.
  `install_receiver_window(&SingletonGuard)` registers a
  `WNDCLASSEXW` (`JianDeepLinkReceiver`, idempotent across the
  process via `OnceLock<Result<...>>` so transient register
  failures don't memoise as "registered"), creates a
  `HWND_MESSAGE` window with our `WindowProc`, and `SetEvent`s
  the singleton's ready event. The `WindowProc` validates
  `dwData == COPYDATA_TAG` (`'JDL1'`), `lpData != NULL`, alignment,
  size + `cbData ≤ COPYDATA_MAX_BYTES` (4 KiB), then decodes
  UTF-16 LE → `String::from_utf16` → `crate::win_deeplink::dispatch_url`.
  `forward_url_to_primary(url)` opens the ready event with
  `SYNCHRONIZE` only, waits up to 5 s with explicit
  `WAIT_OBJECT_0 / WAIT_TIMEOUT / WAIT_FAILED` handling, then
  `FindWindowExW(HWND_MESSAGE, NULL, JianDeepLinkReceiver, NULL)`
  + `SendMessageTimeoutW(SMTO_BLOCK, 5_000ms)`. Returns a typed
  `ForwardOutcome` with `Delivered / NoPeer / SendTimedOut /
  SendFailed { last_error } / PrimaryRejected` so the CLI can
  surface accurate diagnostics (codex caught the prior single
  `Ok(false)` collapsing access-denied / UIPI / hung-pump cases).
- The `extern "system"` `WindowProc` wraps its body in
  `catch_unwind` + `mem::forget(payload)` + panic-safe
  `writeln!(io::stderr(), …)` (mirrors the macOS Apple-Event
  receiver) so a handler panic never escapes the FFI frame.
- Cargo-feature additions for windows-sys 0.61: `Win32_Foundation`,
  `Win32_Security`, `Win32_Storage_FileSystem` (`SYNCHRONIZE`
  constant), `Win32_System_DataExchange` (`COPYDATASTRUCT`),
  `Win32_System_LibraryLoader` (`GetModuleHandleW`),
  `Win32_System_Threading` (`CreateMutexW`/`CreateEventW`/
  `OpenEventW`/`SetEvent`/`WaitForSingleObject`),
  `Win32_UI_WindowsAndMessaging`, `Win32_Graphics_Gdi`
  (`WNDCLASSEXW.hbrBackground`).
- `cross-app spoofing via FindWindowExW` was a round-1 threat-
  model gap; **resolved in Plan 8 §T8 follow-up B** by switching
  the cross-process channel from `WM_COPYDATA` over
  `FindWindowExW` to a per-user named pipe (mirror of
  `jian-asp::transport::named_pipe`'s pattern). See
  `win_deeplink_pipe` module entry below.
- `cold-start message-pump latency`: pipe transport's listener
  thread eliminates this. The listener runs independently of the
  main thread's cold-start work; a secondary's `WriteFile`
  succeeds as soon as the listener `ConnectNamedPipe`-loops once
  (which happens immediately after `install_receiver_window`).
- Windows-only `Cargo.toml` target table for `windows-sys` 0.61
  (separate from macOS's `objc2*` deps).
- Six codex review rounds; final pass clean. cargo check on
  `x86_64-pc-windows-msvc` clean. End-to-end runtime validation
  needs a real Windows GUI session.
- Plan 8 §T8 follow-up A — `deeplink::RuntimeDeepLinkHandler`
  (Clone): buffering DeepLinkHandler that hands URLs off to the
  host's main-loop tick for runtime dispatch. `Rc<RefCell<
  VecDeque<JianUrl>>>` queue; `queue()` accessor lets the CLI
  thread the same Rc into both the platform receiver-side
  install AND the host's `with_deeplink_queue`.
  `for_app(expected)` constructor adds an OS-launcher misroute
  filter (rejects URLs whose `app_id` doesn't match); `new()`
  accepts any. Plus `dispatch_url_into_runtime(url, runtime,
  expected_app_id) -> DispatchOutcome` (`#[non_exhaustive]`
  enum: `Applied { query_writes }` / `AppIdMismatch { url_app_id,
  expected }`) that `nav.push(url.path)` + per-query-key
  `state.route_set(k, Value::String(v))`. Free fn
  `drain_expected_app_id(document)` resolves the filter from
  `document.schema.app.id` with empty-string-collapses-to-None
  semantics so an unset id doesn't silently drop every URL.
  `DesktopHost.pending_deeplinks: Option<Rc<RefCell<...>>>`
  field + `with_deeplink_queue` builder; `about_to_wait` drain
  dispatches each URL with `expected_app_id` resolved per tick.
  Mismatched URLs log to stderr without mutating the runtime.
- Plan 8 §T8 follow-up B — `win_deeplink_pipe` module
  (Windows-only, ~510 LOC): per-user named-pipe transport for
  deep-link forwarding, mirror of `jian-asp::transport::
  named_pipe`'s security model. Pipe name
  `\\.\pipe\jian-deeplink-<user_sid>` (resolved via
  `OpenProcessToken` + `GetTokenInformation(TokenUser)` +
  `ConvertSidToStringSidW`). User-SID DACL `D:P(A;;GA;;;<sid>)`
  built via `ConvertStringSecurityDescriptorToSecurityDescriptorW`
  (`D:P` blocks inherited ACEs; single Allow ACE grants
  GENERIC_ALL to the SID only — no Everyone, no Anonymous, no
  Admins). `PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE`,
  `PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT |
  PIPE_REJECT_REMOTE_CLIENTS`, `nMaxInstances = 1`. Daemon
  listener thread loops `ConnectNamedPipe` → `ReadFile` (until
  `\n` or `PIPE_URL_MAX_BYTES = 4 KiB`) → `Box::into_raw(Box::
  new(url))` → `PostMessageW(hwnd, WM_USER_DEEPLINK_FORWARD =
  WM_USER + 1, 0, leaked_ptr)` → `DisconnectNamedPipe` →
  re-loop. `forward_url_via_pipe(url) -> PipeForwardOutcome`
  (`Delivered` / `NoPeer` / `SendFailed { last_error }`):
  `CreateFileW(GENERIC_WRITE, ...)`, `WriteFile(url + '\n')`,
  `FlushFileBuffers`, `CloseHandle`. `SendableHwnd(usize)` /
  `SendableHandle(usize)` wrappers — store handle as `usize`
  rather than raw `*mut c_void` so the listener-thread closure
  passes Rust's auto-`Send` check (raw-pointer fields make a
  wrapper `!Send` even with explicit `unsafe impl Send`).
- `win_deeplink_receiver` migrated to the pipe transport.
  WindowProc now handles `WM_USER_DEEPLINK_FORWARD` (recovers
  `Box<String>` from lParam, calls `dispatch_url`); `WM_COPYDATA`
  is no longer received from out-of-process secondaries (the
  pipe DACL gates cross-process delivery). `install_receiver_
  window` spawns the pipe listener immediately after
  `CreateWindowExW` and BEFORE signaling the ready event;
  listener-install failure causes `DestroyWindow` + `Err` so
  the CLI hard-exits rather than holding the singleton mutex
  with broken intake (codex round 2 CONCERN). `forward_url_to_
  primary` rewritten to delegate to `forward_url_via_pipe`;
  removed `FindWindowExW` + `SendMessageTimeoutW` +
  `COPYDATASTRUCT` packing + `COPYDATA_TAG` /
  `COPYDATA_MAX_BYTES` / `SEND_TIMEOUT_MS` constants.
  `ForwardOutcome::SendTimedOut` / `PrimaryRejected` variants
  retained for source-compat with the CLI's existing branch arms
  but documented as deprecated under the pipe transport.
- Cargo-feature additions for the pipe transport:
  `Win32_Security_Authorization` (DACL builders),
  `Win32_System_IO` (OVERLAPPED type for `ReadFile`/
  `WriteFile`/`ConnectNamedPipe` signatures),
  `Win32_System_Pipes` (`CreateNamedPipeW` /
  `ConnectNamedPipe` / pipe-mode flags).
- 26 deeplink unit tests pass (was 13 in round 1) covering URL
  parse round-trip, `RuntimeDeepLinkHandler` buffer/drain,
  cross-app filter accept+reject, `dispatch_url_into_runtime`
  navigates + seeds query, app-id mismatch refused, drain helper
  empty/none/declared/no-document permutations. Four codex
  review rounds for follow-ups A+B; final pass CLEAN with no
  BLOCK and all CONCERNs/NITs addressed or deferred per codex.
- Per-platform Skia surface factories for Metal / D3D12 / OpenGL /
  Vulkan / WebGL are **not yet** shipped — each warrants its own
  session against real hardware. The `jian-skia` skeleton stubs
  return a typed `Err("not yet implemented")`.
- `reqwest` network client + SQLite storage backend are **not
  yet** shipped.
