use crate::sizing::SizingBehavior;
use crate::style::{PenEffect, PenFill, PenStroke};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    None,
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum JustifyContent {
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
}

// `Deserialize` is hand-implemented below so the canonical schema tolerates
// the full set of CSS-ish alignment strings TS accepts (see `from_css`).
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum AlignItems {
    Start,
    Center,
    End,
    /// Cross-axis stretch. The TS element builders (tabs, segmented
    /// control, quote block) emit `alignItems: 'stretch'`, so the schema
    /// must accept and round-trip it. The layout engine renders it as
    /// `Start` (TS `normalizeAlignItems('stretch') -> 'start'`); the layout
    /// engine has no font-metrics stretch pass, matching that behavior.
    Stretch,
}

impl AlignItems {
    /// Lenient parse mirroring TS `normalizeAlignItems` (pen-core
    /// `layout/engine.ts`). The TS document model stores raw CSS-ish
    /// alignment strings and folds them only at layout time, so the
    /// canonical schema must accept every alias a TS builder or
    /// TS-generated `.op` file can produce. `stretch` is preserved as its
    /// own variant (a distinct flex value); the baseline/edge synonyms fold
    /// to `End` exactly as TS does (no font-metrics baseline pass).
    /// Returns `None` for genuinely unrecognized input.
    pub fn from_css(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "start" | "flex-start" | "left" | "top" => Some(Self::Start),
            "center" | "middle" => Some(Self::Center),
            "end" | "flex-end" | "right" | "bottom" | "baseline" | "first baseline"
            | "last baseline" => Some(Self::End),
            "stretch" => Some(Self::Stretch),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for AlignItems {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        AlignItems::from_css(&raw).ok_or_else(|| {
            serde::de::Error::unknown_variant(&raw, &["start", "center", "end", "stretch"])
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(untagged)]
pub enum Padding {
    Uniform(f64),
    XY([f64; 2]),
    LtrB([f64; 4]),
    Expression(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(untagged)]
pub enum CornerRadius {
    Uniform(f64),
    PerCorner([f64; 4]),
}

/// Container props — shared by Frame/Group/Rectangle.
/// `children` is NOT included here because PenNode children are recursively
/// defined in node/mod.rs to avoid circular module dependency. Each concrete
/// node type that has children declares it explicitly via `children: Option<Vec<PenNode>>`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ContainerProps {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<SizingBehavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<SizingBehavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<crate::node::base::NumberOrExpression>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<Padding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justify_content: Option<JustifyContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align_items: Option<AlignItems>,
    #[serde(default, alias = "clip", skip_serializing_if = "Option::is_none")]
    pub clip_content: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<CornerRadius>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Vec<PenFill>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<PenStroke>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<PenEffect>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_container_roundtrip() {
        let json = "{}";
        let c: ContainerProps = serde_json::from_str(json).unwrap();
        assert_eq!(c, ContainerProps::default());
        assert_eq!(serde_json::to_string(&c).unwrap(), json);
    }

    #[test]
    fn padding_uniform() {
        let j = r#"{"padding":8}"#;
        let c: ContainerProps = serde_json::from_str(j).unwrap();
        assert!(matches!(c.padding, Some(Padding::Uniform(8.0))));
    }

    #[test]
    fn padding_per_side() {
        let j = r#"{"padding":[1,2,3,4]}"#;
        let c: ContainerProps = serde_json::from_str(j).unwrap();
        assert!(matches!(
            c.padding,
            Some(Padding::LtrB([1.0, 2.0, 3.0, 4.0]))
        ));
    }

    #[test]
    fn layout_horizontal() {
        let j = r#"{"layout":"horizontal","gap":12,"padding":[0,0],"justifyContent":"space_between","alignItems":"center"}"#;
        let c: ContainerProps = serde_json::from_str(j).unwrap();
        assert!(matches!(c.layout, Some(LayoutMode::Horizontal)));
        assert!(matches!(
            c.justify_content,
            Some(JustifyContent::SpaceBetween)
        ));
        assert!(matches!(c.align_items, Some(AlignItems::Center)));
    }

    #[test]
    fn align_items_accepts_css_alias_set() {
        // stretch is its own variant and round-trips byte-identically.
        assert_eq!(AlignItems::from_css("stretch"), Some(AlignItems::Stretch));
        assert_eq!(
            serde_json::to_string(&AlignItems::Stretch).unwrap(),
            "\"stretch\""
        );
        // baseline + edge synonyms fold to End (TS normalizeAlignItems).
        for s in [
            "baseline",
            "first baseline",
            "last baseline",
            "flex-end",
            "right",
            "bottom",
        ] {
            assert_eq!(AlignItems::from_css(s), Some(AlignItems::End), "{s}");
        }
        // start synonyms fold to Start; center synonyms to Center.
        for s in ["flex-start", "left", "top"] {
            assert_eq!(AlignItems::from_css(s), Some(AlignItems::Start), "{s}");
        }
        assert_eq!(AlignItems::from_css("middle"), Some(AlignItems::Center));
        // Case/whitespace tolerant; genuine garbage rejected.
        assert_eq!(AlignItems::from_css(" STRETCH "), Some(AlignItems::Stretch));
        assert_eq!(AlignItems::from_css("nonsense"), None);
    }

    #[test]
    fn align_items_deserializes_stretch_and_baseline_in_container() {
        let j = r#"{"layout":"horizontal","alignItems":"stretch"}"#;
        let c: ContainerProps = serde_json::from_str(j).unwrap();
        assert!(matches!(c.align_items, Some(AlignItems::Stretch)));
        // A TS-generated doc carrying "baseline" must load (folds to End).
        let j2 = r#"{"layout":"horizontal","alignItems":"baseline"}"#;
        let c2: ContainerProps = serde_json::from_str(j2).unwrap();
        assert!(matches!(c2.align_items, Some(AlignItems::End)));
        // Garbage is still rejected.
        assert!(serde_json::from_str::<ContainerProps>(r#"{"alignItems":"sideways"}"#).is_err());
    }
}
