//! `DesktopHost` — the composition root for a windowed Jian app.
//!
//! The struct owns the `Runtime`, the `SkiaBackend`, and the config
//! needed to eventually open a real `winit` window. The `run` feature
//! activates the event loop; by default this module stays pure so CI
//! and unit tests can still construct a host without a display.

use jian_core::geometry::{size, Size};
use jian_core::Runtime;
use jian_ops_schema::document::PenDocument;
use jian_skia::SkiaBackend;
use std::sync::mpsc::Receiver;

#[cfg(feature = "mcp")]
use jian_action_surface::mcp::Drain as McpDrain;
#[cfg(feature = "mcp")]
use jian_action_surface::{ActionAuditLog, ActionSurface, BuildSalt};
#[cfg(feature = "mcp")]
use std::rc::Rc;

/// Channel that delivers new `.op` schemas to a running host. Used by
/// `jian dev` to swap the document on file change without tearing down
/// the window. The receiver lives on the event-loop thread; the
/// matching `Sender` is held by the watcher thread.
pub type ReloadRx = Receiver<PenDocument>;

/// Outcome the run loop should observe after a menu handler fires.
/// `Quit` exits the event loop on the next tick; `Handled` keeps
/// running. `Unknown` is the default for ids the handler doesn't
/// recognise — currently logged to stderr by the run loop and
/// otherwise ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuOutcome {
    Quit,
    Handled,
    Unknown,
}

/// Type alias for the host-installed menu callback: takes the fired
/// item id and a mutable Runtime borrow so handlers can run an
/// `ActionList` against `&mut Runtime` directly.
pub type MenuHandler = Box<dyn FnMut(&str, &mut Runtime) -> MenuOutcome>;

/// Helper: a [`MenuHandler`] that handles `app.quit` (returns
/// `MenuOutcome::Quit`) and routes everything else to a user-supplied
/// closure. Hosts that just want to add a few custom items don't have
/// to re-implement quit semantics each time.
pub fn quit_aware_menu_handler<F>(mut user: F) -> MenuHandler
where
    F: FnMut(&str, &mut Runtime) -> MenuOutcome + 'static,
{
    Box::new(move |id: &str, runtime: &mut Runtime| match id {
        "app.quit" => MenuOutcome::Quit,
        _ => user(id, runtime),
    })
}

pub struct DesktopHost {
    pub runtime: Runtime,
    pub backend: SkiaBackend,
    pub config: HostConfig,
    /// When `Some`, the run loop wakes every ~200ms to drain new
    /// documents and rebuild layout. `None` keeps the original
    /// `ControlFlow::Wait` behaviour (zero CPU when idle).
    pub reload_rx: Option<ReloadRx>,
    /// Routing callback for `muda::MenuEvent`. The run loop drains the
    /// global menu-event channel each `about_to_wait` and forwards
    /// every fired item id (e.g. `"app.quit"`, `"file.open"`,
    /// `"edit.undo"`) to this closure. The default `None` value drops
    /// menu clicks on the floor — useful for headless / single-window
    /// apps that haven't wired UI semantics yet.
    pub menu_handler: Option<MenuHandler>,
    /// Main-thread end of an MCP `Bridge`. The run loop drains it
    /// once per `about_to_wait` and dispatches each request through
    /// the `mcp_surface` / `RuntimeDispatcher` chain.
    #[cfg(feature = "mcp")]
    pub mcp_drain: Option<McpDrain>,
    /// Live `ActionSurface` rebuilt on every hot-reload. Stays
    /// `None` when `with_mcp` is called before a document loads;
    /// `apply_reload` lazily builds it once the first schema arrives.
    #[cfg(feature = "mcp")]
    pub mcp_surface: Option<ActionSurface>,
    /// Build salt used for action-name derivation. Held so the
    /// reload path can `surface.refresh(doc, &salt)` without the
    /// host author re-supplying it on every save.
    #[cfg(feature = "mcp")]
    pub mcp_salt: BuildSalt,
    /// Audit ring buffer attached to every fresh `ActionSurface` the
    /// host derives from a document. Held here so reload-time
    /// re-derivation reuses the same log (audit history survives
    /// hot-reload, matching designer expectations for `jian dev`).
    #[cfg(feature = "mcp")]
    pub mcp_audit: Option<Rc<ActionAuditLog>>,
}

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub title: String,
    pub initial_size: Size,
    /// Optional native menu bar. When `Some` the run loop builds
    /// a `muda::Menu` on first window create and `init_for_*` it
    /// against the active window. When `None`, no menu attaches —
    /// useful for headless / kiosk-style apps. Defaults to
    /// `MenuSpec::default_app_spec(<title>)` via `with_default_menu`.
    pub menu: Option<crate::menus::MenuSpec>,
    /// Optional pre-decoded window icon (RGBA8 pixel buffer + size).
    /// When `Some` the run loop applies it via
    /// `WindowAttributes::with_window_icon` at create time, so it
    /// shows in the **Windows taskbar + X11 WM titlebar**.
    /// Per-platform support: macOS and Wayland return early from
    /// `winit::window::Window::set_window_icon` (unsupported by the
    /// platform), so on those targets the runtime icon is a no-op
    /// and the bundle / `.desktop` icon takes over (Plan 8 Task 10
    /// packaging). Hosts decode the schema's
    /// `app.icon: Option<String>` source via their preferred
    /// `app_icon::AppIconLoader` impl before constructing
    /// `HostConfig`.
    pub icon: Option<crate::app_icon::AppIcon>,
    /// Open the window borderless-fullscreen on the current monitor.
    /// We deliberately use winit's borderless variant rather than
    /// exclusive fullscreen — it skips the resolution-change dance
    /// and works the same way on every platform without a video-mode
    /// query. Default `false`.
    pub fullscreen: bool,
    /// Override the DPI scale factor reported by the OS. `None` keeps
    /// the winit-reported value (typical: 1.0 on standard displays,
    /// 2.0 on Retina, fractional on Windows). Useful for forcing 1×
    /// rendering during a HiDPI screenshot diff or stress-testing
    /// fractional scaling without needing physical hardware. Pointer
    /// coordinates and the canvas transform both use this value.
    pub dpi_override: Option<f64>,
    /// Render a small developer HUD strip at top-left each frame:
    /// physical size / scale factor / draw-op count. The HUD draws
    /// after the scene and before the surface flush so it can never
    /// be hidden by the document. Default `false`.
    pub debug_overlay: bool,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            title: "Jian".to_owned(),
            initial_size: size(800.0, 600.0),
            menu: None,
            icon: None,
            fullscreen: false,
            dpi_override: None,
            debug_overlay: false,
        }
    }
}

impl DesktopHost {
    pub fn new(runtime: Runtime, title: impl Into<String>) -> Self {
        Self {
            runtime,
            backend: SkiaBackend::new(),
            config: HostConfig {
                title: title.into(),
                ..HostConfig::default()
            },
            reload_rx: None,
            menu_handler: None,
            #[cfg(feature = "mcp")]
            mcp_drain: None,
            #[cfg(feature = "mcp")]
            mcp_surface: None,
            #[cfg(feature = "mcp")]
            mcp_salt: [0u8; 16],
            #[cfg(feature = "mcp")]
            mcp_audit: None,
        }
    }

    pub fn with_config(runtime: Runtime, config: HostConfig) -> Self {
        Self {
            runtime,
            backend: SkiaBackend::new(),
            config,
            reload_rx: None,
            menu_handler: None,
            #[cfg(feature = "mcp")]
            mcp_drain: None,
            #[cfg(feature = "mcp")]
            mcp_surface: None,
            #[cfg(feature = "mcp")]
            mcp_salt: [0u8; 16],
            #[cfg(feature = "mcp")]
            mcp_audit: None,
        }
    }

    /// Install a handler that fires whenever a `muda` menu item
    /// matching one of `config.menu`'s ids is clicked. Receives the
    /// item id (e.g. `"file.save"`) and a mutable `Runtime` reference
    /// so handlers can run `execute_list` against `&mut Runtime`
    /// directly. Return `MenuOutcome::Quit` to ask the run loop to
    /// exit on the next tick.
    ///
    /// Idempotent — calling more than once replaces the previous
    /// handler. `app.quit` is *not* short-circuited here; wrap your
    /// closure with [`quit_aware_menu_handler`] if you want the
    /// default Quit semantic without writing a match arm yourself.
    pub fn with_menu_handler(mut self, handler: MenuHandler) -> Self {
        self.menu_handler = Some(handler);
        self
    }

    /// Attach a `Receiver` that delivers fresh `.op` schemas. Activates
    /// dev-mode polling — the run loop wakes every ~200ms to drain
    /// pending reloads instead of sleeping on `ControlFlow::Wait`.
    pub fn with_reloader(mut self, rx: ReloadRx) -> Self {
        self.reload_rx = Some(rx);
        self
    }

    /// Wire an MCP `Bridge::Drain` into the run loop. The host
    /// builds an `ActionSurface` from the runtime's current document,
    /// attaches a fresh `ActionAuditLog` (capacity 256 — spec §8.1's
    /// ring-buffer guidance), stores `salt` so each reload re-derives
    /// action names with the same key, and drains queued requests
    /// once per `about_to_wait`. Each request is dispatched against
    /// the live `Runtime` through `RuntimeDispatcher`, exactly like
    /// the in-process API. Audit rows ride the same log; `session_id`
    /// defaults to `"mcp"`.
    ///
    /// Hosts that want to inspect audit history (or share it with the
    /// in-process surface) read `host.mcp_audit` after construction.
    ///
    /// Lifecycle: if no document is loaded yet (`runtime.document`
    /// is `None`), `mcp_surface` stays `None` and the first
    /// hot-reload (`apply_reload`) lazily builds it. The bridge
    /// drains pending requests but skips them until the surface
    /// exists — clients should expect a brief startup window where
    /// `tools/list` returns empty before the first save lands.
    ///
    /// Activating MCP also forces the run loop into the same
    /// ~200ms-poll mode `with_reloader` uses so a quiet client still
    /// gets serviced when no UI events arrive — `about_to_wait` runs
    /// once per tick, drains the bridge, replies via the oneshot.
    #[cfg(feature = "mcp")]
    pub fn with_mcp(mut self, drain: McpDrain, salt: BuildSalt) -> Self {
        let audit = Rc::new(ActionAuditLog::new(256));
        let surface = self.runtime.document.as_ref().map(|doc| {
            ActionSurface::from_document(&doc.schema, &salt)
                .with_audit(audit.clone())
                .with_session_id("mcp")
        });
        self.mcp_drain = Some(drain);
        self.mcp_surface = surface;
        self.mcp_salt = salt;
        self.mcp_audit = Some(audit);
        self
    }

    /// Attach the standard File / Edit / View / Help menu skeleton
    /// keyed off the current `config.title`. Hosts that want a
    /// custom menu set `config.menu` directly.
    pub fn with_default_menu(mut self) -> Self {
        let title = self.config.title.clone();
        self.config.menu = Some(crate::menus::MenuSpec::default_app_spec(&title));
        self
    }

    /// Set the runtime window icon. The icon is applied via
    /// `winit::window::WindowAttributes::with_window_icon` at window
    /// creation time and shows in the Windows taskbar + X11 WM
    /// titlebar. macOS and Wayland do not honour this (winit returns
    /// early); on those platforms the bundle icon (Plan 8 Task 10
    /// packaging) takes over. Pass `None` to drop a previously-set
    /// icon. Hosts decode the schema's `app.icon: Option<String>`
    /// via an [`crate::app_icon::AppIconLoader`] impl before
    /// calling this.
    pub fn with_icon(mut self, icon: Option<crate::app_icon::AppIcon>) -> Self {
        self.config.icon = icon;
        self
    }

    /// Open the window borderless-fullscreen on the current monitor.
    /// Equivalent to setting `HostConfig::fullscreen = true`.
    pub fn fullscreen(mut self, on: bool) -> Self {
        self.config.fullscreen = on;
        self
    }

    /// Install `jian_skia::SkiaMeasure` as the runtime's text-measure
    /// backend so layout matches the rendered output (CJK widths,
    /// per-segment styling, etc). Available only when the host crate
    /// is compiled with `--features textlayout`. Headless / CI builds
    /// keep the default `EstimateBackend` and skip the ~15 MB ICU +
    /// HarfBuzz dependency.
    ///
    /// Idempotent: calling more than once just reseats the backend;
    /// the layout tree is rebuilt on the next `build_layout` regardless.
    #[cfg(feature = "textlayout")]
    pub fn with_skia_measure(mut self) -> Self {
        self.install_skia_measure();
        self
    }

    /// In-place sibling of [`Self::with_skia_measure`] for hosts that
    /// already own a `DesktopHost` and want to swap in real shaping
    /// after a hot-reload (or never, if a feature flag is off).
    #[cfg(feature = "textlayout")]
    pub fn install_skia_measure(&mut self) {
        let backend = std::rc::Rc::new(jian_skia::measure::SkiaMeasure::new());
        self.runtime.layout.set_backend(backend);
    }

    pub fn title(&self) -> &str {
        &self.config.title
    }

    pub fn initial_size(&self) -> Size {
        self.config.initial_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_constructs_with_defaults() {
        let rt = Runtime::new();
        let host = DesktopHost::new(rt, "Test");
        assert_eq!(host.title(), "Test");
        assert_eq!(host.initial_size().width, 800.0);
    }

    #[test]
    fn host_accepts_custom_config() {
        let rt = Runtime::new();
        let cfg = HostConfig {
            title: "Custom".into(),
            initial_size: size(320.0, 200.0),
            menu: None,
            icon: None,
            fullscreen: false,
            dpi_override: None,
            debug_overlay: false,
        };
        let host = DesktopHost::with_config(rt, cfg);
        assert_eq!(host.title(), "Custom");
        assert_eq!(host.initial_size().width, 320.0);
        assert_eq!(host.initial_size().height, 200.0);
    }

    #[test]
    fn quit_aware_handler_short_circuits_app_quit() {
        // The helper must always treat `app.quit` as a Quit signal, even
        // when the user closure has different semantics. This pins the
        // invariant so a future refactor that moves quit semantics
        // elsewhere flags the change here, not in production hosts.
        let mut runtime = Runtime::new();
        let mut handler = quit_aware_menu_handler(|_id, _rt| MenuOutcome::Handled);
        assert_eq!(handler("app.quit", &mut runtime), MenuOutcome::Quit);
        assert_eq!(handler("file.save", &mut runtime), MenuOutcome::Handled);
    }

    #[test]
    fn quit_aware_handler_forwards_unknown_ids_to_user_closure() {
        // Verify the helper actually delegates non-quit ids to the
        // user closure (and only those). Sharing state with the
        // closure goes through `Rc<RefCell<...>>` so the closure
        // stays `'static`.
        let mut runtime = Runtime::new();
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let log_inner = log.clone();
        let mut handler = quit_aware_menu_handler(move |id, _rt| {
            log_inner.borrow_mut().push(id.to_owned());
            MenuOutcome::Unknown
        });
        assert_eq!(handler("file.open", &mut runtime), MenuOutcome::Unknown);
        assert_eq!(handler("edit.undo", &mut runtime), MenuOutcome::Unknown);
        assert_eq!(handler("app.quit", &mut runtime), MenuOutcome::Quit);
        let entries = log.borrow().clone();
        assert_eq!(entries, vec!["file.open", "edit.undo"]);
    }

    #[test]
    fn fullscreen_builder_toggles_config_field() {
        // Don't open a window in unit tests — just exercise the
        // builder so a future regression that breaks the config
        // wiring (renamed field, missing builder, etc.) trips here
        // before it ships to a real `jian player --fullscreen`.
        let rt = Runtime::new();
        let host = DesktopHost::new(rt, "FS").fullscreen(true);
        assert!(host.config.fullscreen);

        let rt = Runtime::new();
        let off = DesktopHost::new(rt, "FS").fullscreen(false);
        assert!(!off.config.fullscreen);
    }

    #[test]
    fn with_default_menu_attaches_standard_skeleton() {
        let rt = Runtime::new();
        let host = DesktopHost::new(rt, "MenuApp").with_default_menu();
        let spec = host.config.menu.expect("default menu attached");
        // The skeleton always exposes the app submenu first; its label
        // matches the title we passed in.
        match spec.items.first() {
            Some(crate::menus::MenuItem::Submenu { label, .. }) => {
                assert_eq!(label, "MenuApp");
            }
            other => panic!("expected app submenu first, got {other:?}"),
        }
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn with_mcp_builds_action_surface_when_runtime_has_document() {
        // Sanity: the builder should derive an ActionSurface from the
        // runtime's loaded document. A runtime without a document
        // shouldn't crash; it just leaves `mcp_surface` None until the
        // first hot-reload (which the run loop's `apply_reload` path
        // re-derives from anyway).
        let schema: jian_ops_schema::document::PenDocument = serde_json::from_str(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"plus", "semantics":{ "aiName":"plus" },
                  "events":{ "onTap": [] }
                }
              ]}],
              "children":[]
            }"#,
        )
        .expect("fixture parses");
        let rt = Runtime::new_from_document(schema).expect("runtime");
        let (_bridge, drain) = jian_action_surface::mcp::Bridge::new();
        let host = DesktopHost::new(rt, "Mcp").with_mcp(drain, [9u8; 16]);
        assert!(host.mcp_drain.is_some(), "drain stored");
        assert!(host.mcp_surface.is_some(), "surface derived from doc");
        assert!(host.mcp_audit.is_some(), "audit log attached");
        assert_eq!(host.mcp_salt, [9u8; 16], "salt held for reload re-derive");
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn with_mcp_without_document_defers_surface_until_reload() {
        // Spec §10 lifecycle (Codex round 23 MEDIUM): a host that
        // wires `with_mcp` before any document has loaded must NOT
        // crash. `mcp_surface` stays None — the run loop's
        // `apply_reload` path lazily builds it from the first
        // arriving schema. The drain code in `run.rs` skips
        // requests until then.
        let rt = Runtime::new();
        let (_bridge, drain) = jian_action_surface::mcp::Bridge::new();
        let host = DesktopHost::new(rt, "Mcp").with_mcp(drain, [0u8; 16]);
        assert!(host.mcp_drain.is_some());
        assert!(
            host.mcp_surface.is_none(),
            "no doc loaded → surface deferred to first reload"
        );
        assert!(
            host.mcp_audit.is_some(),
            "audit log still attached up-front"
        );
    }
}
