//! Deep-link / file-association abstractions (Plan 8 Task 8 scaffolding).
//!
//! The concrete platform backends — macOS `CFBundleURLTypes` +
//! `application_open_urls`, Windows registry + `WM_COPYDATA` relay,
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
    /// form returned by [`JianUrl::Display`]:
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
/// `WM_COPYDATA`, Linux MIME activation) do not consume an
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
}
