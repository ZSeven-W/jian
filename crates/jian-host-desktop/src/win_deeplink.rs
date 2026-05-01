//! Windows deep-link receiver scaffolding (Plan 8 §T8 / C6).
//!
//! Windows routes a click on a `jian://...` link to a fresh
//! `jian.exe` process via the protocol-handler registry keys
//! authored in [`packaging/windows/wix/main.wxs.tmpl`]. Single-
//! instance forwarding is the host's responsibility — without it,
//! every URL click spawns a new window.
//!
//! The conventional shape on Windows is:
//!
//! 1. The first `jian.exe` instance creates a hidden message-only
//!    window with a fixed class name and registers a global atom so
//!    later instances can find it.
//! 2. A second instance launched with a `jian://...` argv
//!    (a) finds the running peer via `FindWindowExW(HWND_MESSAGE,
//!    HWND_NULL, atom, NULL)`, (b) packages the URL bytes into a
//!    `COPYDATASTRUCT`, (c) calls `SendMessageW(peer, WM_COPYDATA,
//!    self_hwnd, &cds)`, and (d) exits.
//! 3. The first instance's `WindowProc` receives `WM_COPYDATA`,
//!    parses the URL via [`crate::deeplink::JianUrl::parse`], and
//!    dispatches to the registered [`DeepLinkHandler`].
//!
//! ## Why this module is scaffolding rather than a full impl
//!
//! The hidden-window + `WindowProc` plumbing pulls in the `windows`
//! crate (or `winapi`, depending on the host's preference) plus a
//! per-window-class `WNDCLASSEXW` registration that needs careful
//! lifetime management for the Box-leak'd `WindowProc` closure. The
//! shape lands cleanly only on a Windows runner with the SDK
//! available — this module ships the public API surface
//! ([`install_handler`], [`take_handler`], [`dispatch_url`]) so the
//! per-platform CI follow-up can drop in the WindowProc body
//! without rewriting the host integration.
//!
//! Until the WindowProc lands, [`dispatch_url`] is the testable
//! seam: a future `WM_COPYDATA` handler simply forwards the URL
//! bytes to it. The integration tests below exercise the
//! handler-routing path directly so the trait wiring stays correct.

#![cfg(target_os = "windows")]

use crate::deeplink::{DeepLinkError, DeepLinkHandler, JianUrl};
use std::cell::RefCell;

thread_local! {
    /// Single-thread registry for the running instance's handler.
    /// Windows message-only windows process `WM_COPYDATA` on the
    /// thread that created the window, so the registry is naturally
    /// thread-bound.
    static REGISTRY: RefCell<Option<Box<dyn DeepLinkHandler>>> = const { RefCell::new(None) };
}

/// Fixed window-class name used by the message-only single-instance
/// receiver. The release pipeline pins this string in the registry
/// keys authored by `wix/main.wxs.tmpl`'s `RegisterUrlScheme`
/// component so the protocol handler's `ShellExecute` argv reaches
/// the right window.
pub const RECEIVER_CLASS_NAME: &str = "JianDeepLinkReceiver";

/// Errors [`install_handler`] / [`dispatch_url`] can surface. Mirror
/// of the macOS shape in [`crate::app_delegate`] so a cross-platform
/// host integration can switch on the same error variants.
#[derive(Debug, Clone, PartialEq)]
pub enum RegistryError {
    BadUrl(DeepLinkError),
    NoHandlerRegistered,
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadUrl(e) => write!(f, "bad jian:// URL: {e:?}"),
            Self::NoHandlerRegistered => write!(f, "no DeepLinkHandler installed — URL dropped"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Install the running instance's [`DeepLinkHandler`]. Call from
/// the main thread before the message loop starts.
pub fn install_handler(handler: Box<dyn DeepLinkHandler>) -> Option<Box<dyn DeepLinkHandler>> {
    REGISTRY.with(|cell| cell.replace(Some(handler)))
}

/// Remove the registered handler (use during teardown so the boxed
/// trait object's captures don't outlive their references).
pub fn take_handler() -> Option<Box<dyn DeepLinkHandler>> {
    REGISTRY.with(|cell| cell.borrow_mut().take())
}

/// Parse `url` and dispatch to the registered handler.
///
/// The future `WindowProc` body in the per-platform follow-up will
/// call this from its `WM_COPYDATA` handler. Until then the function
/// is the integration seam tested by [`tests`] below; the
/// `WM_COPYDATA` plumbing is content-blind once that data lands at
/// this entry point.
pub fn dispatch_url(url: &str) -> Result<bool, RegistryError> {
    let parsed = JianUrl::parse(url).map_err(RegistryError::BadUrl)?;
    REGISTRY.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let handler = borrow.as_mut().ok_or(RegistryError::NoHandlerRegistered)?;
        Ok(handler.handle(parsed))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deeplink::JianUrl;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Recorder {
        seen: Vec<JianUrl>,
        accept: bool,
    }
    struct RcRec(Rc<RefCell<Recorder>>);
    impl DeepLinkHandler for RcRec {
        fn handle(&mut self, url: JianUrl) -> bool {
            let mut r = self.0.borrow_mut();
            r.seen.push(url);
            r.accept
        }
    }
    fn rec(accept: bool) -> Rc<RefCell<Recorder>> {
        Rc::new(RefCell::new(Recorder {
            seen: Vec::new(),
            accept,
        }))
    }

    #[test]
    fn dispatch_routes_url_to_installed_handler() {
        let r = rec(true);
        install_handler(Box::new(RcRec(r.clone())));
        let claimed = dispatch_url("jian://demo/page?id=42").expect("dispatch ok");
        assert!(claimed);
        assert_eq!(r.borrow().seen.len(), 1);
        let _ = take_handler();
    }

    #[test]
    fn dispatch_with_no_handler_returns_no_handler_registered() {
        let err = dispatch_url("jian://demo/").unwrap_err();
        assert_eq!(err, RegistryError::NoHandlerRegistered);
    }

    #[test]
    fn dispatch_with_malformed_url_returns_bad_url() {
        install_handler(Box::new(RcRec(rec(true))));
        let err = dispatch_url("not-a-url").unwrap_err();
        assert!(matches!(err, RegistryError::BadUrl(_)));
        let _ = take_handler();
    }

    #[test]
    fn receiver_class_name_is_pinned() {
        // Wire-format compatibility with the WiX template's registry
        // keys — bumping this requires a coordinated installer
        // update, so the constant is assertion-pinned.
        assert_eq!(RECEIVER_CLASS_NAME, "JianDeepLinkReceiver");
    }
}
