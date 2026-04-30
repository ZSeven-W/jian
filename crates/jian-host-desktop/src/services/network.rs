//! `reqwest::blocking::Client`-backed `NetworkClient` for the desktop host.
//!
//! Gated behind the `network` cargo feature so headless / CI builds
//! don't pull in the reqwest dependency tree (rustls + hyper +
//! TLS roots). Hosts that expose the runtime's `http_request`
//! action wire one of these into `ActionContext` at startup:
//!
//! ```ignore
//! // jian-host-desktop = { features = ["network"] }
//! let net = std::rc::Rc::new(jian_host_desktop::services::network::DesktopNetworkClient::new());
//! ctx.network = net;
//! ```
//!
//! The client serialises bodies as JSON when `Some` and forwards the
//! schema's `headers` map verbatim. Errors stringify `reqwest::Error`
//! into the trait's `Result<_, String>` shape.
//!
//! ## How the blocking IO doesn't block the executor
//!
//! Each `request` call hands the work off to a fresh
//! `std::thread::spawn`, which runs `reqwest::blocking::Client::send()`
//! against the original `Client` (so the connection pool is shared
//! across calls — `reqwest::blocking::Client` is `Send + Sync` and
//! cheaply clonable). The trait method is `async`, so we return a
//! future that awaits a `futures_channel::oneshot::Receiver`; when
//! the worker thread sends its result, the executor wakes the task.
//! While the request is in flight the executor stays free to poll
//! other tasks, which is the contract every other async-trait
//! service in the runtime depends on.
//!
//! WebSocket (`connect_websocket`) intentionally returns the trait's
//! default `Err(...)` — the reqwest crate doesn't ship a WS client.
//! A follow-up plan can layer `tokio-tungstenite` on top.

use async_trait::async_trait;
use jian_core::action::services::network::{HttpRequest, HttpResponse, NetworkClient};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;

/// Hard upper bound on a single HTTP roundtrip baked into the
/// auto-built [`reqwest::blocking::Client`] inside
/// [`DesktopNetworkClient::new`]. Protects against worker-thread
/// leaks if the awaiting task gets cancelled mid-flight: dropping
/// the `oneshot::Receiver` doesn't kill the detached worker, but
/// the worker is guaranteed to finish (and the thread to be
/// reaped) within this many seconds because reqwest aborts the
/// call when the client's timeout elapses.
///
/// Hosts wiring a custom client via [`DesktopNetworkClient::with_client`]
/// take ownership of the timeout policy themselves — we deliberately
/// don't rewrite their config. Per-request `RequestBuilder::timeout`
/// overrides the client default, so schemas that need a longer
/// budget for one specific call set their own `timeout_ms` and the
/// per-request value wins.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// `reqwest`-backed network client. Single-threaded interior — clones
/// of `reqwest::blocking::Client` share a connection pool, so the
/// overhead of a per-call clone is negligible.
pub struct DesktopNetworkClient {
    client: reqwest::blocking::Client,
}

impl DesktopNetworkClient {
    /// Build with reqwest's defaults (rustls + system roots) plus a
    /// `DEFAULT_REQUEST_TIMEOUT` baked into the client so worker
    /// threads can't leak indefinitely on a hung server.
    pub fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(DEFAULT_REQUEST_TIMEOUT)
            .build()
            // Falling back to the no-timeout default is strictly
            // worse than panicking — `Client::builder().build()`
            // failing on a desktop host indicates a broken
            // rustls / DNS install that the user needs to know
            // about. Match the panicking behaviour of the previous
            // `Client::new()` form.
            .expect("reqwest blocking Client::builder().build() failed");
        Self { client }
    }

    /// Build from a pre-configured `Client`. Use when the host wants a
    /// custom timeout, proxy, or User-Agent. The host is responsible
    /// for any timeout policy on the supplied `Client`; this method
    /// does not impose `DEFAULT_REQUEST_TIMEOUT`.
    pub fn with_client(client: reqwest::blocking::Client) -> Self {
        Self { client }
    }
}

impl Default for DesktopNetworkClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait(?Send)]
impl NetworkClient for DesktopNetworkClient {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, String> {
        // Offload the blocking HTTP call to a fresh worker thread and
        // bridge the result back through a `oneshot` channel. The
        // executor stays free to poll other tasks while the future
        // awaits — this is the contract async-trait promises and the
        // earlier inline `request_blocking(...)` body broke.
        //
        // Cloning the `Client` is cheap (an Arc bump): reqwest's
        // connection pool lives behind it, so concurrent requests
        // share keep-alive connections.
        let client = self.client.clone();
        let (tx, rx) = futures_channel::oneshot::channel();
        std::thread::spawn(move || {
            // `tx.send` only fails if the receiver was dropped — the
            // task got cancelled before the response arrived. Drop
            // the result quietly in that case; nothing to report to.
            let _ = tx.send(request_blocking(&client, req));
        });
        match rx.await {
            Ok(r) => r,
            Err(_canceled) => Err("network worker thread dropped before sending a response".into()),
        }
    }
}

fn request_blocking(
    client: &reqwest::blocking::Client,
    req: HttpRequest,
) -> Result<HttpResponse, String> {
    let method = parse_method(&req.method)?;
    let mut builder = client.request(method, &req.url);
    for (k, v) in &req.headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    // Per-request override only when the schema spells out a value —
    // `RequestBuilder::timeout` overrides whatever the `Client` was
    // built with, so unconditionally setting it would clobber a
    // host-supplied custom timeout from `with_client`. The auto-built
    // client (`DesktopNetworkClient::new`) bakes
    // `DEFAULT_REQUEST_TIMEOUT` directly into the `Client`, so
    // unset-`timeout_ms` requests already inherit a bounded budget
    // from there.
    if let Some(ms) = req.timeout_ms {
        builder = builder.timeout(Duration::from_millis(ms));
    }
    if let Some(body) = req.body {
        builder = builder.json(&body);
    }
    let resp = builder.send().map_err(|e| format!("{}", e))?;

    let status = resp.status().as_u16();
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in resp.headers().iter() {
        if let Ok(s) = v.to_str() {
            headers.insert(k.as_str().to_owned(), s.to_owned());
        }
    }
    let bytes = resp.bytes().map_err(|e| format!("{}", e))?;
    // Try JSON; fall back to a string body when the server didn't send
    // valid JSON. The runtime's `Value` shape preserves either form so
    // `http_request` actions can route on `body.is_object()` etc.
    let body = serde_json::from_slice::<Value>(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));

    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

fn parse_method(s: &str) -> Result<reqwest::Method, String> {
    s.parse::<reqwest::Method>()
        .map_err(|_| format!("invalid HTTP method `{}`", s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_method_accepts_standard_verbs() {
        assert_eq!(parse_method("GET").unwrap(), reqwest::Method::GET);
        // `reqwest::Method::from_str` is case-sensitive — "post" parses
        // to a custom extension method, not POST. The runtime's wire
        // shape always sends uppercase verbs, so this is fine; the
        // test pins the contract so a future case-folding helper
        // doesn't silently change behaviour.
        assert_eq!(parse_method("POST").unwrap(), reqwest::Method::POST);
        assert_eq!(parse_method("DELETE").unwrap(), reqwest::Method::DELETE);
        assert_eq!(parse_method("PATCH").unwrap(), reqwest::Method::PATCH);
    }

    #[test]
    fn parse_method_rejects_garbage() {
        assert!(parse_method("not-a-method!@#").is_err());
    }
}
