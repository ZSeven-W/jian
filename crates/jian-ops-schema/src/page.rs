use crate::lifecycle::PageLifecycleHooks;
use crate::node::PenNode;
use crate::state::StateSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct PenPage {
    pub id: String,
    pub name: String,
    pub children: Vec<PenNode>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<StateSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<PageLifecycleHooks>,
}
