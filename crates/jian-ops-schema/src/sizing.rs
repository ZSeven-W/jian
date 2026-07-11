use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "snake_case")]
pub enum SizingKeyword {
    FitContent,
    FillContainer,
}

/// Sizing value: a number, a fixed keyword, or an arbitrary string (typically `$variable` ref).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(untagged)]
pub enum SizingBehavior {
    Number(f64),
    Keyword(SizingKeyword),
    Expression(String),
}

/// Optional minimum and maximum size bounds in logical pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[cfg_attr(feature = "export-ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "ops.ts"))]
#[serde(rename_all = "camelCase")]
pub struct SizeLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_height: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<f64>,
}

impl SizeLimits {
    /// Drop invalid values and emit one load warning for each ignored bound.
    pub fn sanitized(&self, node_id: &str, warnings: &mut Vec<String>) -> Self {
        let mut out = *self;
        for (name, slot) in [
            ("minWidth", &mut out.min_width),
            ("maxWidth", &mut out.max_width),
            ("minHeight", &mut out.min_height),
            ("maxHeight", &mut out.max_height),
        ] {
            if let Some(value) = *slot {
                if value < 0.0 || !value.is_finite() {
                    warnings.push(format!("node `{node_id}`: {name} {value} invalid; ignored"));
                    *slot = None;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizing_number() {
        let s: SizingBehavior = serde_json::from_str("100").unwrap();
        assert!(matches!(s, SizingBehavior::Number(100.0)));
    }

    #[test]
    fn sizing_keyword() {
        let s: SizingBehavior = serde_json::from_str(r#""fit_content""#).unwrap();
        assert!(matches!(
            s,
            SizingBehavior::Keyword(SizingKeyword::FitContent)
        ));
    }

    #[test]
    fn sizing_expression_variable_ref() {
        let s: SizingBehavior = serde_json::from_str(r#""$spacing-lg""#).unwrap();
        match s {
            SizingBehavior::Expression(ref e) => assert_eq!(e, "$spacing-lg"),
            _ => panic!("expected Expression"),
        }
    }

    #[test]
    fn size_limits_parse_and_absent_fields_skip() {
        let limits: SizeLimits =
            serde_json::from_str(r#"{"minWidth":320,"maxWidth":768}"#).unwrap();
        assert_eq!(limits.min_width, Some(320.0));
        assert_eq!(limits.max_width, Some(768.0));
        assert!(limits.min_height.is_none());
        let value = serde_json::to_value(limits).unwrap();
        assert!(value.get("minHeight").is_none());
    }

    #[test]
    fn negative_limits_sanitize_to_none_with_warning() {
        let limits: SizeLimits = serde_json::from_str(r#"{"minWidth":-5}"#).unwrap();
        let mut warnings = Vec::new();
        let sanitized = limits.sanitized("n1", &mut warnings);
        assert!(sanitized.min_width.is_none());
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn non_finite_limits_sanitize_to_none_with_warning() {
        let limits = SizeLimits {
            max_height: Some(f64::INFINITY),
            ..SizeLimits::default()
        };
        let mut warnings = Vec::new();
        let sanitized = limits.sanitized("n1", &mut warnings);
        assert!(sanitized.max_height.is_none());
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn container_props_carry_limits_and_stay_bit_identical_without() {
        use crate::node::container::ContainerProps;

        let container: ContainerProps =
            serde_json::from_str(r#"{"width":100,"maxWidth":80}"#).unwrap();
        assert_eq!(container.limits.max_width, Some(80.0));
        let empty: ContainerProps = serde_json::from_str(r#"{}"#).unwrap();
        let value = serde_json::to_value(&empty).unwrap();
        assert!(value.get("maxWidth").is_none());
    }
}
