use super::base::PenNodeBase;
use crate::sizing::SizingBehavior;
use crate::style::{PenFill, PenStroke};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconFontNode {
    #[serde(flatten)]
    pub base: PenNodeBase,
    pub icon_font_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_font_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<SizingBehavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<SizingBehavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Vec<PenFill>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<PenStroke>,
}
