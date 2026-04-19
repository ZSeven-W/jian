//! `RuntimeValue` wraps [`serde_json::Value`] for use inside Signals.
//!
//! Using `serde_json::Value` directly works; this newtype exists to:
//! - keep room for a future optimized `Value` enum (e.g. interning strings, i64 vs f64 split)
//! - provide conversion helpers from Tier 1 expression outputs and JSON literals

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeValue(pub Value);

impl RuntimeValue {
    pub fn null() -> Self {
        Self(Value::Null)
    }
    pub fn from_i64(v: i64) -> Self {
        Self(Value::from(v))
    }
    pub fn from_f64(v: f64) -> Self {
        Self(Value::from(v))
    }
    pub fn from_bool(v: bool) -> Self {
        Self(Value::Bool(v))
    }
    pub fn from_string(v: impl Into<String>) -> Self {
        Self(Value::String(v.into()))
    }

    pub fn is_null(&self) -> bool {
        self.0.is_null()
    }

    pub fn as_bool(&self) -> Option<bool> {
        self.0.as_bool()
    }
    pub fn as_i64(&self) -> Option<i64> {
        self.0.as_i64()
    }
    pub fn as_f64(&self) -> Option<f64> {
        self.0.as_f64()
    }
    pub fn as_str(&self) -> Option<&str> {
        self.0.as_str()
    }

    /// Loose equality matching JS-style `==` (best-effort):
    /// - null == null
    /// - number == number (f64 compare)
    /// - string == string
    /// - bool == bool
    /// - otherwise strict
    pub fn loose_eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a
                .as_f64()
                .zip(b.as_f64())
                .map(|(x, y)| x == y)
                .unwrap_or(false),
            _ => self.0 == other.0,
        }
    }
}

impl From<Value> for RuntimeValue {
    fn from(v: Value) -> Self {
        RuntimeValue(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_roundtrip() {
        let v = RuntimeValue::null();
        assert!(v.is_null());
    }

    #[test]
    fn primitive_accessors() {
        assert_eq!(RuntimeValue::from_i64(42).as_i64(), Some(42));
        assert_eq!(RuntimeValue::from_bool(true).as_bool(), Some(true));
        assert_eq!(RuntimeValue::from_string("hi").as_str(), Some("hi"));
    }

    #[test]
    fn loose_eq_matches() {
        let a = RuntimeValue::from_i64(1);
        let b = RuntimeValue::from_i64(1);
        assert!(a.loose_eq(&b));
    }
}
