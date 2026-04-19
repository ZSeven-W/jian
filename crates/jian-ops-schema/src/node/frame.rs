use super::base::PenNodeBase;
use super::container::ContainerProps;
use serde::{Deserialize, Serialize};

/// Forward declaration of PenNode union — defined in `node/mod.rs`.
/// We accept `Vec<super::PenNode>` as children.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameNode {
    #[serde(flatten)]
    pub base: PenNodeBase,
    #[serde(flatten)]
    pub container: ContainerProps,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<super::PenNode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reusable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupNode {
    #[serde(flatten)]
    pub base: PenNodeBase,
    #[serde(flatten)]
    pub container: ContainerProps,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<super::PenNode>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RectangleNode {
    #[serde(flatten)]
    pub base: PenNodeBase,
    #[serde(flatten)]
    pub container: ContainerProps,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<super::PenNode>>,
}
