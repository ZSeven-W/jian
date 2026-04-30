//! `Request` / `Response` envelope types.
//!
//! Both ride on a single NDJSON line. `Request.verb` is the tagged
//! enum from `verbs.rs` (`#[serde(tag = "verb")]`), so the discriminator
//! lives at the top level alongside `id`. `Response` is intentionally
//! flat so an LLM-driven agent can read the outcome with one look at
//! the line.

use serde::{Deserialize, Serialize};

use super::verbs::Verb;

/// One inbound request line. `id` is monotonic-per-client and the
/// `Response` echoes it back so the agent can match request and
/// reply when several are in flight on the same connection.
///
/// `serde(flatten)` lifts the `Verb` enum's tagged fields onto the
/// top-level object, producing `{"id":1,"verb":"tap","selector":{…}}`
/// rather than the deeper `{"id":1,"verb":{"verb":"tap","selector":…}}`.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub verb: Verb,
}

/// Response envelope. `body` carries the rendered payload — JSON or
/// Markdown — depending on the session's chosen `format`. The session
/// (Plan 18 Task 6) negotiates that during handshake; the protocol
/// layer treats `body` as opaque text, with `\n` characters left as
/// JSON-string escapes per spec. `ok` duplicates `OutcomePayload.ok`
/// so an agent can branch on success without re-parsing `body`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    pub ok: bool,
    pub body: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::verbs::Verb;

    #[test]
    fn request_with_tap_verb_round_trips() {
        let json = r#"{"id":1,"verb":"tap","selector":{"role":"button","text":"Submit"}}"#;
        let req: Request = serde_json::from_str(json).unwrap();
        assert_eq!(req.id, 1);
        assert!(matches!(req.verb, Verb::Tap { .. }));
    }

    #[test]
    fn request_with_handshake_verb_carries_token() {
        let json = r#"{"id":0,"verb":"handshake","token":"abc","client":"agent","version":"0.1"}"#;
        let req: Request = serde_json::from_str(json).unwrap();
        match req.verb {
            Verb::Handshake {
                token,
                client,
                version,
            } => {
                assert_eq!(token, "abc");
                assert_eq!(client, "agent");
                assert_eq!(version, "0.1");
            }
            other => panic!("expected Handshake, got {:?}", other),
        }
    }

    #[test]
    fn response_serialises_as_one_line() {
        let resp = Response {
            id: 7,
            ok: true,
            body: r#"{"verb":"tap","ok":true}"#.into(),
        };
        let line = serde_json::to_string(&resp).unwrap();
        assert!(!line.contains('\n'));
        // `body` is just a string, so JSON escapes its inner quotes.
        assert!(line.contains(r#""body":"{\"verb\":\"tap\",\"ok\":true}""#));
    }
}
