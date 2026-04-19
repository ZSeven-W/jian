use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizingKeyword {
    FitContent,
    FillContainer,
}

/// Sizing value: a number, a fixed keyword, or an arbitrary string (typically `$variable` ref).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SizingBehavior {
    Number(f64),
    Keyword(SizingKeyword),
    Expression(String),
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
        assert!(matches!(s, SizingBehavior::Keyword(SizingKeyword::FitContent)));
    }

    #[test]
    fn sizing_expression_variable_ref() {
        let s: SizingBehavior = serde_json::from_str(r#""$spacing-lg""#).unwrap();
        match s {
            SizingBehavior::Expression(ref e) => assert_eq!(e, "$spacing-lg"),
            _ => panic!("expected Expression"),
        }
    }
}
