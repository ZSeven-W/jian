//! ASP verb tagged enum (Plan 18 Task 1).
//!
//! `Verb` is a `#[serde(tag = "verb", rename_all = "snake_case")]`
//! enum so each variant deserialises from a JSON object whose `"verb"`
//! field selects the variant and the remaining fields populate the
//! variant's struct payload. `Request` flattens this onto the
//! envelope so the wire shape is
//! `{"id": …, "verb": "tap", "selector": …}` rather than nested.

use serde::{Deserialize, Serialize};

use crate::selector::Selector;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum Verb {
    /// First message on every session. `token` is the per-session
    /// secret the host bootstrap step issued; `client` / `version`
    /// are diagnostic strings logged in the audit ring.
    Handshake {
        token: String,
        client: String,
        version: String,
    },

    /// Resolve a selector and return the matching nodes.
    Find {
        selector: Selector,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },

    /// Synthesise a tap (PointerDown + PointerUp inside the matched
    /// node's rect).
    Tap { selector: Selector },

    /// Synthesise text input. `clear: true` first deletes the
    /// existing value so the new `text` lands in an empty field.
    Type {
        selector: Selector,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clear: Option<bool>,
    },

    /// Wheel-style scroll within the matched node.
    Scroll {
        selector: Selector,
        direction: ScrollDir,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        distance: Option<f32>,
    },

    /// Pointer swipe (touch-style flick) — same shape as Scroll but
    /// dispatched as a pointer drag rather than a wheel event.
    Swipe {
        selector: Selector,
        direction: ScrollDir,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        distance: Option<f32>,
    },

    /// Push / replace / pop / reset the route stack. `mode` defaults
    /// to `push` when omitted.
    Navigate {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<NavMode>,
    },

    /// Block until `expr` evaluates truthy or `timeout_ms` elapses.
    /// Evaluation happens in the runtime's expression sandbox.
    WaitFor {
        expr: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },

    /// Evaluate `expr` and fail the verb if it isn't truthy.
    Assert { expr: String },

    /// Read structured information from the runtime. Cheaper than
    /// `Snapshot` and intended for the agent's per-step planning loop.
    Inspect {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selector: Option<Selector>,
        what: InspectKind,
    },

    /// Capture a richer view of the running app. Heavy — agents
    /// should reach for this only when `Inspect` doesn't fit.
    Snapshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<SnapshotFormat>,
    },

    /// Direct state-graph write. Restricted to the `Full` permission
    /// tier per Plan 18 Task 6 (`session.rs`). `value_json` is the
    /// raw JSON-text value, parsed and validated by the runtime
    /// against the path's declared schema.
    SetState {
        scope: String,
        key: String,
        value_json: String,
    },

    /// Replay the last `last_n` audit-ring entries (default: 32).
    Audit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_n: Option<u32>,
    },

    /// Tear down the session cleanly. The server sends a final
    /// `Response` and closes the transport.
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDir {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NavMode {
    Push,
    Replace,
    Pop,
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectKind {
    /// Just the matched scope's state key-value.
    State,
    /// Just the visible props of the matched node.
    NodeProps,
    /// Compressed accessibility-tree text.
    AxTree,
    /// Current path + route stack depth.
    Route,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotFormat {
    /// Base64-encoded PNG of the current frame. Heavy; agents that
    /// can read PNGs end-to-end choose this when other inspect kinds
    /// don't fit.
    PngBase64,
    /// Indented text rendering of the visible scene tree. Cheap and
    /// LLM-friendly when image input isn't available.
    TextTree,
    /// No payload — the agent only needed the verb to fire (e.g. as
    /// a synchronisation point).
    None,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selector::Selector;

    #[test]
    fn tap_verb_round_trips() {
        let v: Verb =
            serde_json::from_str(r#"{"verb":"tap","selector":{"role":"button","text":"Save"}}"#)
                .unwrap();
        match v {
            Verb::Tap { selector } => {
                assert_eq!(selector.role.as_deref(), Some("button"));
                assert_eq!(selector.text.as_deref(), Some("Save"));
            }
            other => panic!("expected Tap, got {:?}", other),
        }
    }

    #[test]
    fn type_verb_optional_clear_omits_when_none() {
        // `Some(true)` round-trips; `None` is omitted from the JSON.
        let with_clear = Verb::Type {
            selector: Selector::default(),
            text: "hi".into(),
            clear: Some(true),
        };
        let json = serde_json::to_string(&with_clear).unwrap();
        assert!(json.contains(r#""clear":true"#));

        let no_clear = Verb::Type {
            selector: Selector::default(),
            text: "hi".into(),
            clear: None,
        };
        let json = serde_json::to_string(&no_clear).unwrap();
        assert!(!json.contains("clear"));
    }

    #[test]
    fn snapshot_format_variants_serialise_snake_case() {
        let png = serde_json::to_string(&SnapshotFormat::PngBase64).unwrap();
        assert_eq!(png, "\"png_base64\"");
        let tree = serde_json::to_string(&SnapshotFormat::TextTree).unwrap();
        assert_eq!(tree, "\"text_tree\"");
    }

    #[test]
    fn unknown_verb_rejected() {
        // The tag enum's `#[serde(tag = "verb")]` rejects unknown
        // discriminators — better than silently ignoring, because a
        // typo'd verb is almost always agent-side bug, not a forward-
        // compat extension.
        let bad = serde_json::from_str::<Verb>(r#"{"verb":"made_up"}"#);
        assert!(bad.is_err());
    }

    #[test]
    fn exit_verb_has_no_payload() {
        let v: Verb = serde_json::from_str(r#"{"verb":"exit"}"#).unwrap();
        assert!(matches!(v, Verb::Exit));
    }
}
