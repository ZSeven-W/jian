use jian_ops_schema::{load_str, OpsSchemaError};

fn motion_document(extra: &str) -> String {
    format!(
        r##"{{
            "version":"1.1",
            "formatVersion":"1.1",
            "children":[{{
                "type":"rectangle","id":"hero","width":100,"height":100,
                {extra}
            }}]
        }}"##
    )
}

#[test]
fn motion_fields_round_trip_for_both_triggers_and_document_preference() {
    let source = motion_document(
        r##""transition":{"durationMs":240,"easing":"emphasized","properties":["opacity","fill"],"futureToken":42},
                "animations":[
                    {"trigger":"mount","keyframes":[
                        {"offset":0,"values":{"opacity":0,"translateY":24}},
                        {"offset":1,"values":{"opacity":1,"translateY":0}}
                    ],"durationMs":400,"easing":"standard","fillMode":"forwards"},
                    {"trigger":"inView","keyframes":[
                        {"offset":0,"values":{"opacity":0}},
                        {"offset":1,"values":{"opacity":1}}
                    ],"durationMs":300,"delayMs":80,"once":false,"fillMode":"none"}
                ]"##,
    );
    let source = source.replace("\"children\":[{", "\"motion\":\"reduced\",\"children\":[{");
    let loaded = load_str(&source).expect("motion document");
    assert_eq!(
        loaded.value.motion,
        Some(jian_ops_schema::MotionPreference::Reduced)
    );
    let node = &loaded.value.children[0];
    let (transition, animations) = node.motion_declarations();
    assert_eq!(transition.unwrap().duration_ms, 240);
    assert_eq!(animations.unwrap().len(), 2);
    assert_eq!(
        serde_json::to_value(&loaded.value).unwrap()["motion"],
        "reduced"
    );
    assert_eq!(
        serde_json::to_value(&loaded.value).unwrap()["children"][0]["transition"]["easing"],
        "emphasized"
    );
    assert_eq!(
        serde_json::to_value(&loaded.value).unwrap()["children"][0]["transition"]["futureToken"],
        42
    );
}

#[test]
fn old_documents_without_motion_load_unchanged() {
    let source = r#"{"version":"0.8.0","children":[{"type":"rectangle","id":"r"}]}"#;
    let loaded = load_str(source).expect("legacy document");
    assert_eq!(loaded.value.motion, None);
    assert_eq!(serde_json::to_string(&loaded.value).unwrap(), source);
}

#[test]
fn invalid_keyframes_are_typed_load_errors() {
    for (name, keyframes) in [
        (
            "unsorted",
            r#"[{"offset":0,"values":{"opacity":0}},{"offset":0.8,"values":{"opacity":1}},{"offset":0.5,"values":{"opacity":1}}]"#,
        ),
        (
            "missing_zero",
            r#"[{"offset":0.2,"values":{"opacity":0}},{"offset":1,"values":{"opacity":1}}]"#,
        ),
        (
            "missing_one",
            r#"[{"offset":0,"values":{"opacity":0}},{"offset":0.8,"values":{"opacity":1}}]"#,
        ),
        (
            "unknown_property",
            r#"[{"offset":0,"values":{"opacity":0,"notAProperty":1}},{"offset":1,"values":{"opacity":1}}]"#,
        ),
    ] {
        let source = motion_document(&format!(
            r##""animations":[{{"trigger":"mount","keyframes":{keyframes},"durationMs":200}}]"##
        ));
        match load_str(&source) {
            Err(OpsSchemaError::MotionValidation { path, .. }) => {
                assert!(path.contains("animations"), "{name}: {path}");
            }
            Ok(_) => panic!("{name}: expected motion validation error"),
            Err(other) => panic!("{name}: expected motion validation error, got {other}"),
        }
    }
}
