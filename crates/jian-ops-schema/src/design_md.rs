//! `design.md` — the per-document design-system spec.
//!
//! A `.op` document can carry a free-form "design.md" markdown brief
//! (visual theme, colour palette, typography, …). It is parsed into
//! this structured shape for display while the original markdown is
//! kept verbatim in [`DesignMdSpec::raw`] for round-trip fidelity.
//! Mirrors the TS `pen-types` `DesignMdSpec`.

use serde::{Deserialize, Serialize};

/// A structured design-system brief attached to a [`PenDocument`].
///
/// [`PenDocument`]: crate::document::PenDocument
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct DesignMdSpec {
    /// Original markdown source — kept verbatim for round-trip fidelity.
    pub raw: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_theme: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_palette: Option<Vec<DesignMdColor>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typography: Option<DesignMdTypography>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_styles: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_principles: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_notes: Option<String>,
}

/// One named colour from the design-md colour palette.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct DesignMdColor {
    /// Human label, e.g. "Primary".
    pub name: String,
    /// `#RRGGBB` hex value.
    pub hex: String,
    /// How the colour is used, e.g. "buttons and links".
    pub role: String,
}

/// Typography guidance from the design-md typography section.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct DesignMdTypography {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headings: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,

    /// Free-form scale description — usually the whole section text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let spec = DesignMdSpec {
            raw: "# Design System: Demo\n".to_string(),
            project_name: Some("Demo".to_string()),
            visual_theme: Some("Calm and minimal".to_string()),
            color_palette: Some(vec![DesignMdColor {
                name: "Primary".to_string(),
                hex: "#3366FF".to_string(),
                role: "buttons".to_string(),
            }]),
            typography: Some(DesignMdTypography {
                font_family: Some("Inter".to_string()),
                ..DesignMdTypography::default()
            }),
            component_styles: None,
            layout_principles: None,
            generation_notes: None,
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: DesignMdSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, back);
        // Optional empty fields are omitted from the wire form.
        assert!(!json.contains("componentStyles"));
        assert!(json.contains("\"projectName\":\"Demo\""));
    }
}
