use super::base::PenNodeBase;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Partial-override dictionary: descendant id → partial PenNode (as raw JSON).
/// We keep it untyped (`Value`) because validating partial nodes needs the full
/// schema and is not the job of the schema crate.
pub type DescendantOverrides = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<crate::state::StateSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<crate::events::Bindings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<crate::events::EventHandlers>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<crate::lifecycle::NodeLifecycleHooks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantics: Option<crate::semantics::SemanticsMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gestures: Option<crate::gestures::GestureOverrides>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<crate::navigation::NavigationRoute>,
}
