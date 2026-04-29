//! `OutcomePayload` — the semantic-result shape every verb fills in.
//!
//! Plan 18 Task 1 + Task 4.5 (AI-native output format). The payload
//! is structured Rust; rendering to JSON or Markdown happens in the
//! session layer based on the agent's negotiated `format`. A bespoke
//! string DSL would only fight the LLM's native parsing — JSON is
//! already a trained-on shape, and the Markdown render is just a
//! prose template over the same fields.

use serde::Serialize;
use std::collections::BTreeMap;

/// The verb-result payload. `verb` / `target` / `narrative` form a
/// human-readable summary; `deltas` records the state mutations the
/// verb caused; `hints` carries actionable next-step advice for the
/// agent (e.g. "the button is currently disabled, wait for `$can_save`
/// to be true"); `detail` is the verb-specific structured payload
/// (state read, snapshot bytes, audit log, …); `error` is `Some` when
/// `ok == false` and carries a stable error code (`NotFound`,
/// `Timeout`, `Denied`, `Invalid`, `RuntimeError`).
#[derive(Debug, Clone, Serialize)]
pub struct OutcomePayload {
    pub ok: bool,
    pub verb: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub narrative: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub deltas: Vec<DeltaEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<DetailKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One state-graph mutation observed during the verb's execution.
/// `path` follows the same `$scope.key.subkey` shape jian's expression
/// engine reads; `before` / `after` carry JSON-typed snapshots so the
/// agent doesn't have to introspect the runtime to make sense of the
/// change.
#[derive(Debug, Clone, Serialize)]
pub struct DeltaEntry {
    pub path: String,
    pub before: serde_json::Value,
    pub after: serde_json::Value,
    /// Optional human-readable cause string. `Some("on_tap @ btn")`
    /// is more useful for an agent's plan-step trace than the raw
    /// expression text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
}

/// Verb-specific structured detail. Internally-tagged enum
/// (`#[serde(tag = "kind")]`) so an agent can dispatch on
/// `detail.kind` without inspecting the rest of the object.
///
/// All variants are *struct* variants (rather than tuple / newtype)
/// because serde's internally-tagged representation can only embed
/// the tag inside an object — newtype variants carrying a
/// `Vec` / `Map` would have nowhere to put the `kind` field. Using
/// struct variants keeps the wire shape uniform: every detail
/// payload looks like `{"kind": "...", <field>: <body>}`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DetailKind {
    /// `inspect what=state` returns the matched scope's key-value
    /// pairs verbatim (filtered by capability — `Observe` agents
    /// don't see `$secrets.*`, etc.).
    State {
        entries: BTreeMap<String, serde_json::Value>,
    },
    /// `inspect what=node_props` returns a compact node summary.
    Node { node: NodeSummary },
    /// `inspect what=ax_tree` returns a compressed accessibility tree.
    /// `truncated: true` flags that the source tree exceeded the
    /// session-level byte budget and was cut off.
    AxTree { text: String, truncated: bool },
    /// `snapshot` returns base64 PNG bytes or text-tree depending on
    /// the requested format.
    Snapshot {
        format: String,
        bytes_or_text: String,
    },
    /// `audit last_n` returns the most recent ring-buffer entries.
    Audit { entries: Vec<AuditEntry> },
}

/// Compact `inspect node_props` payload — bounded fields rather than
/// the runtime's full property bag, so an agent's context window
/// doesn't explode on a deeply-nested schema.
#[derive(Debug, Clone, Serialize)]
pub struct NodeSummary {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub visible: bool,
    /// Layout rect in logical pixels: `[x, y, width, height]`.
    pub rect: [f32; 4],
}

/// One audit-ring entry, returned by the `audit` verb.
#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    /// Timestamp in milliseconds since the runtime's launch instant.
    pub at_ms: u64,
    pub verb: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub ok: bool,
    pub summary: String,
}

impl OutcomePayload {
    /// Empty success outcome — verb fired without state change. Pair
    /// with `with_delta` / `with_detail` / `with_hint` to fill in the
    /// rest. `target` is the resolved target description (typically
    /// the matched node's id or the `selector`'s shortened form).
    pub fn ok(
        verb: &'static str,
        target: impl Into<Option<String>>,
        narrative: impl Into<String>,
    ) -> Self {
        Self {
            ok: true,
            verb,
            target: target.into(),
            narrative: narrative.into(),
            deltas: Vec::new(),
            hints: Vec::new(),
            detail: None,
            error: None,
        }
    }

    /// Selector resolved to zero matches. The agent typically retries
    /// with a relaxed selector (drop `text`, keep `role`) or a
    /// `wait_for` so a not-yet-mounted view appears.
    pub fn not_found(verb: &'static str, target_sel: &str) -> Self {
        Self {
            ok: false,
            verb,
            target: Some(target_sel.to_owned()),
            narrative: format!("no match for target `{}`", target_sel),
            deltas: Vec::new(),
            hints: Vec::new(),
            detail: None,
            error: Some("NotFound".into()),
        }
    }

    /// Wait-style verb expired before the predicate became true.
    pub fn timeout(verb: &'static str, expr: &str, waited_ms: u64) -> Self {
        Self {
            ok: false,
            verb,
            target: None,
            narrative: format!(
                "expression `{}` did not become truthy within {} ms",
                expr, waited_ms
            ),
            deltas: Vec::new(),
            hints: Vec::new(),
            detail: None,
            error: Some("Timeout".into()),
        }
    }

    /// Capability gate refused the verb. `hint` should suggest the
    /// next step (typically the permission tier the agent needs to
    /// request, e.g. `Full` for `set_state`).
    pub fn denied(verb: &'static str, reason: &str, hint: Option<&str>) -> Self {
        Self {
            ok: false,
            verb,
            target: None,
            narrative: reason.to_owned(),
            deltas: Vec::new(),
            hints: hint.into_iter().map(str::to_owned).collect(),
            detail: None,
            error: Some("Denied".into()),
        }
    }

    /// Verb's payload was malformed or violated a per-verb invariant
    /// (e.g. `selector` carrying both `first` and `index`).
    pub fn invalid(verb: &'static str, err: &str) -> Self {
        Self {
            ok: false,
            verb,
            target: None,
            narrative: err.to_owned(),
            deltas: Vec::new(),
            hints: Vec::new(),
            detail: None,
            error: Some("Invalid".into()),
        }
    }

    /// Runtime threw while executing the verb (panic recovery,
    /// failed expression eval, etc.). The string carries the
    /// underlying error's `Display` form — the agent typically
    /// surfaces it verbatim and bails on the test.
    pub fn error(verb: &'static str, err: &str) -> Self {
        Self {
            ok: false,
            verb,
            target: None,
            narrative: err.to_owned(),
            deltas: Vec::new(),
            hints: Vec::new(),
            detail: None,
            error: Some("RuntimeError".into()),
        }
    }

    /// Append a state-graph delta. Builder so verb impls can chain.
    pub fn with_delta(
        mut self,
        path: impl Into<String>,
        before: serde_json::Value,
        after: serde_json::Value,
        cause: Option<String>,
    ) -> Self {
        self.deltas.push(DeltaEntry {
            path: path.into(),
            before,
            after,
            cause,
        });
        self
    }

    /// Attach a verb-specific structured detail. Replaces any prior
    /// detail — verbs only carry one.
    pub fn with_detail(mut self, d: DetailKind) -> Self {
        self.detail = Some(d);
        self
    }

    /// Append an actionable hint string.
    pub fn with_hint(mut self, h: impl Into<String>) -> Self {
        self.hints.push(h.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_constructor_omits_optional_fields() {
        let o = OutcomePayload::ok("tap", Some("btn".to_string()), "tapped Submit");
        let json = serde_json::to_string(&o).unwrap();
        assert!(json.contains(r#""ok":true"#));
        assert!(json.contains(r#""verb":"tap""#));
        assert!(json.contains(r#""target":"btn""#));
        // No deltas/hints/error keys when the slots are empty.
        assert!(!json.contains("\"deltas\""));
        assert!(!json.contains("\"hints\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn not_found_carries_stable_error_code() {
        let o = OutcomePayload::not_found("tap", "Submit button");
        assert!(!o.ok);
        assert_eq!(o.error.as_deref(), Some("NotFound"));
    }

    #[test]
    fn with_delta_appends_chained_entries() {
        let o = OutcomePayload::ok("tap", Some("btn".to_string()), "tapped").with_delta(
            "$app.count",
            serde_json::json!(0),
            serde_json::json!(1),
            Some("on_tap @ btn".into()),
        );
        assert_eq!(o.deltas.len(), 1);
        assert_eq!(o.deltas[0].path, "$app.count");
        assert_eq!(o.deltas[0].before, serde_json::json!(0));
        assert_eq!(o.deltas[0].after, serde_json::json!(1));
    }

    #[test]
    fn detail_kind_serialises_with_kind_tag() {
        let mut entries = BTreeMap::new();
        entries.insert("count".into(), serde_json::json!(5));
        let payload = DetailKind::State { entries };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.starts_with(r#"{"kind":"state""#));
        assert!(json.contains(r#""entries":{"count":5}"#));
    }

    #[test]
    fn timeout_constructor_includes_waited_ms() {
        let o = OutcomePayload::timeout("wait_for", "$ready == true", 5_000);
        assert!(!o.ok);
        assert!(o.narrative.contains("5000"));
        assert_eq!(o.error.as_deref(), Some("Timeout"));
    }

    #[test]
    fn audit_detail_round_trips_one_entry() {
        let entries = vec![AuditEntry {
            at_ms: 12_345,
            verb: "tap".into(),
            target: Some("btn".into()),
            ok: true,
            summary: "tapped Submit".into(),
        }];
        let detail = DetailKind::Audit { entries };
        let json = serde_json::to_string(&detail).unwrap();
        assert!(json.contains(r#""kind":"audit""#));
        assert!(json.contains(r#""at_ms":12345"#));
    }
}
