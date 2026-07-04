use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Ledger of code-to-design conversion units.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ConversionSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<ConversionEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ConversionEntry {
    pub kind: ConversionKind,
    /// Caller-stable key, e.g. "src/Button.tsx#Button" or "route:/settings".
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    /// Content fingerprint of the source unit; used for incremental conversion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    /// Master frame id (component) or screen frame id. None for token entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// Source-node id to document-node id mapping for idempotent reruns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_ids: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "lowercase")]
pub enum ConversionKind {
    Token,
    Component,
    Screen,
}
