//! ASP server main loop (Plan 18 Task 7).
//!
//! Pulls together the four supporting pieces — `protocol::Request`
//! parsing, `transport::Transport` line I/O, `session::Session` +
//! token validation, `verb_impls::dispatch` — into one
//! `run_session` entry point a host can call from a worker
//! thread.
//!
//! Lifecycle:
//! 1. Read the first line and parse it as a `Verb::Handshake`.
//!    Anything else returns `Err` immediately so the host knows
//!    the agent is misbehaving.
//! 2. Validate the token via the host-supplied
//!    [`crate::session::TokenValidator`]. On success, build a
//!    [`Session`] with the granted permission tier; on failure,
//!    write a denied-response line and return.
//! 3. Loop: read a line → parse a `Request` → dispatch →
//!    write the response → record one audit entry. Quit on
//!    `Verb::Exit` or transport EOF.
//!
//! The function is fully synchronous and takes `&mut Runtime` so
//! it can be called from a host's worker thread that owns the
//! runtime borrow for the duration of the session. Hosts that
//! want to share a runtime across threads pair this with their
//! own locking.

use std::time::Instant;

use crate::bridge::AspBridge;
use crate::protocol::{OutcomePayload, Request, Response, Verb};
use crate::session::{Session, TokenValidator};
use crate::transport::{Transport, TransportError};
use crate::verb_impls::{dispatch, dispatch_with_mode, verb_name, DispatchControl, Mode};
use jian_core::Runtime;

/// Top-level error type — a real I/O failure or a malformed
/// handshake. Per-verb invalid input flows through
/// `OutcomePayload::invalid` on the wire and never reaches this
/// type.
#[derive(Debug)]
pub enum ServerError {
    /// Transport read / write failed (peer disconnect, broken
    /// pipe, etc).
    Transport(TransportError),
    /// First line wasn't a parseable `handshake` request.
    BadHandshake(String),
    /// Validator rejected the handshake's token.
    AuthFailed(String),
    /// Prod session refused to start because the runtime's loaded
    /// document has no `app.capabilities` declared (or the field
    /// exists but is empty). Spec §4 / Plan 18 ASP prod mode (C3b):
    /// prod ASP requires the author to have opted into machine-
    /// readable automation by populating capabilities. Apps that
    /// haven't opted in stay closed to prod ASP.
    ProdCapabilitiesEmpty,
    /// Prod session refused to start because the runtime had no
    /// document loaded at all. Caller should `Runtime::load_str`
    /// before invoking [`run_prod_session`].
    ProdNoDocument,
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::Transport(e) => write!(f, "transport: {}", e),
            ServerError::BadHandshake(m) => write!(f, "bad handshake: {}", m),
            ServerError::AuthFailed(m) => write!(f, "auth failed: {}", m),
            ServerError::ProdCapabilitiesEmpty => write!(
                f,
                "prod ASP refused to start: app.capabilities is empty or absent \
                 (spec §4 — author must opt in to machine-readable automation)"
            ),
            ServerError::ProdNoDocument => write!(
                f,
                "prod ASP refused to start: no document loaded in the runtime"
            ),
        }
    }
}

impl std::error::Error for ServerError {}

impl From<TransportError> for ServerError {
    fn from(e: TransportError) -> Self {
        ServerError::Transport(e)
    }
}

/// Run one ASP session over `transport` against `runtime`. Blocks
/// the calling thread until the session ends (clean exit, peer
/// EOF, or unrecoverable error).
///
/// `start` is the timestamp the caller picked as t=0 — usually
/// `Instant::now()` right before the call. Audit entries record
/// `at_ms` relative to it so the agent's `audit` payload is
/// session-relative regardless of how long the host has been up.
pub fn run_session(
    transport: &mut dyn Transport,
    validator: &dyn TokenValidator,
    runtime: &mut Runtime,
    start: Instant,
) -> Result<(), ServerError> {
    // 1. Handshake.
    let line = transport.read_line()?;
    let req: Request = serde_json::from_str(&line)
        .map_err(|e| ServerError::BadHandshake(format!("first line is not a Request: {}", e)))?;
    let (token, client, version) = match req.verb {
        Verb::Handshake {
            token,
            client,
            version,
        } => (token, client, version),
        other => {
            return Err(ServerError::BadHandshake(format!(
                "first verb must be `handshake`, got `{}`",
                verb_name(&other)
            )))
        }
    };
    let permission = match validator.validate(&token) {
        Ok(p) => p,
        Err(reason) => {
            // Write back a denied response so the agent sees the
            // rejection, then end. Failure to write is non-fatal
            // here — the peer probably already hung up.
            let payload = OutcomePayload::denied(
                "handshake",
                reason,
                Some("re-handshake with a token granting the required tier"),
            );
            let _ = write_response(transport, req.id, &payload);
            return Err(ServerError::AuthFailed(reason.to_owned()));
        }
    };
    let mut session = Session::new(permission, client, version);
    let ack = OutcomePayload::ok(
        "handshake",
        None,
        format!("handshake ok, permission={:?}", permission),
    );
    write_response(transport, req.id, &ack)?;
    session.record_outcome(start.elapsed().as_millis() as u64, &ack);

    // 2. Steady state — request → dispatch → response loop.
    loop {
        let line = match transport.read_line() {
            Ok(s) => s,
            // Clean peer-close ends the session normally.
            Err(TransportError::Eof) => return Ok(()),
            Err(e) => return Err(ServerError::Transport(e)),
        };
        if line.is_empty() {
            // Skip blank lines so a peer can heartbeat by sending
            // `\n`. The cost is one allocation per blank, which
            // is fine for a debugging channel.
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let payload =
                    OutcomePayload::invalid("request", &format!("could not parse request: {}", e));
                // Use id=0 because we couldn't read the agent's
                // intended id. Agents typically log the parse
                // failure and resync from the next line.
                write_response(transport, 0, &payload)?;
                session.record_outcome(start.elapsed().as_millis() as u64, &payload);
                continue;
            }
        };
        let (payload, control) = dispatch(&req.verb, runtime, &mut session);
        write_response(transport, req.id, &payload)?;
        session.record_outcome(start.elapsed().as_millis() as u64, &payload);
        if control == DispatchControl::Exit {
            return Ok(());
        }
    }
}

/// Run one **production-mode** ASP session over `transport` against
/// `runtime` (Plan 18 ASP prod mode / C3b).
///
/// Same lifecycle as [`run_session`] but with two prod-specific
/// guards before the steady-state loop spins:
///
/// 1. **Document required** — `runtime.document` must be `Some`. A
///    fresh `Runtime::new()` with no document loaded gets
///    `ServerError::ProdNoDocument`. (`run_session` allows this for
///    debug agents that attach a document mid-session via
///    `set_state`; prod ASP doesn't expose `set_state` so the
///    pre-condition is mandatory here.)
/// 2. **Non-empty capabilities** — `runtime.document.schema.app.capabilities`
///    must be `Some(non-empty)`. Apps that haven't opted into
///    machine-readable automation stay closed to prod ASP per spec
///    §4. `ServerError::ProdCapabilitiesEmpty` on violation.
///
/// Once both guards pass, the loop dispatches every verb through
/// [`dispatch_with_mode`] with [`Mode::Prod`]. Structural verbs
/// (`find` / `inspect` / `snapshot` / `audit` / `wait_for` /
/// `assert` / `navigate` / `set_state`) reject with
/// `OutcomePayload::unsupported_verb_in_prod` (stable error tag
/// `UnsupportedVerbInProd`); the session stays open so the agent
/// can recover with an allowed verb.
///
/// Token validation is the host's responsibility — pass a
/// real [`TokenValidator`] that actually checks the token. The
/// session module deliberately doesn't bake one in (different
/// hosts use different bootstrap channels: file / Keychain /
/// Keystore / postMessage). The server can't structurally prove
/// the validator isn't a no-op stub; that's a host-side contract.
pub fn run_prod_session(
    transport: &mut dyn Transport,
    validator: &dyn TokenValidator,
    runtime: &mut Runtime,
    start: Instant,
) -> Result<(), ServerError> {
    // Prod-mode preconditions BEFORE we read anything off the
    // transport. A misconfigured host should fail closed at boot,
    // not after the agent has already sent its handshake.
    let doc = runtime
        .document
        .as_ref()
        .ok_or(ServerError::ProdNoDocument)?;
    let capabilities_ok = doc
        .schema
        .app
        .as_ref()
        .and_then(|a| a.capabilities.as_ref())
        .is_some_and(|caps| !caps.is_empty());
    if !capabilities_ok {
        return Err(ServerError::ProdCapabilitiesEmpty);
    }

    // 1. Handshake — same parser as run_session.
    let line = transport.read_line()?;
    let req: Request = serde_json::from_str(&line)
        .map_err(|e| ServerError::BadHandshake(format!("first line is not a Request: {}", e)))?;
    let (token, client, version) = match req.verb {
        Verb::Handshake {
            token,
            client,
            version,
        } => (token, client, version),
        other => {
            return Err(ServerError::BadHandshake(format!(
                "first verb must be `handshake`, got `{}`",
                verb_name(&other)
            )))
        }
    };
    let permission = match validator.validate(&token) {
        Ok(p) => p,
        Err(reason) => {
            let payload = OutcomePayload::denied(
                "handshake",
                reason,
                Some("re-handshake with a token granting the required tier"),
            );
            let _ = write_response(transport, req.id, &payload);
            return Err(ServerError::AuthFailed(reason.to_owned()));
        }
    };
    let mut session = Session::new(permission, client, version);
    let ack = OutcomePayload::ok(
        "handshake",
        None,
        format!("handshake ok (prod), permission={:?}", permission),
    );
    write_response(transport, req.id, &ack)?;
    session.record_outcome(start.elapsed().as_millis() as u64, &ack);

    // 2. Steady state — every verb routes through `dispatch_with_mode(Mode::Prod)`.
    loop {
        let line = match transport.read_line() {
            Ok(s) => s,
            Err(TransportError::Eof) => return Ok(()),
            Err(e) => return Err(ServerError::Transport(e)),
        };
        if line.is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let payload =
                    OutcomePayload::invalid("request", &format!("could not parse request: {}", e));
                write_response(transport, 0, &payload)?;
                session.record_outcome(start.elapsed().as_millis() as u64, &payload);
                continue;
            }
        };
        let (payload, control) = dispatch_with_mode(&req.verb, runtime, &mut session, Mode::Prod);
        write_response(transport, req.id, &payload)?;
        session.record_outcome(start.elapsed().as_millis() as u64, &payload);
        if control == DispatchControl::Exit {
            return Ok(());
        }
    }
}

/// Run one prod ASP session whose dispatch is ferried over an
/// [`AspBridge`] (Plan 18 ASP prod mode / C4 follow-up).
///
/// Identical lifecycle to [`run_prod_session`] except every verb
/// (post-handshake) is sent to the runtime thread via the bridge
/// instead of being dispatched against a borrowed `&mut Runtime`
/// here. Use this entry point when the runtime lives on a different
/// thread than the transport listener — e.g. `jian player --asp`,
/// where winit owns the runtime.
///
/// Pre-conditions:
/// - The host's runtime side must call [`crate::bridge::AspDrain::try_recv`]
///   in its event loop, dispatch each request via
///   [`dispatch_with_mode`] with [`Mode::Prod`], and reply through
///   the request's `reply` channel. The
///   `jian_host_desktop::run::about_to_wait` hook does this when
///   `with_asp` is wired.
/// - **No document/capability check is performed here.** The caller
///   is expected to gate the listener by verifying both BEFORE
///   binding (the `jian player --asp` command refuses to start the
///   listener thread when the loaded document lacks
///   `app.capabilities`). Splitting the check out of this function
///   keeps the bridge variant runtime-borrow-free.
pub fn run_prod_session_via_bridge(
    transport: &mut dyn Transport,
    validator: &dyn TokenValidator,
    bridge: &AspBridge,
    // `_start` carries the `Instant::now()` the caller picked as t=0.
    // The bridge variant doesn't keep a listener-side audit ring (the
    // host's `asp_session` is the canonical one — see codex C4
    // follow-up review LOW #7), so the timestamp is unused here. We
    // keep the parameter so the function signature stays
    // structurally identical to `run_prod_session`; a future
    // revision that adds listener-side telemetry can read it without
    // breaking call sites.
    _start: Instant,
) -> Result<(), ServerError> {
    // 1. Handshake — same parser as run_prod_session. Token
    //    validation runs locally so the bridge round-trip is
    //    skipped for the auth gate.
    let line = transport.read_line()?;
    let req: Request = serde_json::from_str(&line)
        .map_err(|e| ServerError::BadHandshake(format!("first line is not a Request: {}", e)))?;
    let (token, client, version) = match req.verb {
        Verb::Handshake {
            token,
            client,
            version,
        } => (token, client, version),
        other => {
            return Err(ServerError::BadHandshake(format!(
                "first verb must be `handshake`, got `{}`",
                verb_name(&other)
            )))
        }
    };
    let permission = match validator.validate(&token) {
        Ok(p) => p,
        Err(reason) => {
            let payload = OutcomePayload::denied(
                "handshake",
                reason,
                Some("re-handshake with a token granting the required tier"),
            );
            let _ = write_response(transport, req.id, &payload);
            return Err(ServerError::AuthFailed(reason.to_owned()));
        }
    };
    // The bridge variant deliberately *doesn't* maintain its own
    // `Session` post-handshake. Audit accounting is the runtime
    // thread's responsibility: the host's `with_asp(...)` installs
    // a long-lived `Session` that `drain_asp_requests` records
    // outcomes onto, and that's the audit ring an operator
    // post-mortem reads. A second listener-side ring would be a
    // duplicate that's discarded when this function returns
    // (codex C4 follow-up round 1, LOW #7).
    //
    // We keep the locals `permission` / `client` / `version` for
    // the handshake ack narrative + future telemetry hooks; if a
    // future revision needs listener-side audit, build a `Session`
    // here with a clear contract about which side owns the ring.
    let _ = (client, version);
    let ack = OutcomePayload::ok(
        "handshake",
        None,
        format!("handshake ok (prod-bridge), permission={:?}", permission),
    );
    write_response(transport, req.id, &ack)?;

    // 2. Steady state — read line → parse → bridge → write reply.
    loop {
        let line = match transport.read_line() {
            Ok(s) => s,
            Err(TransportError::Eof) => return Ok(()),
            Err(e) => return Err(ServerError::Transport(e)),
        };
        if line.is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let payload =
                    OutcomePayload::invalid("request", &format!("could not parse request: {}", e));
                write_response(transport, 0, &payload)?;
                continue;
            }
        };
        // Send to the runtime thread; block on the reply. `None`
        // means the host dropped its drain (event loop quit) — we
        // tear the session down so the agent learns the upstream
        // is gone.
        let resp = match bridge.dispatch_blocking(req.verb) {
            Some(r) => r,
            None => {
                let payload = OutcomePayload::error(
                    "session",
                    "runtime bridge closed (host event loop exited)",
                );
                let _ = write_response(transport, req.id, &payload);
                return Ok(());
            }
        };
        write_response(transport, req.id, &resp.payload)?;
        if resp.control == DispatchControl::Exit {
            return Ok(());
        }
    }
}

fn write_response(
    transport: &mut dyn Transport,
    id: u64,
    payload: &OutcomePayload,
) -> Result<(), TransportError> {
    let body = serde_json::to_string(payload).unwrap_or_else(|_| "{\"ok\":false}".to_owned());
    let resp = Response {
        id,
        ok: payload.ok,
        body,
    };
    let line = serde_json::to_string(&resp).unwrap_or_else(|_| String::from("{}"));
    transport.write_line(&line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Permission, StaticTokenValidator};
    use crate::transport::stdio::StdioTransport;
    use jian_ops_schema::document::PenDocument;
    use std::cell::RefCell;
    use std::io::{Cursor, Write};
    use std::rc::Rc;

    /// In-memory `Write` impl that captures bytes into a shared
    /// `Rc<RefCell<Vec<u8>>>` so the test inspects what the
    /// server wrote.
    struct SharedWriter(Rc<RefCell<Vec<u8>>>);
    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn rig(input: &str) -> (StdioTransport, Rc<RefCell<Vec<u8>>>) {
        let cursor = Cursor::new(input.as_bytes().to_vec());
        let out = Rc::new(RefCell::new(Vec::new()));
        let writer: Box<dyn Write> = Box::new(SharedWriter(out.clone()));
        (StdioTransport::from_streams(cursor, writer), out)
    }

    fn make_runtime() -> Runtime {
        let doc_json = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,"children":[] }
          ]
        }"##;
        let schema: PenDocument = jian_ops_schema::load_str(doc_json).unwrap().value;
        let mut rt = Runtime::new_from_document(schema).unwrap();
        rt.build_layout((480.0, 320.0)).unwrap();
        rt.rebuild_spatial();
        rt
    }

    fn read_lines(out: &Rc<RefCell<Vec<u8>>>) -> Vec<String> {
        let bytes = out.borrow().clone();
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn handshake_then_exit_runs_clean() {
        let input = r#"{"id":1,"verb":"handshake","token":"secret","client":"agent","version":"0.1"}
{"id":2,"verb":"exit"}
"#;
        let (mut transport, out) = rig(input);
        let validator = StaticTokenValidator::new("secret", Permission::Observe);
        let mut runtime = make_runtime();
        run_session(&mut transport, &validator, &mut runtime, Instant::now()).unwrap();
        let lines = read_lines(&out);
        assert_eq!(lines.len(), 2);
        // Handshake ack: ok=true, body has the OutcomePayload.
        let resp1: Response = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(resp1.id, 1);
        assert!(resp1.ok);
        let resp2: Response = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(resp2.id, 2);
        assert!(resp2.ok);
    }

    #[test]
    fn bad_token_returns_auth_failed_and_writes_denied() {
        let input = r#"{"id":1,"verb":"handshake","token":"wrong","client":"agent","version":"0.1"}
"#;
        let (mut transport, out) = rig(input);
        let validator = StaticTokenValidator::new("right", Permission::Observe);
        let mut runtime = make_runtime();
        let err =
            run_session(&mut transport, &validator, &mut runtime, Instant::now()).unwrap_err();
        assert!(matches!(err, ServerError::AuthFailed(_)));
        let lines = read_lines(&out);
        // One denied response was written before bailing.
        assert_eq!(lines.len(), 1);
        let resp: Response = serde_json::from_str(&lines[0]).unwrap();
        assert!(!resp.ok);
    }

    #[test]
    fn first_message_must_be_handshake() {
        let input = r#"{"id":1,"verb":"exit"}
"#;
        let (mut transport, _out) = rig(input);
        let validator = StaticTokenValidator::new("secret", Permission::Observe);
        let mut runtime = make_runtime();
        let err =
            run_session(&mut transport, &validator, &mut runtime, Instant::now()).unwrap_err();
        assert!(
            matches!(err, ServerError::BadHandshake(_)),
            "expected BadHandshake, got {:?}",
            err
        );
    }

    #[test]
    fn malformed_request_emits_invalid_response_and_continues() {
        let input = r#"{"id":1,"verb":"handshake","token":"secret","client":"agent","version":"0.1"}
not-json
{"id":3,"verb":"exit"}
"#;
        let (mut transport, out) = rig(input);
        let validator = StaticTokenValidator::new("secret", Permission::Observe);
        let mut runtime = make_runtime();
        run_session(&mut transport, &validator, &mut runtime, Instant::now()).unwrap();
        let lines = read_lines(&out);
        // 1 handshake ack + 1 invalid response (id=0) + 1 exit ack.
        assert_eq!(lines.len(), 3);
        let invalid: Response = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(invalid.id, 0);
        assert!(!invalid.ok);
    }

    #[test]
    fn blank_lines_are_skipped() {
        let input = "{\"id\":1,\"verb\":\"handshake\",\"token\":\"s\",\"client\":\"a\",\"version\":\"0.1\"}\n\n\n{\"id\":2,\"verb\":\"exit\"}\n";
        let (mut transport, out) = rig(input);
        let validator = StaticTokenValidator::new("s", Permission::Observe);
        let mut runtime = make_runtime();
        run_session(&mut transport, &validator, &mut runtime, Instant::now()).unwrap();
        let lines = read_lines(&out);
        // Exactly two responses: handshake ack + exit ack.
        assert_eq!(lines.len(), 2);
    }

    // ──────────────────────────────────────────────────────────────
    // C3b — run_prod_session preconditions + dispatch routing
    // ──────────────────────────────────────────────────────────────

    fn make_runtime_with_capabilities(caps_json: &str) -> Runtime {
        let doc_json = format!(
            r##"{{
              "formatVersion":"1.0","version":"1.0.0","id":"x",
              "app":{{"name":"x","version":"1","id":"x","capabilities":{caps_json}}},
              "children":[
                {{ "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
                "children":[
                  {{ "type":"rectangle","id":"save-btn","x":0,"y":0,"width":50,"height":20,
                    "events": {{ "onTap": [{{ "set": {{ "$state.x": "1" }} }}] }} }}
                ]}}
              ],
              "state":{{"x":{{"type":"int","default":0}}}}
            }}"##
        );
        let schema: PenDocument = jian_ops_schema::load_str(&doc_json).unwrap().value;
        let mut rt = Runtime::new_from_document(schema).unwrap();
        rt.build_layout((480.0, 320.0)).unwrap();
        rt.rebuild_spatial();
        rt
    }

    #[test]
    fn run_prod_session_refuses_when_capabilities_absent() {
        // The default `make_runtime` fixture has no capabilities at
        // all. Prod must fail closed before reading the transport.
        let input = "";
        let (mut transport, _out) = rig(input);
        let validator = StaticTokenValidator::new("s", Permission::Act);
        let mut runtime = make_runtime();
        let err =
            run_prod_session(&mut transport, &validator, &mut runtime, Instant::now()).unwrap_err();
        assert!(matches!(err, ServerError::ProdCapabilitiesEmpty));
    }

    #[test]
    fn run_prod_session_refuses_when_capabilities_empty_array() {
        // Author wrote `app.capabilities: []` — explicit empty.
        // Same outcome as absent: refusal.
        let input = "";
        let (mut transport, _out) = rig(input);
        let validator = StaticTokenValidator::new("s", Permission::Act);
        let mut runtime = make_runtime_with_capabilities("[]");
        let err =
            run_prod_session(&mut transport, &validator, &mut runtime, Instant::now()).unwrap_err();
        assert!(matches!(err, ServerError::ProdCapabilitiesEmpty));
    }

    #[test]
    fn run_prod_session_refuses_when_no_document_loaded() {
        // Fresh runtime with no document at all → ProdNoDocument.
        let input = "";
        let (mut transport, _out) = rig(input);
        let validator = StaticTokenValidator::new("s", Permission::Act);
        let mut runtime = Runtime::new();
        let err =
            run_prod_session(&mut transport, &validator, &mut runtime, Instant::now()).unwrap_err();
        assert!(matches!(err, ServerError::ProdNoDocument));
    }

    #[test]
    fn run_prod_session_with_capabilities_allows_handshake_and_list_actions() {
        // Capabilities present → prod session starts. Agent calls
        // list_actions and receives the projected rows.
        let input = r#"{"id":1,"verb":"handshake","token":"s","client":"agent","version":"0.1"}
{"id":2,"verb":"list_actions"}
{"id":3,"verb":"exit"}
"#;
        let (mut transport, out) = rig(input);
        let validator = StaticTokenValidator::new("s", Permission::Act);
        let mut runtime = make_runtime_with_capabilities(r#"["network"]"#);
        run_prod_session(&mut transport, &validator, &mut runtime, Instant::now()).unwrap();
        let lines = read_lines(&out);
        assert_eq!(lines.len(), 3);
        // Handshake ack + list_actions response + exit ack.
        let r2: Response = serde_json::from_str(&lines[1]).unwrap();
        assert!(r2.ok, "list_actions should succeed in prod");
        let payload: serde_json::Value = serde_json::from_str(&r2.body).unwrap();
        assert_eq!(payload["verb"], "list_actions");
    }

    #[test]
    fn run_prod_session_rejects_structural_verbs_with_unsupported_tag() {
        // Structural verb (snapshot) under prod-mode dispatch → the
        // `UnsupportedVerbInProd` error tag travels back to the client.
        // The session stays open (the `exit` after still gets a
        // response).
        let input = r#"{"id":1,"verb":"handshake","token":"s","client":"agent","version":"0.1"}
{"id":2,"verb":"snapshot"}
{"id":3,"verb":"exit"}
"#;
        let (mut transport, out) = rig(input);
        let validator = StaticTokenValidator::new("s", Permission::Act);
        let mut runtime = make_runtime_with_capabilities(r#"["network"]"#);
        run_prod_session(&mut transport, &validator, &mut runtime, Instant::now()).unwrap();
        let lines = read_lines(&out);
        assert_eq!(lines.len(), 3);
        let r2: Response = serde_json::from_str(&lines[1]).unwrap();
        assert!(!r2.ok);
        let payload: serde_json::Value = serde_json::from_str(&r2.body).unwrap();
        assert_eq!(payload["error"], "UnsupportedVerbInProd");
    }

    // C4 follow-up — `run_prod_session_via_bridge` round-trip.
    //
    // The bridge tests run the session on a *separate thread* so we
    // can drain dispatch requests from the "main thread" (the test
    // body). `StdioTransport`'s `Box<dyn BufRead>` / `Box<dyn Write>`
    // are not `Send`, so we use a tiny mpsc-backed `ChannelTransport`
    // instead — naturally `Send` because every captured field is
    // `Send`.

    use std::sync::mpsc;

    /// Test-only Send transport. `read_line` pulls strings from a
    /// `Receiver<String>`; `write_line` pushes into a
    /// `Sender<String>`. The closed-channel side surfaces as
    /// `TransportError::Eof` so the session loop exits cleanly.
    struct ChannelTransport {
        reader: mpsc::Receiver<String>,
        writer: mpsc::Sender<String>,
    }

    impl Transport for ChannelTransport {
        fn read_line(&mut self) -> Result<String, TransportError> {
            self.reader
                .recv()
                .map_err(|_| TransportError::Eof)
        }
        fn write_line(&mut self, line: &str) -> Result<(), TransportError> {
            self.writer
                .send(line.to_owned())
                .map_err(|e| TransportError::Io(format!("channel send: {}", e)))
        }
    }

    /// Build a `ChannelTransport` plus the test-side handles. The
    /// test feeds request lines through `request_tx` and receives
    /// the session's responses on `response_rx`. Closing
    /// `request_tx` (drop) signals EOF and ends the session loop.
    fn channel_rig() -> (ChannelTransport, mpsc::Sender<String>, mpsc::Receiver<String>) {
        let (req_tx, req_rx) = mpsc::channel::<String>();
        let (resp_tx, resp_rx) = mpsc::channel::<String>();
        let transport = ChannelTransport {
            reader: req_rx,
            writer: resp_tx,
        };
        (transport, req_tx, resp_rx)
    }

    #[test]
    fn run_prod_session_via_bridge_round_trips_handshake_and_exit() {
        // The transport carries handshake + exit; the bridge thread
        // drives the session, the "main thread" (this test) drains
        // the bridge once to dispatch `exit`. Pins the contract that
        // the bridge variant lifecycles cleanly without ever touching
        // a `Runtime` value.
        use crate::bridge::{channel, DispatchResponse};
        use std::thread;

        let (mut transport, req_tx, resp_rx) = channel_rig();
        req_tx
            .send(r#"{"id":1,"verb":"handshake","token":"s","client":"agent","version":"0.1"}"#.into())
            .unwrap();
        req_tx
            .send(r#"{"id":2,"verb":"exit"}"#.into())
            .unwrap();
        let (bridge, drain) = channel();
        let validator = StaticTokenValidator::new("s", Permission::Act);

        let session_thread = thread::spawn(move || {
            crate::server::run_prod_session_via_bridge(
                &mut transport,
                &validator,
                &bridge,
                Instant::now(),
            )
            .unwrap();
        });

        // Drain one request — the `exit` verb. Reply with a stock
        // exit outcome + Exit control. The handshake never reaches
        // the bridge because it's dispatched locally for the auth
        // check.
        let req = loop {
            if let Some(r) = drain.try_recv() {
                break r;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        assert!(matches!(req.verb, Verb::Exit));
        req.reply
            .send(DispatchResponse {
                payload: OutcomePayload::ok("exit", None, "session ended"),
                control: DispatchControl::Exit,
            })
            .unwrap();

        session_thread.join().expect("session thread");

        // Two written lines: handshake ack + exit response.
        let line1 = resp_rx.recv().expect("first response");
        let line2 = resp_rx.recv().expect("second response");
        let r1: Response = serde_json::from_str(&line1).unwrap();
        assert_eq!(r1.id, 1);
        assert!(r1.ok);
        let r2: Response = serde_json::from_str(&line2).unwrap();
        assert_eq!(r2.id, 2);
        assert!(r2.ok);
        // No further responses expected — drop is enough to close.
        drop(req_tx);
    }

    #[test]
    fn run_prod_session_via_bridge_tears_down_when_bridge_closes() {
        // If the host's event loop drops the drain mid-session, the
        // listener-side dispatch_blocking returns None — the session
        // surfaces that as a transport-level "runtime gone" error
        // and exits cleanly so the agent's transport sees the
        // session end.
        use crate::bridge::channel;
        use std::thread;

        let (mut transport, req_tx, resp_rx) = channel_rig();
        req_tx
            .send(r#"{"id":1,"verb":"handshake","token":"s","client":"agent","version":"0.1"}"#.into())
            .unwrap();
        req_tx
            .send(r#"{"id":2,"verb":"list_actions"}"#.into())
            .unwrap();
        let (bridge, drain) = channel();
        let validator = StaticTokenValidator::new("s", Permission::Observe);

        let session_thread = thread::spawn(move || {
            crate::server::run_prod_session_via_bridge(
                &mut transport,
                &validator,
                &bridge,
                Instant::now(),
            )
        });

        // Wait until the session has produced the handshake ack +
        // started waiting for the dispatch reply, then drop the
        // drain to simulate the host event loop quitting.
        let req = loop {
            if let Some(r) = drain.try_recv() {
                break r;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        // Drop the drain *and* the request handle (which holds the
        // reply sender) so dispatch_blocking sees a closed reply
        // channel.
        drop(drain);
        drop(req);

        let result = session_thread.join().unwrap();
        // Clean termination — the session interpreted the
        // closed-bridge as a teardown, not an error.
        assert!(result.is_ok(), "session should exit Ok on closed bridge");
        // It also wrote a session-level error response back to the
        // agent for the in-flight verb so the client can resync.
        let _ack = resp_rx.recv().expect("handshake ack");
        let last_line = resp_rx.recv().expect("closed-bridge response");
        let last: Response = serde_json::from_str(&last_line).unwrap();
        assert!(!last.ok);
        let body: serde_json::Value = serde_json::from_str(&last.body).unwrap();
        assert!(
            body["narrative"]
                .as_str()
                .map(|s| s.contains("runtime bridge closed"))
                .unwrap_or(false),
            "narrative should mention bridge close: {}",
            last.body
        );
        drop(req_tx);
    }
}
