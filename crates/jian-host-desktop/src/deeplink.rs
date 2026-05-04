//! Deep-link / file-association abstractions (Plan 8 Task 8 scaffolding).
//!
//! The concrete platform backends — macOS `CFBundleURLTypes` +
//! `application_open_urls`, Windows registry + per-user named-pipe
//! relay (Plan 8 §T8 follow-up B; see `crate::win_deeplink_pipe`,
//! cfg-gated to `target_os = "windows"` so the intra-doc link is a
//! code span rather than `[`...`]` that fails under Linux rustdoc),
//! Linux `.desktop` `MimeType=` + `x-scheme-handler/jian` — each touch
//! installer / OS-bundle infrastructure that doesn't yet exist in this
//! workspace (Plan 8 Task 10 packaging is a separate follow-up). What
//! ships today is the **runtime-side abstraction** every platform
//! backend will plug into:
//!
//! - [`JianUrl`] parses the canonical `jian://<app-id>/<path>?query`
//!   wire form.
//! - [`DeepLinkHandler`] is the trait a host's deep-link receiver
//!   implements; the handler hands the parsed URL into its router /
//!   document-loader (see `crate::services::router::HistoryRouter`).
//! - [`NullDeepLinkHandler`] is the no-op default for hosts that don't
//!   yet wire a platform-specific receiver.
//!
//! Per-platform receivers (NSApplicationDelegate, Windows registry,
//! `.desktop` registration) land in dedicated follow-up commits; each
//! drops in as a new `DeepLinkHandler` impl behind the corresponding
//! `cfg(target_os)` and feature flag.

use std::collections::BTreeMap;

/// Canonical Jian deep-link URL: `jian://<app-id>/<path>?<query>`.
///
/// `app_id` selects which installed Jian app receives the link
/// (multiple apps can register the same `jian://` scheme; routing
/// between them is the OS launcher's job). `path` is the in-app route
/// the runtime's router should `push` to. `query` is the parsed
/// query-string parameters available to expressions / actions.
///
/// Constructed via [`JianUrl::parse`]; `Display` re-emits the wire
/// form so a host can round-trip a URL through its own logging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JianUrl {
    pub app_id: String,
    pub path: String,
    pub query: BTreeMap<String, String>,
}

/// Errors returned by [`JianUrl::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLinkError {
    /// URL did not start with the canonical `jian://` scheme.
    BadScheme,
    /// The `<app-id>` component (the URL's "host") was empty.
    EmptyAppId,
    /// The path / query syntax was malformed (e.g. an unparseable
    /// `key=value` pair).
    BadPathOrQuery(String),
}

impl std::fmt::Display for DeepLinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeepLinkError::BadScheme => f.write_str("URL must use the jian:// scheme"),
            DeepLinkError::EmptyAppId => f.write_str("missing <app-id> in jian://<app-id>/<path>"),
            DeepLinkError::BadPathOrQuery(reason) => {
                write!(f, "malformed path or query: {reason}")
            }
        }
    }
}

impl std::error::Error for DeepLinkError {}

impl JianUrl {
    /// Parse a `jian://<app-id>[/path][?key=value&…]` URL.
    ///
    /// Returns `BadScheme` for any input that doesn't start with the
    /// literal `jian://` prefix. Empty `<app-id>` is `EmptyAppId`.
    /// The path is everything between the second `/` and the `?` (or
    /// end-of-string); empty path is OK and represented as `"/"`.
    /// Query keys without `=` get an empty string value (matches the
    /// usual `application/x-www-form-urlencoded` convention).
    ///
    /// ### Canonical form
    ///
    /// The parser is lenient on input but strict on the canonical
    /// form returned by the [`Display`][std::fmt::Display] impl:
    ///
    /// - **Empty path canonicalises to `/`.** `jian://app` and
    ///   `jian://app/` are equivalent — both parse to `path == "/"`,
    ///   and `Display` emits the slash. (Same semantics HTTP gives
    ///   to `https://example.com` vs `https://example.com/`.)
    /// - **Query parameters are stored in `BTreeMap`**, so the
    ///   `Display` round-trip emits them in alphabetical order. Two
    ///   inputs that differ only in query-pair order produce the
    ///   same canonical form. Hosts that need original ordering
    ///   should retain the raw URL alongside the parsed value.
    /// - **Duplicate query keys collapse last-wins.** A URL like
    ///   `jian://app/?k=1&k=2` parses to a single `k → "2"` entry.
    ///   Hosts needing multi-value semantics should use a list-typed
    ///   key on the wire (`?k[]=1&k[]=2`) and parse the suffix in
    ///   their own router.
    pub fn parse(s: &str) -> Result<Self, DeepLinkError> {
        const SCHEME: &str = "jian://";
        let rest = s.strip_prefix(SCHEME).ok_or(DeepLinkError::BadScheme)?;

        // Split host at the first `/` or `?` so a host-only URL like
        // `jian://app?x=1` doesn't capture the query into the host
        // (Codex round 1 WARN). Whichever delimiter appears first
        // ends the host segment; the rest goes to path/query.
        let (host, path_q) = match rest.find(['/', '?']) {
            Some(idx) => (&rest[..idx], &rest[idx..]),
            None => (rest, ""),
        };
        if host.is_empty() {
            return Err(DeepLinkError::EmptyAppId);
        }

        // Split path from query at the first `?`.
        let (path, query_str) = match path_q.find('?') {
            Some(idx) => (&path_q[..idx], &path_q[idx + 1..]),
            None => (path_q, ""),
        };
        // Empty path canonicalises to "/" so callers don't have to
        // special-case `jian://app-id` (no trailing slash) vs
        // `jian://app-id/`.
        let path = if path.is_empty() { "/" } else { path };

        let mut query = BTreeMap::new();
        if !query_str.is_empty() {
            for pair in query_str.split('&') {
                if pair.is_empty() {
                    continue;
                }
                let (k, v) = match pair.find('=') {
                    Some(idx) => (&pair[..idx], &pair[idx + 1..]),
                    None => (pair, ""),
                };
                if k.is_empty() {
                    return Err(DeepLinkError::BadPathOrQuery(format!(
                        "empty key in query pair `{pair}`"
                    )));
                }
                // BTreeMap::insert is last-wins for duplicate keys —
                // see the canonical-form note in `parse`'s docstring.
                query.insert(k.to_owned(), v.to_owned());
            }
        }

        Ok(Self {
            app_id: host.to_owned(),
            path: path.to_owned(),
            query,
        })
    }
}

impl std::fmt::Display for JianUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "jian://{}{}", self.app_id, self.path)?;
        if !self.query.is_empty() {
            f.write_str("?")?;
            let mut first = true;
            for (k, v) in &self.query {
                if !first {
                    f.write_str("&")?;
                }
                first = false;
                if v.is_empty() {
                    f.write_str(k)?;
                } else {
                    write!(f, "{k}={v}")?;
                }
            }
        }
        Ok(())
    }
}

/// Receives parsed deep-link URLs from the OS-specific listener
/// (NSApplicationDelegate, Windows registry relay, `.desktop` MIME
/// invocation). Implementations dispatch the URL into a router or
/// document-loader.
///
/// `handle` returns **host-level routing telemetry**: `true` if the
/// URL matched an app this host owns and was routed; `false` if the
/// host doesn't own the URL's `app_id`. **The platform listener
/// callbacks the trait wraps (macOS `application_open_urls`, Windows
/// pipe-listener thread, Linux MIME activation) do not consume an
/// accept/reject return** — `false` is for host-side logging or for
/// dispatching to a fallback handler when one process serves multiple
/// `jian://` apps. Treat the return as advisory, not OS-controlling.
///
/// ### Threading & sharing
///
/// `handle` takes `&mut self` because real backends typically own
/// mutable state (recent-URL cache, route history, document loader
/// pointer). The trait deliberately does not require `Send + Sync`:
/// most platform listeners deliver URLs on the main thread, where
/// non-`Send` state (`Rc`, `RefCell`) is acceptable.
///
/// Sharing patterns the trait shape supports:
///
/// - **Single-thread, single owner** (the common case): the host
///   owns `Box<dyn DeepLinkHandler>` directly and calls `handle` from
///   its main-thread listener.
/// - **Single-thread, multiple references**: wrap in
///   `Rc<RefCell<dyn DeepLinkHandler>>` so multiple subsystems on
///   the main thread can borrow the handler.
/// - **Cross-thread sharing**: needs the full
///   `Arc<Mutex<dyn DeepLinkHandler + Send>>` envelope — `Arc` for
///   shared ownership across threads, `Mutex` for the `&mut self`
///   borrow at call time, and the explicit `+ Send` bound to make
///   the trait object cross thread boundaries. Hosts add this bound
///   at their use site (`fn install(handler: Arc<Mutex<dyn DeepLinkHandler + Send>>)`)
///   rather than baking it into the trait so the single-threaded
///   majority pays no `Send` tax.
pub trait DeepLinkHandler {
    fn handle(&mut self, url: JianUrl) -> bool;
}

/// No-op default. Hosts with no deep-link integration use this as a
/// placeholder so the `DeepLinkHandler` trait surface stays uniform
/// across "wired" and "not wired" builds.
#[derive(Debug, Default, Copy, Clone)]
pub struct NullDeepLinkHandler;

impl DeepLinkHandler for NullDeepLinkHandler {
    fn handle(&mut self, _url: JianUrl) -> bool {
        false
    }
}

/// Buffering [`DeepLinkHandler`] that hands URLs off to the host's
/// main-loop tick for runtime dispatch.
///
/// **Why buffer.** Deep-link handlers are installed BEFORE the
/// runtime exists (the OS may deliver a URL at process launch, e.g.
/// `open jian://app/page` on macOS or a pipe `WriteFile` arriving
/// during cold-start on Windows). The handler runs on the OS event
/// thread (Apple-Event handler thread on macOS; receiver-window
/// thread on Windows) and can't safely touch the runtime — the
/// runtime construction is sequenced through the data-path stage
/// of `HostAgnosticBootstrap`. Queueing keeps the handler synchronous
/// (returns immediately, OS-side delivery succeeds) while the host
/// drains the queue once per `about_to_wait` tick and dispatches
/// each URL into the live runtime.
///
/// **App-id gate.** When `expected_app_id` is `Some`, the handler
/// rejects (returns `false`) URLs whose `app_id` doesn't match. The
/// CLI populates this from the schema's `app.id` so a URL meant for
/// `app.foo` doesn't land in `app.bar`'s runtime when both are
/// installed and a launcher routes the click to either. `None`
/// (the default) accepts any app id — useful for early-load before
/// a schema is parsed, or for hosts that load multiple apps under
/// one runtime.
///
/// **Threading.** `Rc<RefCell<…>>` is fine because all paths run
/// on the same thread:
/// - macOS: the Apple-Event handler runs on the main thread (the
///   `kAEGetURL` IMP is dispatched by `NSAppleEventManager` on
///   `[NSApp run]`'s thread, which is main).
/// - Windows: the receiver `WindowProc` runs on the thread that
///   created the receiver window — which is also main per
///   `install_receiver_window`'s contract.
///
/// A future cross-thread variant (e.g. a named-pipe listener
/// thread) would post URLs through a `Sender<JianUrl>` instead of
/// pushing onto this `RefCell`; the queue type stays the public
/// drain target either way.
///
/// `Clone` is shallow — both clones share the same `Rc` queue, so
/// the CLI can clone the handler once and install separate
/// instances on macOS (Apple-Event registry) and Windows
/// (`win_deeplink::install_handler`) with the queue staying
/// single-source-of-truth.
#[derive(Clone)]
pub struct RuntimeDeepLinkHandler {
    queue: std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<JianUrl>>>,
    expected_app_id: Option<String>,
}

impl RuntimeDeepLinkHandler {
    /// Build a handler that accepts URLs for any `app_id`. Useful
    /// during cold-start before the schema is parsed.
    pub fn new() -> Self {
        Self {
            queue: std::rc::Rc::new(std::cell::RefCell::new(std::collections::VecDeque::new())),
            expected_app_id: None,
        }
    }

    /// Build a handler that only accepts URLs whose `app_id`
    /// matches `expected`. The CLI uses this to reject cross-app
    /// URLs that the OS launcher mis-routed.
    pub fn for_app(expected: impl Into<String>) -> Self {
        Self {
            queue: std::rc::Rc::new(std::cell::RefCell::new(std::collections::VecDeque::new())),
            expected_app_id: Some(expected.into()),
        }
    }

    /// Borrow the shared queue. The host's main-loop tick uses
    /// this to drain pending URLs and dispatch them into the
    /// runtime. `Rc::clone`-cheap; the cell is single-thread-safe.
    pub fn queue(&self) -> std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<JianUrl>>> {
        std::rc::Rc::clone(&self.queue)
    }
}

impl Default for RuntimeDeepLinkHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl DeepLinkHandler for RuntimeDeepLinkHandler {
    fn handle(&mut self, url: JianUrl) -> bool {
        if let Some(expected) = self.expected_app_id.as_ref() {
            if &url.app_id != expected {
                // Cross-app URL — refuse so the OS-side dispatcher
                // can route to a different listener (or the
                // secondary CLI path emits a clear error). Don't
                // log here; the receiver does its own logging.
                return false;
            }
        }
        self.queue.borrow_mut().push_back(url);
        true
    }
}

/// Outcome of [`dispatch_url_into_runtime`]: did we apply the URL,
/// and how many query writes did it produce?
///
/// `#[non_exhaustive]` — codex round 2 NIT: future variants like
/// `RouterRefused` (a typed router gates the path) or
/// `CapabilityDenied` (capability gate refused the implied state
/// write) will land here. Marking the enum non-exhaustive lets
/// downstream `match` arms get a `_ => {}` for new variants
/// without a breaking-change diff.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DispatchOutcome {
    /// URL applied — `nav.push` ran and `count` `state.route_set`
    /// writes happened.
    Applied { query_writes: usize },
    /// URL's `app_id` didn't match the runtime's loaded
    /// `schema.app.id` — refused. Codex round 1 CONCERN: this
    /// drain-time check is the cross-app misroute defense the
    /// receiver-side `RuntimeDeepLinkHandler::for_app` would
    /// normally provide. We do it at drain time because the
    /// schema isn't loaded when the receiver is installed
    /// (cold-start ordering: receiver up before runtime built).
    AppIdMismatch {
        url_app_id: String,
        expected: String,
    },
}

/// Resolve the `expected_app_id` filter the deep-link drain feeds
/// to [`dispatch_url_into_runtime`]. Pulled out as a free function
/// (rather than inlined into the run loop) so tests can pin the
/// schema-id → filter mapping without spinning a winit event loop.
///
/// Returns `Some(id)` when the loaded schema declares a non-empty
/// `app.id`; `None` otherwise. **Empty `app.id` → `None`** is
/// load-bearing — the bare schema-id-to-filter map would refuse
/// every URL whose `app_id` is non-empty (since the URL parser
/// rejects empty `app_id` at parse time per
/// [`DeepLinkError::EmptyAppId`]), silently dropping every
/// legitimate deeplink. The empty-string path is unreachable for a
/// schema-validated `.op` today (the loader requires `app.id`), but
/// defending here keeps the drain robust against future schema
/// relaxation. Codex round 2 CONCERN Q2.
pub fn drain_expected_app_id(
    document: Option<&jian_core::document::RuntimeDocument>,
) -> Option<String> {
    document
        .and_then(|d| d.schema.app.as_ref())
        .map(|a| a.id.clone())
        .filter(|s| !s.is_empty())
}

/// Apply a parsed [`JianUrl`] to a live `jian_core::Runtime`. The
/// host calls this once per drained URL on the main thread:
///
/// 1. **App-id gate**: when `expected_app_id` is `Some`, refuse
///    URLs whose `app_id` doesn't match. Returns
///    [`DispatchOutcome::AppIdMismatch`] without touching the
///    runtime. Codex round 1 CONCERN: prevents an OS-level
///    misroute (the launcher hands us a `jian://otherapp/x`
///    URL because both apps register the same scheme) from
///    silently mutating our app's route stack.
/// 2. `runtime.nav.push(url.path)` so the route stack now has the
///    deep-linked path on top. The router's `current()` reflects
///    it; bindings on `$route.*` re-evaluate via the scheduler.
/// 3. `runtime.state.route_set(k, v)` for every `(k, v)` in the
///    URL's query map so the doc's `$route.<key>` expressions
///    see the deep-link parameters as untyped JSON strings
///    (matching the existing query-string contract from
///    `Router::current`'s `RouteState.query` map).
pub fn dispatch_url_into_runtime(
    url: &JianUrl,
    runtime: &mut jian_core::Runtime,
    expected_app_id: Option<&str>,
) -> DispatchOutcome {
    if let Some(expected) = expected_app_id {
        if url.app_id != expected {
            return DispatchOutcome::AppIdMismatch {
                url_app_id: url.app_id.clone(),
                expected: expected.to_owned(),
            };
        }
    }
    runtime.nav.push(&url.path);
    let mut writes = 0usize;
    for (k, v) in &url.query {
        runtime
            .state
            .route_set(k, serde_json::Value::String(v.clone()));
        writes += 1;
    }
    DispatchOutcome::Applied {
        query_writes: writes,
    }
}

/// Cross-platform install shim. Wires `handler` into the
/// platform-specific receiver registry so OS-delivered URLs route
/// through it:
///
/// - **macOS**: stores the handler in `crate::app_delegate`'s
///   thread-local registry AND registers the `kAEGetURL` Apple-
///   Event handler via `crate::apple_event_receiver`.
/// - **Windows**: stores the handler in `crate::win_deeplink`'s
///   thread-local registry. (Both modules are `cfg`-gated to their
///   target_os and unreachable from a Linux rustdoc build, so the
///   intra-doc links above are inlined as code spans rather than
///   `[`...`]` pairs that would error under `RUSTDOCFLAGS=-D warnings`.)
/// - **Linux / other**: stores the handler in a no-op registry —
///   the `.desktop` MIME entry can dispatch via the player's
///   command-line argv path, which is host-driven.
///
/// Idempotent across calls — the previous handler (if any) is
/// returned. Hosts typically call this exactly once during
/// startup, before `event_loop.run_app`.
pub fn install_deeplink_handler(
    handler: Box<dyn DeepLinkHandler>,
) -> Option<Box<dyn DeepLinkHandler>> {
    #[cfg(target_os = "macos")]
    {
        let prev = crate::app_delegate::install_handler(handler);
        // The Apple-Event handler must register AFTER the registry
        // holds the handler, so a URL arriving immediately on launch
        // (process started by `open jian://...`) finds a live target.
        crate::apple_event_receiver::install_apple_event_handler();
        prev
    }
    #[cfg(target_os = "windows")]
    {
        // Plan 8 §T8 Windows leg: the cross-platform shim only
        // installs the handler registry. The CLI is responsible
        // for `try_acquire_singleton` BEFORE calling this (so a
        // secondary process never reaches here) and for calling
        // `crate::win_deeplink_receiver::install_receiver_window
        // (&singleton_guard)` after the registry has the handler.
        // Splitting these out of the shim is necessary because
        // `install_receiver_window` requires a `&SingletonGuard`
        // for its pre-created ready event handle.
        return crate::win_deeplink::install_handler(handler);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // Suppress unused-variable on non-Mac/Win targets without
        // pretending to install — Linux deeplinks land via argv from
        // the .desktop entry's `%U`, not through a process registry.
        let _ = handler;
        None
    }
}

/// Inverse of [`install_deeplink_handler`]. Used during host
/// teardown so the boxed handler doesn't outlive its captures and
/// the platform-specific registry returns to its pre-install state.
pub fn take_deeplink_handler() -> Option<Box<dyn DeepLinkHandler>> {
    #[cfg(target_os = "macos")]
    {
        crate::apple_event_receiver::uninstall_apple_event_handler();
        crate::app_delegate::take_handler()
    }
    #[cfg(target_os = "windows")]
    {
        // Drop the stored receiver window before the handler so a
        // late pipe-listener PostMessage after teardown can't find a live
        // target with no handler behind it. The thread-local
        // `take` returns the guard which `Drop` destroys.
        let _ = crate::win_deeplink_receiver::take_receiver_window();
        return crate::win_deeplink::take_handler();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_url_with_query() {
        let u = JianUrl::parse("jian://demo.counter/page/home?count=5&dark=true").unwrap();
        assert_eq!(u.app_id, "demo.counter");
        assert_eq!(u.path, "/page/home");
        assert_eq!(u.query.get("count"), Some(&"5".to_string()));
        assert_eq!(u.query.get("dark"), Some(&"true".to_string()));
    }

    #[test]
    fn parses_url_without_path() {
        let u = JianUrl::parse("jian://demo.counter").unwrap();
        assert_eq!(u.app_id, "demo.counter");
        assert_eq!(u.path, "/", "empty path canonicalises to slash");
        assert!(u.query.is_empty());
    }

    #[test]
    fn parses_url_with_trailing_slash_no_query() {
        let u = JianUrl::parse("jian://demo.counter/").unwrap();
        assert_eq!(u.app_id, "demo.counter");
        assert_eq!(u.path, "/");
    }

    #[test]
    fn parses_query_with_valueless_key() {
        let u = JianUrl::parse("jian://app/?flag&q=hi").unwrap();
        assert_eq!(u.query.get("flag"), Some(&"".to_string()));
        assert_eq!(u.query.get("q"), Some(&"hi".to_string()));
    }

    #[test]
    fn host_only_url_with_query_does_not_capture_query_into_host() {
        // Codex round 1 WARN: previous parser used `find('/')` first,
        // which captured `app?x=1` as a single host literal. Fix
        // splits on `/` OR `?` whichever comes first.
        let u = JianUrl::parse("jian://app?x=1").unwrap();
        assert_eq!(u.app_id, "app");
        assert_eq!(u.path, "/", "missing path canonicalises to slash");
        assert_eq!(u.query.get("x"), Some(&"1".to_string()));
    }

    #[test]
    fn duplicate_query_keys_are_last_wins() {
        // Documented canonical-form rule from `parse`'s docstring.
        let u = JianUrl::parse("jian://app/?k=1&k=2&k=last").unwrap();
        assert_eq!(u.query.len(), 1);
        assert_eq!(u.query.get("k"), Some(&"last".to_string()));
    }

    #[test]
    fn display_emits_alphabetised_query_regardless_of_input_order() {
        // BTreeMap iteration is sorted by key; Display reflects that.
        // Two inputs differing only in query-pair order canonicalise
        // to the same Display form.
        let a = JianUrl::parse("jian://app/?b=2&a=1").unwrap();
        let b = JianUrl::parse("jian://app/?a=1&b=2").unwrap();
        assert_eq!(a.to_string(), b.to_string());
        assert_eq!(a.to_string(), "jian://app/?a=1&b=2");
    }

    #[test]
    fn host_only_url_canonicalises_to_trailing_slash_on_display() {
        // Documented: `jian://app` and `jian://app/` are equivalent;
        // Display emits the slash.
        let u = JianUrl::parse("jian://app").unwrap();
        assert_eq!(u.to_string(), "jian://app/");
    }

    #[test]
    fn rejects_non_jian_scheme() {
        assert_eq!(
            JianUrl::parse("https://demo.counter/x"),
            Err(DeepLinkError::BadScheme)
        );
        assert_eq!(JianUrl::parse(""), Err(DeepLinkError::BadScheme));
    }

    #[test]
    fn rejects_empty_app_id() {
        assert_eq!(
            JianUrl::parse("jian:///path"),
            Err(DeepLinkError::EmptyAppId)
        );
        assert_eq!(JianUrl::parse("jian://"), Err(DeepLinkError::EmptyAppId));
    }

    #[test]
    fn rejects_empty_query_key() {
        match JianUrl::parse("jian://app/?=value") {
            Err(DeepLinkError::BadPathOrQuery(_)) => {}
            other => panic!("expected BadPathOrQuery, got {other:?}"),
        }
    }

    #[test]
    fn round_trips_through_display() {
        let original = "jian://demo.counter/page/home?count=5&dark=true";
        let u = JianUrl::parse(original).unwrap();
        // BTreeMap iterates keys in sorted order, so the round-tripped
        // query is alphabetised. `count < dark` happens to match
        // input order; this asserts the canonical Display form.
        assert_eq!(u.to_string(), original);
    }

    #[test]
    fn round_trips_with_no_query() {
        let original = "jian://demo.counter/page/home";
        let u = JianUrl::parse(original).unwrap();
        assert_eq!(u.to_string(), original);
    }

    #[test]
    fn null_handler_returns_false() {
        let mut h = NullDeepLinkHandler;
        let u = JianUrl::parse("jian://demo.counter/").unwrap();
        assert!(!h.handle(u));
    }

    /// Demonstrates the canonical custom-impl shape that future
    /// platform backends (NSApplicationDelegate, Windows registry,
    /// .desktop MIME) will follow: store the most-recent URL on
    /// `handle`, return whether it matched the host's expected
    /// app-id.
    #[test]
    fn custom_handler_is_invoked_with_parsed_url() {
        struct Recording {
            expected_app_id: &'static str,
            last: Option<JianUrl>,
        }
        impl DeepLinkHandler for Recording {
            fn handle(&mut self, url: JianUrl) -> bool {
                let matched = url.app_id == self.expected_app_id;
                self.last = Some(url);
                matched
            }
        }
        let mut h = Recording {
            expected_app_id: "demo.counter",
            last: None,
        };
        assert!(h.handle(JianUrl::parse("jian://demo.counter/x").unwrap()));
        assert!(!h.handle(JianUrl::parse("jian://other.app/y").unwrap()));
        assert_eq!(h.last.as_ref().unwrap().app_id, "other.app");
    }

    #[test]
    fn runtime_deep_link_handler_buffers_urls_for_drain() {
        let mut handler = RuntimeDeepLinkHandler::new();
        let q = handler.queue();
        assert!(handler.handle(JianUrl::parse("jian://demo/page1").unwrap()));
        assert!(handler.handle(JianUrl::parse("jian://demo/page2?id=42").unwrap()));
        let drained: Vec<JianUrl> = q.borrow_mut().drain(..).collect();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].path, "/page1");
        assert_eq!(drained[1].path, "/page2");
        assert_eq!(drained[1].query.get("id"), Some(&"42".to_string()));
    }

    #[test]
    fn runtime_deep_link_handler_rejects_cross_app_urls() {
        let mut handler = RuntimeDeepLinkHandler::for_app("demo.counter");
        let q = handler.queue();
        assert!(handler.handle(JianUrl::parse("jian://demo.counter/x").unwrap()));
        // Cross-app: refused, not buffered.
        assert!(!handler.handle(JianUrl::parse("jian://other.app/y").unwrap()));
        assert_eq!(q.borrow().len(), 1, "cross-app URL was buffered");
        assert_eq!(q.borrow()[0].app_id, "demo.counter");
    }

    #[test]
    fn runtime_deep_link_handler_default_app_id_accepts_any() {
        let mut handler = RuntimeDeepLinkHandler::new();
        let q = handler.queue();
        // Three different app_ids — all accepted.
        assert!(handler.handle(JianUrl::parse("jian://app.a/x").unwrap()));
        assert!(handler.handle(JianUrl::parse("jian://app.b/y").unwrap()));
        assert!(handler.handle(JianUrl::parse("jian://app.c/z").unwrap()));
        assert_eq!(q.borrow().len(), 3);
    }

    #[test]
    fn dispatch_url_into_runtime_pushes_route_and_seeds_query() {
        // Build a minimal runtime; install `HistoryRouter` so `nav.push`
        // is observable. `state` is the default `StateGraph`.
        use crate::services::router::HistoryRouter;
        use jian_core::Runtime;
        use std::rc::Rc;

        let mut rt = Runtime::new();
        let router = Rc::new(HistoryRouter::new("/"));
        rt.nav = router.clone();

        let url = JianUrl::parse("jian://demo/detail/42?ref=email&utm=launch").unwrap();
        let outcome = dispatch_url_into_runtime(&url, &mut rt, None);
        assert_eq!(
            outcome,
            DispatchOutcome::Applied { query_writes: 2 },
            "two query keys → two route_set writes"
        );
        assert_eq!(router.snapshot(), vec!["/", "/detail/42"]);
        // `StateGraph` exposes a snapshot dump but no per-key read for
        // `$route` (the runtime evaluates `$route.<key>` via the
        // expression VM's `lookup_scope` path). Pull the snapshot and
        // assert the writes landed in the `route` scope.
        let snap = rt.state.dump_default_state();
        assert_eq!(
            snap.route.get("ref"),
            Some(&serde_json::Value::String("email".into()))
        );
        assert_eq!(
            snap.route.get("utm"),
            Some(&serde_json::Value::String("launch".into()))
        );
    }

    #[test]
    fn dispatch_url_into_runtime_no_query_still_navigates() {
        use crate::services::router::HistoryRouter;
        use jian_core::Runtime;
        use std::rc::Rc;

        let mut rt = Runtime::new();
        let router = Rc::new(HistoryRouter::new("/"));
        rt.nav = router.clone();

        let url = JianUrl::parse("jian://demo/about").unwrap();
        let outcome = dispatch_url_into_runtime(&url, &mut rt, None);
        assert_eq!(outcome, DispatchOutcome::Applied { query_writes: 0 });
        assert_eq!(router.snapshot(), vec!["/", "/about"]);
    }

    #[test]
    fn dispatch_url_into_runtime_rejects_cross_app_when_expected_set() {
        // Codex round 1 CONCERN: drain-time app-id filter
        // prevents OS-level misroutes from mutating our route
        // stack. URL says `otherapp`; we expect `demo`. Refuse.
        use crate::services::router::HistoryRouter;
        use jian_core::Runtime;
        use std::rc::Rc;

        let mut rt = Runtime::new();
        let router = Rc::new(HistoryRouter::new("/"));
        rt.nav = router.clone();

        let url = JianUrl::parse("jian://otherapp/x?k=v").unwrap();
        let outcome = dispatch_url_into_runtime(&url, &mut rt, Some("demo"));
        assert_eq!(
            outcome,
            DispatchOutcome::AppIdMismatch {
                url_app_id: "otherapp".into(),
                expected: "demo".into()
            }
        );
        // Runtime untouched: route stack still just `/`, no
        // route writes leaked.
        assert_eq!(router.snapshot(), vec!["/"]);
    }

    fn doc_with_app_id(id: &str) -> jian_core::document::RuntimeDocument {
        use jian_ops_schema::load_str;
        let json = format!(
            r##"{{
              "formatVersion": "1.0",
              "version": "1.0.0",
              "id": "test-doc",
              "app": {{ "id": "{id}", "name": "Test", "version": "1" }},
              "children": []
            }}"##
        );
        let schema = load_str(&json).expect("parse").value;
        let rt = jian_core::Runtime::new_from_document(schema).expect("runtime");
        rt.document.expect("document populated")
    }

    fn doc_without_app_block() -> jian_core::document::RuntimeDocument {
        use jian_ops_schema::load_str;
        let json = r##"{
          "formatVersion": "1.0",
          "version": "1.0.0",
          "id": "no-app",
          "children": []
        }"##;
        let schema = load_str(json).expect("parse").value;
        let rt = jian_core::Runtime::new_from_document(schema).expect("runtime");
        rt.document.expect("document populated")
    }

    #[test]
    fn drain_expected_app_id_returns_id_when_schema_declares_one() {
        let doc = doc_with_app_id("demo.counter");
        assert_eq!(
            drain_expected_app_id(Some(&doc)),
            Some("demo.counter".to_string())
        );
    }

    #[test]
    fn drain_expected_app_id_returns_none_when_schema_has_no_app_block() {
        let doc = doc_without_app_block();
        assert_eq!(drain_expected_app_id(Some(&doc)), None);
    }

    #[test]
    fn drain_expected_app_id_returns_none_for_no_runtime_document() {
        assert_eq!(drain_expected_app_id(None), None);
    }

    #[test]
    fn drain_expected_app_id_returns_none_for_empty_app_id() {
        // Codex round 2 CONCERN Q2: empty `app.id` mapped to
        // `Some("")` would silently reject every URL since
        // `JianUrl::parse` rejects empty `app_id` at parse time.
        // The filter must collapse empty to None.
        let doc = doc_with_app_id("");
        assert_eq!(drain_expected_app_id(Some(&doc)), None);
    }

    #[test]
    fn dispatch_url_into_runtime_passes_when_app_id_matches() {
        use crate::services::router::HistoryRouter;
        use jian_core::Runtime;
        use std::rc::Rc;

        let mut rt = Runtime::new();
        let router = Rc::new(HistoryRouter::new("/"));
        rt.nav = router.clone();

        let url = JianUrl::parse("jian://demo/about").unwrap();
        let outcome = dispatch_url_into_runtime(&url, &mut rt, Some("demo"));
        assert_eq!(outcome, DispatchOutcome::Applied { query_writes: 0 });
        assert_eq!(router.snapshot(), vec!["/", "/about"]);
    }
}
