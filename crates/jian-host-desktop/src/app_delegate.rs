//! macOS NSApplicationDelegate scaffolding for deep-link routing
//! (Plan 8 §T8 / C5).
//!
//! The runtime-side abstractions ship in
//! [`crate::deeplink`] — [`JianUrl::parse`][crate::deeplink::JianUrl::parse]
//! pulls a `jian://app-id/path?query` into a typed value, and
//! [`DeepLinkHandler`][crate::deeplink::DeepLinkHandler] is the
//! per-host receiver. This module wires that receiver into Cocoa's
//! Apple-Event pipeline so URLs sent by `open jian://...` land in the
//! running app instead of falling on the floor.
//!
//! ## Architecture
//!
//! Cocoa delivers URL-scheme dispatches to the
//! `NSApplicationDelegate.application(_:openURLs:)` selector and file
//! double-clicks to `application(_:openFile:)`. Implementing those
//! requires a real Objective-C subclass of `NSObject` conforming to
//! `NSApplicationDelegate`, registered via `NSApp.delegate = …` —
//! winit installs its own delegate to forward the event-pump events
//! it needs, so this module does NOT replace winit's delegate.
//!
//! Instead it stores the host's
//! [`DeepLinkHandler`][crate::deeplink::DeepLinkHandler] in a
//! main-thread-only `RefCell` and exposes
//! [`dispatch_url`] for the integration glue (whether that's a custom
//! delegate proxy, a winit fork, or a future objc2 hook) to call when
//! a URL arrives. The function:
//!
//! 1. Parses the URL via `JianUrl::parse`.
//! 2. Borrows the registered handler.
//! 3. Calls `handler.handle(url)`.
//! 4. Returns `true` when a handler claimed the URL.
//!
//! ## Single-thread invariant
//!
//! Cocoa fires `application:openURLs:` on the main thread; winit's
//! event loop runs on the main thread too. The
//! [`DeepLinkRegistry`] lives in a `thread_local!` on the main
//! thread — accessing it from a worker thread is a logic bug and
//! produces `RegistryError::WrongThread`.
//!
//! ## Why a thread-local rather than a `Box<dyn Handler>` field on
//! `DesktopHost`
//!
//! The Apple-Event delivery happens before `event_loop.run_app(host)`
//! takes ownership of the host: `[NSApp finishLaunching]` may fire an
//! immediate `application:openURLs:` (when the app was launched by a
//! `jian://` click rather than user double-click). Storing the
//! handler on the host means it's not yet reachable from the
//! delegate at that moment. The thread-local closes the timing gap —
//! `DesktopHost::install_deeplink_handler` registers it before
//! the event loop spins.

#![cfg(target_os = "macos")]

use crate::deeplink::{DeepLinkError, DeepLinkHandler, JianUrl};
use std::cell::RefCell;

thread_local! {
    /// Main-thread-only registry. The `RefCell` lets the delegate
    /// borrow the boxed handler without taking ownership; the boxed
    /// `dyn DeepLinkHandler` lets the host install any concrete
    /// handler at startup time.
    static REGISTRY: RefCell<Option<Box<dyn DeepLinkHandler>>> = const { RefCell::new(None) };
}

/// Errors [`install_handler`] / [`dispatch_url`] can produce.
#[derive(Debug, Clone, PartialEq)]
pub enum RegistryError {
    /// Caller is not on the main thread. Cocoa delivers
    /// Apple-Events on the main thread; routing them through a
    /// worker thread would deadlock the `RefCell` borrow at best
    /// and corrupt the handler state at worst.
    ///
    /// Today this can't actually be observed (the registry is a
    /// `thread_local!`, so cross-thread access produces a fresh
    /// `None` rather than a panic), but the type lets a future
    /// `MainThreadMarker`-checked install path surface the case.
    WrongThread,
    /// `JianUrl::parse` rejected the input. Forwarded verbatim from
    /// [`crate::deeplink::DeepLinkError`].
    BadUrl(DeepLinkError),
    /// No handler was registered when the URL arrived. Cocoa's
    /// default behaviour is to do nothing; the caller may surface
    /// this to a one-shot log line.
    NoHandlerRegistered,
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongThread => write!(f, "deeplink registry accessed off the main thread"),
            Self::BadUrl(e) => write!(f, "bad jian:// URL: {e:?}"),
            Self::NoHandlerRegistered => {
                write!(f, "no DeepLinkHandler installed — URL dropped")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// Install the host's [`DeepLinkHandler`] into the main-thread
/// registry. Call once during `DesktopHost::install_deeplink_handler`,
/// before `event_loop.run_app`. Replacing a previously installed
/// handler is allowed — `Some(old)` is returned so the caller can
/// flush queued events on the way out (today the trait has no such
/// API; reserved for a future hot-reload extension).
pub fn install_handler(handler: Box<dyn DeepLinkHandler>) -> Option<Box<dyn DeepLinkHandler>> {
    REGISTRY.with(|cell| cell.replace(Some(handler)))
}

/// Remove the registered handler. Returns the previously installed
/// handler if any. Use during host teardown so the boxed `dyn
/// DeepLinkHandler` doesn't outlive its captures.
pub fn take_handler() -> Option<Box<dyn DeepLinkHandler>> {
    REGISTRY.with(|cell| cell.borrow_mut().take())
}

/// Parse `url` and dispatch to the registered handler.
///
/// Returns `Ok(true)` when the handler claimed the URL, `Ok(false)`
/// when the handler ran but declined (defaults pass-through to
/// Cocoa's open-document path for `.op` files), and `Err` for parse
/// failures or missing-handler cases. The integration glue (custom
/// delegate proxy or future objc2-app-kit hook) calls this from
/// `application:openURLs:`.
///
/// On `RegistryError::NoHandlerRegistered` the URL is dropped — Cocoa
/// has no second receiver to fall back to once the delegate signals
/// it handled the event, so a missing-handler case is the one place
/// the integration glue must check the error code and decide whether
/// to log or short-circuit before calling this function.
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

    /// Test handler that records every URL it sees. Each test runs in
    /// its own thread (Rust's default test harness) so the
    /// `thread_local!` registry stays isolated.
    #[derive(Default)]
    struct RecordingHandler {
        seen: Vec<JianUrl>,
        accept: bool,
    }
    impl DeepLinkHandler for RecordingHandler {
        fn handle(&mut self, url: JianUrl) -> bool {
            self.seen.push(url);
            self.accept
        }
    }

    /// Wraps a `Rc<RefCell<RecordingHandler>>` so the test can inspect
    /// the `seen` vec after dispatch without needing to take_handler.
    /// `dyn DeepLinkHandler` is `?Sized` so the indirection is
    /// box-around-rc rather than box-on-trait.
    struct RcHandler(std::rc::Rc<std::cell::RefCell<RecordingHandler>>);
    impl DeepLinkHandler for RcHandler {
        fn handle(&mut self, url: JianUrl) -> bool {
            self.0.borrow_mut().handle(url)
        }
    }

    fn shared_recorder(accept: bool) -> std::rc::Rc<std::cell::RefCell<RecordingHandler>> {
        std::rc::Rc::new(std::cell::RefCell::new(RecordingHandler {
            seen: Vec::new(),
            accept,
        }))
    }

    #[test]
    fn dispatch_routes_url_to_installed_handler() {
        let rec = shared_recorder(true);
        install_handler(Box::new(RcHandler(rec.clone())));
        let claimed = dispatch_url("jian://demo/page?id=42").expect("dispatch ok");
        assert!(claimed, "handler returned true → claimed");
        assert_eq!(rec.borrow().seen.len(), 1);
        assert_eq!(rec.borrow().seen[0].app_id, "demo");
        // Cleanup so the next test in this module doesn't see leftover state.
        let _ = take_handler();
    }

    #[test]
    fn dispatch_returns_false_when_handler_declines() {
        let rec = shared_recorder(false);
        install_handler(Box::new(RcHandler(rec.clone())));
        let claimed = dispatch_url("jian://demo/").expect("dispatch ok");
        assert!(!claimed);
        assert_eq!(rec.borrow().seen.len(), 1);
        let _ = take_handler();
    }

    #[test]
    fn dispatch_with_no_handler_returns_no_handler_registered() {
        // Each #[test] runs in its own thread → fresh thread_local.
        let err = dispatch_url("jian://demo/").unwrap_err();
        assert_eq!(err, RegistryError::NoHandlerRegistered);
    }

    #[test]
    fn dispatch_with_malformed_url_returns_bad_url() {
        install_handler(Box::new(RcHandler(shared_recorder(true))));
        let err = dispatch_url("not-a-url").unwrap_err();
        assert!(matches!(err, RegistryError::BadUrl(_)));
        let _ = take_handler();
    }

    #[test]
    fn install_returns_previous_handler_on_replace() {
        let r1 = shared_recorder(true);
        let r2 = shared_recorder(true);
        let prev = install_handler(Box::new(RcHandler(r1)));
        assert!(prev.is_none(), "no handler installed yet");
        let prev = install_handler(Box::new(RcHandler(r2)));
        assert!(prev.is_some(), "replacement returns the old handler");
        let _ = take_handler();
    }
}
