use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct VideoMeta {
    // Video URL or path; this source is never externalized in slice 1.
    pub src: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub autoplay: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub r#loop: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub muted: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hold_last_frame: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub click_to_replay: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_prompt: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !value
}
