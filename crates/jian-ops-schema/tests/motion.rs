use jian_ops_schema::{load_str, NodeAnimation, OpsSchemaError};

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

fn hero_animation_values(source: &str) -> serde_json::Value {
    let loaded = load_str(source).expect("motion document");
    serde_json::to_value(&loaded.value).unwrap()["children"][0]["animations"][0].clone()
}

#[test]
fn canonical_keyframes_keep_the_values_wrapper() {
    let source = motion_document(
        r##""animations":[{"trigger":"mount","durationMs":400,"keyframes":[
            {"offset":0,"values":{"opacity":0,"translateY":16}},
            {"offset":1,"values":{"opacity":1,"translateY":0}}
        ]}]"##,
    );
    let animation = hero_animation_values(&source);
    assert_eq!(animation["keyframes"][0]["offset"], 0.0);
    assert_eq!(animation["keyframes"][0]["values"]["opacity"], 0.0);
    assert_eq!(animation["keyframes"][0]["values"]["translateY"], 16.0);
    assert!(animation["keyframes"][0].get("opacity").is_none());
}

#[test]
fn flat_keyframes_load_as_property_maps() {
    let source = motion_document(
        r##""animations":[{"trigger":"mount","durationMs":400,"keyframes":[
            {"offset":0,"opacity":0,"translateY":16},
            {"offset":1,"opacity":1,"translateY":0}
        ]}]"##,
    );
    let loaded = load_str(&source).expect("flat keyframes");
    let (_, animations) = loaded.value.children[0].motion_declarations();
    let keyframes = &animations.expect("animations")[0].keyframes;
    assert_eq!(keyframes[0].offset, 0.0);
    assert_eq!(keyframes[0].values["opacity"], serde_json::json!(0));
    assert_eq!(keyframes[0].values["translateY"], serde_json::json!(16));
    assert_eq!(keyframes[1].offset, 1.0);
    assert_eq!(keyframes[1].values["opacity"], serde_json::json!(1));
}

#[test]
fn mixed_keyframe_list_loads_canonical_and_flat_together() {
    let source = motion_document(
        r##""animations":[{"trigger":"mount","durationMs":400,"keyframes":[
            {"offset":0,"opacity":0,"translateY":16},
            {"offset":1,"values":{"opacity":1,"translateY":0}}
        ]}]"##,
    );
    let animation = hero_animation_values(&source);
    assert_eq!(animation["keyframes"][0]["values"]["opacity"], 0.0);
    assert_eq!(animation["keyframes"][0]["values"]["translateY"], 16.0);
    assert_eq!(animation["keyframes"][1]["values"]["opacity"], 1.0);
    assert_eq!(animation["keyframes"][1]["values"]["translateY"], 0.0);
}

#[test]
fn from_to_shorthand_deserializes_to_offsets_zero_and_one() {
    let animation: NodeAnimation = serde_json::from_value(serde_json::json!({
        "trigger": "mount",
        "durationMs": 400,
        "keyframes": {
            "from": {"opacity": 0, "translateY": 16},
            "to": {"opacity": 1, "translateY": 0}
        }
    }))
    .expect("from/to shorthand");
    assert_eq!(animation.keyframes.len(), 2);
    assert_eq!(animation.keyframes[0].offset, 0.0);
    assert_eq!(animation.keyframes[1].offset, 1.0);
    assert_eq!(
        animation.keyframes[0].values["translateY"],
        serde_json::json!(16)
    );
    assert_eq!(
        animation.keyframes[1].values["opacity"],
        serde_json::json!(1)
    );
}

#[test]
fn missing_keyframe_offset_names_the_index() {
    let err = serde_json::from_str::<NodeAnimation>(
        r#"{"trigger":"mount","durationMs":200,"keyframes":[{"opacity":0},{"offset":1,"opacity":1}]}"#,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("missing field `offset`"),
        "expected missing offset, got {msg}"
    );
    assert!(
        msg.contains("at keyframe 0"),
        "expected keyframe index, got {msg}"
    );
}

#[test]
fn flat_keyframes_serialize_canonical_values_wrapper() {
    let source = motion_document(
        r##""animations":[{"trigger":"mount","durationMs":400,"keyframes":[
            {"offset":0,"opacity":0,"translateY":16},
            {"offset":1,"opacity":1,"translateY":0}
        ]}]"##,
    );
    let animation = hero_animation_values(&source);
    assert_eq!(
        animation["keyframes"][0],
        serde_json::json!({"offset": 0.0, "values": {"opacity": 0, "translateY": 16}})
    );
    assert_eq!(
        animation["keyframes"][1],
        serde_json::json!({"offset": 1.0, "values": {"opacity": 1, "translateY": 0}})
    );
    assert!(animation["keyframes"][0].get("opacity").is_none());
    assert!(animation["keyframes"][0].get("translateY").is_none());
}
