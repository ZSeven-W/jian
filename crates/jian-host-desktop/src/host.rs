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
}

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub title: String,
    pub initial_size: Size,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            title: "Jian".to_owned(),
            initial_size: size(800.0, 600.0),
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
        }
    }

    pub fn with_config(runtime: Runtime, config: HostConfig) -> Self {
        Self {
            runtime,
            backend: SkiaBackend::new(),
            config,
            reload_rx: None,
        }
    }

    /// Attach a `Receiver` that delivers fresh `.op` schemas. Activates
    /// dev-mode polling — the run loop wakes every ~200ms to drain
    /// pending reloads instead of sleeping on `ControlFlow::Wait`.
    pub fn with_reloader(mut self, rx: ReloadRx) -> Self {
        self.reload_rx = Some(rx);
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
        };
        let host = DesktopHost::with_config(rt, cfg);
        assert_eq!(host.title(), "Custom");
        assert_eq!(host.initial_size().width, 320.0);
        assert_eq!(host.initial_size().height, 200.0);
    }
}
