//! Helpers to parse Action bodies from arbitrary JSON.

use super::error::ActionError;
use serde_json::Value;

/// Extract a required `&str` field.
pub fn take_str<'a>(
    body: &'a Value,
    action: &'static str,
    field: &'static str,
) -> Result<&'a str, ActionError> {
    body.get(field)
        .and_then(|v| v.as_str())
        .ok_or(ActionError::MissingField {
            name: action,
            field,
        })
}

/// Extract an optional `&str` field.
pub fn opt_str<'a>(body: &'a Value, field: &str) -> Option<&'a str> {
    body.get(field).and_then(|v| v.as_str())
}

/// Extract a required array field.
pub fn take_array<'a>(
    body: &'a Value,
    action: &'static str,
    field: &'static str,
) -> Result<&'a Vec<Value>, ActionError> {
    body.get(field)
        .and_then(|v| v.as_array())
        .ok_or(ActionError::MissingField {
            name: action,
            field,
        })
}

/// Extract a required object field.
pub fn take_object<'a>(
    body: &'a Value,
    action: &'static str,
    field: &'static str,
) -> Result<&'a serde_json::Map<String, Value>, ActionError> {
    body.get(field)
        .and_then(|v| v.as_object())
        .ok_or(ActionError::MissingField {
            name: action,
            field,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn take_str_ok() {
        let b = json!({"url": "/api"});
        assert_eq!(take_str(&b, "fetch", "url").unwrap(), "/api");
    }

    #[test]
    fn take_str_missing() {
        let b = json!({});
        assert!(matches!(
            take_str(&b, "fetch", "url"),
            Err(ActionError::MissingField { .. })
        ));
    }
}
