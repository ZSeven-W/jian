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
use jian_action_surface::{ActionSurface, BuildSalt};

/// Channel that delivers new `.op` schemas to a running host. Used by
/// `jian dev` to swap the document on file change without tearing down
/// the window. The receiver lives on the event-loop thread; the
/// matching `Sender` is held by the watcher thread.
pub type ReloadRx = Receiver<PenDocument>;

pub struct DesktopHost {
    pub runtime: Runtime,
    pub backend: SkiaBackend,
    pub config: HostConfig,
    /// When `Some`, the run loop wakes every ~200ms to drain new
    /// documents and rebuild layout. `None` keeps the original
    /// `ControlFlow::Wait` behaviour (zero CPU when idle).
    pub reload_rx: Option<ReloadRx>,
    /// Main-thread end of an MCP `Bridge`. The run loop drains it
    /// once per `about_to_wait` and dispatches each request through
    /// the `mcp_surface` / `RuntimeDispatcher` chain.
    #[cfg(feature = "mcp")]
    pub mcp_drain: Option<McpDrain>,
    /// Live `ActionSurface` rebuilt on every hot-reload. Stays
    /// `None` until `with_mcp` wires the bridge.
    #[cfg(feature = "mcp")]
    pub mcp_surface: Option<ActionSurface>,
    /// Build salt used for action-name derivation. Held so the
    /// reload path can `surface.refresh(doc, &salt)` without the
    /// host author re-supplying it on every save.
    #[cfg(feature = "mcp")]
    pub mcp_salt: BuildSalt,
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
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            title: "Jian".to_owned(),
            initial_size: size(800.0, 600.0),
            menu: None,
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
            #[cfg(feature = "mcp")]
            mcp_drain: None,
            #[cfg(feature = "mcp")]
            mcp_surface: None,
            #[cfg(feature = "mcp")]
            mcp_salt: [0u8; 16],
        }
    }

    pub fn with_config(runtime: Runtime, config: HostConfig) -> Self {
        Self {
            runtime,
            backend: SkiaBackend::new(),
            config,
            reload_rx: None,
            #[cfg(feature = "mcp")]
            mcp_drain: None,
            #[cfg(feature = "mcp")]
            mcp_surface: None,
            #[cfg(feature = "mcp")]
            mcp_salt: [0u8; 16],
        }
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
    /// stores `salt` so each reload re-derives action names with the
    /// same key, and drains queued requests once per `about_to_wait`.
    /// Each request is dispatched against the live `Runtime` through
    /// `RuntimeDispatcher`, exactly like the in-process API.
    ///
    /// Activating MCP also forces the run loop into the same
    /// ~200ms-poll mode `with_reloader` uses so a quiet client still
    /// gets serviced when no UI events arrive — `about_to_wait` runs
    /// once per tick, drains the bridge, replies via the oneshot.
    #[cfg(feature = "mcp")]
    pub fn with_mcp(mut self, drain: McpDrain, salt: BuildSalt) -> Self {
        let surface = self
            .runtime
            .document
            .as_ref()
            .map(|doc| ActionSurface::from_document(&doc.schema, &salt).with_session_id("mcp"));
        self.mcp_drain = Some(drain);
        self.mcp_surface = surface;
        self.mcp_salt = salt;
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
        };
        let host = DesktopHost::with_config(rt, cfg);
        assert_eq!(host.title(), "Custom");
        assert_eq!(host.initial_size().width, 320.0);
        assert_eq!(host.initial_size().height, 200.0);
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
        assert_eq!(host.mcp_salt, [9u8; 16], "salt held for reload re-derive");
    }
}
