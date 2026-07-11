use jian_ops_schema::state::{PrimitiveType, StateSchema, StateType};
use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;

pub fn value_conforms(value: &Value, kind: &StateType) -> bool {
    match kind {
        StateType::Primitive(PrimitiveType::Int) => value.as_f64().is_some_and(|number| {
            number.is_finite()
                && number.fract() == 0.0
                && number >= i64::MIN as f64
                && number <= i64::MAX as f64
        }),
        StateType::Primitive(PrimitiveType::Float | PrimitiveType::Number) => value.is_number(),
        StateType::Primitive(PrimitiveType::String | PrimitiveType::Date) => value.is_string(),
        StateType::Primitive(PrimitiveType::Bool) => value.is_boolean(),
        StateType::Primitive(PrimitiveType::Array) => value.is_array(),
        StateType::Primitive(PrimitiveType::Object) => value.is_object(),
        StateType::OneOf { options } => {
            !options.is_empty() && options.iter().any(|option| value_conforms(value, option))
        }
        StateType::Array { array } => value
            .as_array()
            .is_some_and(|values| values.iter().all(|value| value_conforms(value, array))),
        StateType::Object { object } => value.as_object().is_some_and(|value| {
            object.iter().all(|(key, kind)| {
                value
                    .get(key)
                    .is_some_and(|field| value_conforms(field, kind))
            })
        }),
    }
}

pub fn zero_value(kind: &StateType) -> Value {
    match kind {
        StateType::Primitive(PrimitiveType::Int | PrimitiveType::Float | PrimitiveType::Number) => {
            Value::Number(Number::from(0))
        }
        StateType::Primitive(PrimitiveType::String | PrimitiveType::Date) => {
            Value::String(String::new())
        }
        StateType::Primitive(PrimitiveType::Bool) => Value::Bool(false),
        StateType::Primitive(PrimitiveType::Array) | StateType::Array { .. } => {
            Value::Array(Vec::new())
        }
        StateType::Primitive(PrimitiveType::Object) => Value::Object(Map::new()),
        StateType::Object { object } => Value::Object(
            object
                .iter()
                .map(|(key, kind)| (key.clone(), zero_value(kind)))
                .collect(),
        ),
        StateType::OneOf { options } => options.first().map_or(Value::Null, zero_value),
    }
}

pub fn merge_scope(
    live: &BTreeMap<String, Value>,
    staged_defaults: &BTreeMap<String, Value>,
    declared: &StateSchema,
) -> (BTreeMap<String, Value>, Vec<String>) {
    let mut merged = BTreeMap::new();
    let mut warnings = Vec::new();
    for (key, entry) in declared {
        let staged = staged_defaults
            .get(key)
            .cloned()
            .or_else(|| entry.default.clone())
            .unwrap_or(Value::Null);
        let value = match (&entry.kind, live.get(key)) {
            (StateType::Object { object }, Some(Value::Object(live_object))) => {
                let staged_object = staged.as_object();
                let mut result = Map::new();
                for (field, field_kind) in object {
                    let candidate = live_object.get(field);
                    let selected = candidate
                        .filter(|value| value_conforms(value, field_kind))
                        .cloned()
                        .or_else(|| {
                            staged_object
                                .and_then(|object| object.get(field))
                                .filter(|value| value_conforms(value, field_kind))
                                .cloned()
                        })
                        .unwrap_or_else(|| zero_value(field_kind));
                    if candidate.is_some_and(|value| !value_conforms(value, field_kind)) {
                        warnings.push(format!("state `{key}.{field}` no longer conforms"));
                    }
                    result.insert(field.clone(), selected);
                }
                Value::Object(result)
            }
            (_, Some(live_value))
                if value_conforms(live_value, &entry.kind)
                    || (live_value.is_null() && staged.is_null()) =>
            {
                live_value.clone()
            }
            (_, Some(_)) => {
                warnings.push(format!("state `{key}` no longer conforms"));
                staged
            }
            (_, None) => staged,
        };
        merged.insert(key.clone(), value);
    }
    (merged, warnings)
}
