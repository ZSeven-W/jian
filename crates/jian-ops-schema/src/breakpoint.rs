use serde::{Deserialize, Serialize};

/// Inclusive viewport-width range for a responsive screen variant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct BreakpointRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointInvalid {
    Empty,
    NegativeOrNonFinite,
    Reversed,
}

impl BreakpointRange {
    pub fn validate(&self) -> Result<(), BreakpointInvalid> {
        if self.min_width.is_none() && self.max_width.is_none() {
            return Err(BreakpointInvalid::Empty);
        }
        if [self.min_width, self.max_width]
            .into_iter()
            .flatten()
            .any(|bound| bound < 0.0 || !bound.is_finite())
        {
            return Err(BreakpointInvalid::NegativeOrNonFinite);
        }
        if matches!((self.min_width, self.max_width), (Some(min), Some(max)) if min > max) {
            return Err(BreakpointInvalid::Reversed);
        }
        Ok(())
    }
}
