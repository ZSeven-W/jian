//! Canvas comments — review threads pinned to a node or a canvas point.
//!
//! A document carries its comment threads in the top-level `comments`
//! array so the review history travels with the file: a designer can
//! leave feedback in a GUI, an agent can read it over MCP, change the
//! canvas, reply, and mark the thread addressed — and all of that is
//! still there after save / reload.
//!
//! The lifecycle is `pending → addressed → archived`, with `addressed →
//! pending` ("continue working") and `archived → pending` ("reopen").
//! Archived threads are kept for history; tools skip them by default.

use serde::{Deserialize, Serialize};

/// One comment thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct CanvasComment {
    /// Stable thread id, unique within the document.
    pub id: String,

    #[serde(default)]
    pub status: CommentStatus,

    pub anchor: CommentAnchor,

    /// The opening message followed by its replies, oldest first.
    #[serde(default)]
    pub messages: Vec<CommentMessage>,

    /// Unix epoch milliseconds of the last change (message or status).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "export-ts", ts(type = "number | null", optional = nullable))]
    pub updated_at: Option<u64>,
}

/// Where a thread is pinned.
///
/// With `nodeId`, the pin follows that node and `x` / `y` are an offset
/// from the node's top-left corner (absent ⇒ the node's top-right
/// corner). Without `nodeId`, `x` / `y` are absolute document
/// coordinates on `pageId` (absent ⇒ the document's first page).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct CommentAnchor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "export-ts", ts(optional = nullable))]
    pub page_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "export-ts", ts(optional = nullable))]
    pub node_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "export-ts", ts(optional = nullable))]
    pub x: Option<f64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "export-ts", ts(optional = nullable))]
    pub y: Option<f64>,
}

/// Review state of a thread.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub enum CommentStatus {
    /// Feedback waiting to be addressed.
    #[default]
    Pending,
    /// Changes or a reply have been provided.
    Addressed,
    /// Kept for history; skipped by agents unless history is requested.
    Archived,
}

impl CommentStatus {
    /// Wire name (`pending` / `addressed` / `archived`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Addressed => "addressed",
            Self::Archived => "archived",
        }
    }

    /// Parse a wire name; case-insensitive.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pending" => Some(Self::Pending),
            "addressed" => Some(Self::Addressed),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }

    /// Whether the lifecycle allows moving from `self` to `next`.
    ///
    /// `pending ↔ addressed`, `addressed → archived`, and
    /// `archived → pending` (reopen). Pending threads can also be archived
    /// directly so stale feedback can be filed away. Staying in the same
    /// state is not a transition.
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Pending, Self::Addressed)
                | (Self::Pending, Self::Archived)
                | (Self::Addressed, Self::Pending)
                | (Self::Addressed, Self::Archived)
                | (Self::Archived, Self::Pending)
        )
    }
}

/// One message in a thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct CommentMessage {
    pub author: CommentAuthor,
    pub text: String,
    /// Unix epoch milliseconds.
    #[serde(default)]
    #[cfg_attr(feature = "export-ts", ts(type = "number"))]
    pub created_at: u64,
}

/// Who wrote a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct CommentAuthor {
    #[serde(default)]
    pub kind: CommentAuthorKind,
    /// Display name — a person's name or an agent's name.
    #[serde(default)]
    pub name: String,
}

/// Whether a message came from a person (GUI) or an agent (MCP).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub enum CommentAuthorKind {
    #[default]
    User,
    Agent,
}

impl CommentAuthorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_roundtrips_with_camel_case_wire_names() {
        let raw = r#"{"id":"c1","status":"addressed","anchor":{"pageId":"p1","nodeId":"n1","x":4.0,"y":2.0},"messages":[{"author":{"kind":"agent","name":"Claude"},"text":"done","createdAt":5}],"updatedAt":9}"#;
        let comment: CanvasComment = serde_json::from_str(raw).unwrap();
        assert_eq!(comment.status, CommentStatus::Addressed);
        assert_eq!(comment.anchor.node_id.as_deref(), Some("n1"));
        assert_eq!(comment.messages[0].author.kind, CommentAuthorKind::Agent);
        assert_eq!(serde_json::to_string(&comment).unwrap(), raw);
    }

    #[test]
    fn missing_status_defaults_to_pending() {
        let comment: CanvasComment =
            serde_json::from_str(r#"{"id":"c1","anchor":{"x":1,"y":2}}"#).unwrap();
        assert_eq!(comment.status, CommentStatus::Pending);
        assert!(comment.messages.is_empty());
    }

    #[test]
    fn lifecycle_matches_the_documented_graph() {
        use CommentStatus::*;
        assert!(Pending.can_transition_to(Addressed));
        assert!(Addressed.can_transition_to(Pending));
        assert!(Addressed.can_transition_to(Archived));
        assert!(Archived.can_transition_to(Pending));
        assert!(Pending.can_transition_to(Archived));
        assert!(!Archived.can_transition_to(Addressed));
        assert!(!Pending.can_transition_to(Pending));
    }

    #[test]
    fn status_parse_is_case_insensitive() {
        assert_eq!(
            CommentStatus::parse(" Archived "),
            Some(CommentStatus::Archived)
        );
        assert_eq!(CommentStatus::parse("done"), None);
    }
}
