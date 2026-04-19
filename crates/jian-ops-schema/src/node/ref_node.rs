use super::base::PenNodeBase;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Partial-override dictionary: descendant id → partial PenNode (as raw JSON).
/// We keep it untyped (`Value`) because validating partial nodes needs the full
/// schema and is not the job of the schema crate.
pub type DescendantOverrides = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefNode {
    #[serde(flatten)]
    pub base: PenNodeBase,
    #[serde(rename = "ref")]
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descendants: Option<DescendantOverrides>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<super::PenNode>>,
}
