use crate::node::PenNode;
use serde::{Deserialize, Serialize};

/// A page is a top-level container. Children are PenNodes.
/// Jian extension fields (state / lifecycle) are added in Task 15.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PenPage {
    pub id: String,
    pub name: String,
    pub children: Vec<PenNode>,
}
