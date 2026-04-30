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
//!    [`session::TokenValidator`]. On success, build a
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

use crate::protocol::{OutcomePayload, Request, Response, Verb};
use crate::session::{Session, TokenValidator};
use crate::transport::{Transport, TransportError};
use crate::verb_impls::{dispatch, verb_name, DispatchControl};
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
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::Transport(e) => write!(f, "transport: {}", e),
            ServerError::BadHandshake(m) => write!(f, "bad handshake: {}", m),
            ServerError::AuthFailed(m) => write!(f, "auth failed: {}", m),
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
    let req: Request = serde_json::from_str(&line).map_err(|e| {
        ServerError::BadHandshake(format!("first line is not a Request: {}", e))
    })?;
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
                let payload = OutcomePayload::invalid(
                    "request",
                    &format!("could not parse request: {}", e),
                );
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
    use std::io::{Cursor, Write};
    use std::rc::Rc;
    use std::cell::RefCell;

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
        let err = run_session(&mut transport, &validator, &mut runtime, Instant::now())
            .unwrap_err();
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
        let err = run_session(&mut transport, &validator, &mut runtime, Instant::now())
            .unwrap_err();
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
}
