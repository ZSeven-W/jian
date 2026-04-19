use serde::{Deserialize, Serialize};

/// A Tier 1 expression source — represented as a raw string.
///
/// Parsing and validation are the responsibility of `jian-core::expression::parser`
/// (Plan 2). The schema crate only guarantees the string is present; content-level
/// correctness is deferred.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Expression(pub String);

impl Expression {
    pub fn new(src: impl Into<String>) -> Self {
        Self(src.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Expression {
    fn from(s: &str) -> Self {
        Expression(s.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expression_roundtrip() {
        let json = r#""$state.count + 1""#;
        let e: Expression = serde_json::from_str(json).unwrap();
        assert_eq!(e.as_str(), "$state.count + 1");
        assert_eq!(serde_json::to_string(&e).unwrap(), json);
    }
}
