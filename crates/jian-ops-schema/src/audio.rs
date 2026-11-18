//! Embedded soundtracks attached to a container's motion timeline.

use crate::motion::MotionTrigger;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct AudioTrack {
    pub id: String,
    pub name: String,
    /// Embedded audio data URI. No dependency on the importing machine's path.
    pub source: String,
    pub duration_ms: u64,
    #[serde(default)]
    pub start_ms: u64,
    #[serde(default)]
    pub trim_start_ms: u64,
    #[serde(default = "full_volume")]
    pub volume: f32,
    #[serde(default)]
    pub muted: bool,
    #[serde(default = "mount")]
    pub trigger: MotionTrigger,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narration: Option<Narration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct Narration {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    pub rate: u32,
}

fn full_volume() -> f32 {
    1.0
}

fn mount() -> MotionTrigger {
    MotionTrigger::Mount
}
