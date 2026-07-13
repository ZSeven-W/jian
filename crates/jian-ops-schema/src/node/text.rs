use super::base::PenNodeBase;
use crate::sizing::SizingBehavior;
use crate::style::{PenEffect, PenFill, StyledTextSegment};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(untagged)]
pub enum TextContent {
    Plain(String),
    Styled(Vec<StyledTextSegment>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(untagged)]
pub enum FontWeight {
    Number(u32),
    Keyword(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum FontStyleKind {
    Normal,
    Italic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum TextAlignVertical {
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "kebab-case")]
pub enum TextGrowth {
    Auto,
    FixedWidth,
    FixedWidthHeight,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct TextNode {
    #[serde(flatten)]
    pub base: PenNodeBase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<SizingBehavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<SizingBehavior>,
    pub content: TextContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<FontWeight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_style: Option<FontStyleKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub letter_spacing: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_align: Option<TextAlign>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_align_vertical: Option<TextAlignVertical>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_growth: Option<TextGrowth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strikethrough: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Vec<PenFill>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<PenEffect>>,
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

impl TextNode {
    /// Return the canonical unitless `lineHeight` multiplier used by both
    /// measurement and paint.
    ///
    /// Compatibility callers sometimes supply an absolute pixel line height
    /// (for example `fontSize:13, lineHeight:17`). Its meaning does not change
    /// when a text box has an explicit height: treating it as a multiplier
    /// still paints lines hundreds of pixels apart. Invalid or pixel-like
    /// values therefore fall back to the shared renderer default everywhere.
    pub fn layout_line_height_multiplier(&self) -> Option<f64> {
        canonical_line_height_multiplier(self.line_height)
    }
}

/// Validate an authored unitless line-height multiplier without consulting
/// box geometry. `None` tells all consumers to use the shared default.
pub fn canonical_line_height_multiplier(line_height: Option<f64>) -> Option<f64> {
    line_height.filter(|value| value.is_finite() && *value > 0.0 && *value <= 4.0)
}

#[cfg(test)]
mod tests {
    use super::canonical_line_height_multiplier;

    #[test]
    fn canonical_line_height_accepts_only_finite_unitless_multipliers() {
        assert_eq!(canonical_line_height_multiplier(None), None);
        assert_eq!(canonical_line_height_multiplier(Some(1.5)), Some(1.5));
        assert_eq!(canonical_line_height_multiplier(Some(4.0)), Some(4.0));
        assert_eq!(canonical_line_height_multiplier(Some(0.0)), None);
        assert_eq!(canonical_line_height_multiplier(Some(-1.0)), None);
        assert_eq!(canonical_line_height_multiplier(Some(4.01)), None);
        assert_eq!(canonical_line_height_multiplier(Some(17.0)), None);
        assert_eq!(canonical_line_height_multiplier(Some(f64::NAN)), None);
        assert_eq!(canonical_line_height_multiplier(Some(f64::INFINITY)), None);
    }
}
