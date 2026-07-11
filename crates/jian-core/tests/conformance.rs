use jian_core::state::conformance::{merge_scope, value_conforms, zero_value};
use jian_ops_schema::state::{PrimitiveType, StateEntry, StateType};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn every_state_type_arm_has_total_conformance_and_zero_values() {
    let cases = [
        (StateType::Primitive(PrimitiveType::Int), json!(2), json!(0)),
        (
            StateType::Primitive(PrimitiveType::Float),
            json!(2.5),
            json!(0),
        ),
        (
            StateType::Primitive(PrimitiveType::Number),
            json!(2),
            json!(0),
        ),
        (
            StateType::Primitive(PrimitiveType::String),
            json!("x"),
            json!(""),
        ),
        (
            StateType::Primitive(PrimitiveType::Bool),
            json!(true),
            json!(false),
        ),
        (
            StateType::Primitive(PrimitiveType::Date),
            json!("today"),
            json!(""),
        ),
        (
            StateType::Primitive(PrimitiveType::Array),
            json!([1]),
            json!([]),
        ),
        (
            StateType::Primitive(PrimitiveType::Object),
            json!({"x":1}),
            json!({}),
        ),
        (
            StateType::Array {
                array: Box::new(StateType::Primitive(PrimitiveType::Int)),
            },
            json!([1, 2]),
            json!([]),
        ),
        (
            StateType::Object {
                object: BTreeMap::from([("ok".into(), StateType::Primitive(PrimitiveType::Bool))]),
            },
            json!({"ok":true}),
            json!({"ok":false}),
        ),
        (
            StateType::OneOf {
                options: vec![StateType::Primitive(PrimitiveType::String)],
            },
            json!("x"),
            json!(""),
        ),
    ];
    for (kind, value, zero) in cases {
        assert!(value_conforms(&value, &kind));
        assert_eq!(zero_value(&kind), zero);
    }
    let empty = StateType::OneOf { options: vec![] };
    assert!(!value_conforms(&serde_json::Value::Null, &empty));
    assert_eq!(zero_value(&empty), serde_json::Value::Null);
}

#[test]
fn merge_scope_prunes_objects_and_falls_back_fieldwise() {
    let kind = StateType::Object {
        object: BTreeMap::from([
            ("name".into(), StateType::Primitive(PrimitiveType::String)),
            ("count".into(), StateType::Primitive(PrimitiveType::Int)),
        ]),
    };
    let declared = BTreeMap::from([(
        "user".into(),
        StateEntry {
            kind,
            default: Some(json!({"name":"staged","count":7})),
            description: None,
            persist: None,
        },
    )]);
    let (merged, warnings) = merge_scope(
        &BTreeMap::from([(
            "user".into(),
            json!({"name":"live","count":"bad","extra":1}),
        )]),
        &BTreeMap::from([("user".into(), json!({"name":"staged","count":7}))]),
        &declared,
    );
    assert_eq!(merged["user"], json!({"name":"live","count":7}));
    assert!(!warnings.is_empty());
}
