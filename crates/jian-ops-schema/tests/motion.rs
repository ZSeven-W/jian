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

fn hero_iterations(iterations: &str) -> Result<serde_json::Value, OpsSchemaError> {
    let source = motion_document(&format!(
        r##""animations":[{{"trigger":"mount","durationMs":200,"iterations":{iterations},"keyframes":[
            {{"offset":0,"values":{{"opacity":0}}}},
            {{"offset":1,"values":{{"opacity":1}}}}
        ]}}]"##
    ));
    let loaded = load_str(&source)?;
    Ok(serde_json::to_value(&loaded.value).unwrap()["children"][0]["animations"][0].clone())
}

#[test]
fn iterations_round_trip_counts_and_infinite() {
    assert_eq!(hero_iterations("3").unwrap()["iterations"], 3);
    assert_eq!(
        hero_iterations("\"infinite\"").unwrap()["iterations"],
        "infinite"
    );
    // Older documents may carry 0; the runtime already treats it as one pass.
    assert_eq!(hero_iterations("0").unwrap()["iterations"], 0);
    assert!(hero_iterations("1").unwrap().get("iterations").is_none());
}

#[test]
fn iterations_reject_other_values() {
    for bad in ["-1", "\"forever\"", "1.5", "null"] {
        assert!(
            hero_iterations(bad).is_err(),
            "iterations {bad} should not load"
        );
    }
}

#[test]
fn iterations_api_reports_infinite_and_caps_counts() {
    let animation: NodeAnimation = serde_json::from_value(serde_json::json!({
        "trigger": "mount", "durationMs": 100, "iterations": "infinite",
        "keyframes": {"from": {"opacity": 0}, "to": {"opacity": 1}}
    }))
    .expect("infinite animation");
    assert!(animation.iterations.is_infinite());
    assert_eq!(animation.iterations.count_or(u32::MAX), u32::MAX);
    assert_eq!(jian_ops_schema::Iterations::Count(4).count_or(9), 4);
    assert_eq!(
        jian_ops_schema::Iterations::default(),
        jian_ops_schema::Iterations::Count(1)
    );
}

fn animations_document(animations: &str) -> String {
    motion_document(&format!(r##""animations":{animations}"##))
}

const FADE: &str =
    r##""keyframes":[{"offset":0,"values":{"opacity":0}},{"offset":1,"values":{"opacity":1}}]"##;

#[test]
fn manual_trigger_with_id_round_trips() {
    let source = animations_document(&format!(
        r##"[{{"trigger":"manual","id":"reveal","durationMs":300,{FADE}}}]"##
    ));
    let animation = hero_animation_values(&source);
    assert_eq!(animation["trigger"], "manual");
    assert_eq!(animation["id"], "reveal");
    let loaded = load_str(&source).expect("manual animation");
    let (_, animations) = loaded.value.children[0].motion_declarations();
    let animation = &animations.expect("animations")[0];
    assert_eq!(animation.trigger, jian_ops_schema::MotionTrigger::Manual);
    assert_eq!(animation.id.as_deref(), Some("reveal"));
}

#[test]
fn animations_without_id_do_not_serialize_one() {
    let source = animations_document(&format!(
        r##"[{{"trigger":"mount","durationMs":300,{FADE}}}]"##
    ));
    assert!(hero_animation_values(&source).get("id").is_none());
}

#[test]
fn mount_and_in_view_may_carry_ids_for_replay() {
    let source = animations_document(&format!(
        r##"[{{"trigger":"mount","id":"intro","durationMs":300,{FADE}}},{{"trigger":"inView","id":"rise","durationMs":300,{FADE}}}]"##
    ));
    assert!(load_str(&source).is_ok());
}

#[test]
fn manual_ids_are_required_and_unique_per_node() {
    for (name, animations) in [
        (
            "missing id",
            format!(r##"[{{"trigger":"manual","durationMs":300,{FADE}}}]"##),
        ),
        (
            "blank id",
            format!(r##"[{{"trigger":"manual","id":"  ","durationMs":300,{FADE}}}]"##),
        ),
        (
            "duplicate id",
            format!(
                r##"[{{"trigger":"manual","id":"a","durationMs":300,{FADE}}},{{"trigger":"mount","id":"a","durationMs":300,{FADE}}}]"##
            ),
        ),
    ] {
        match load_str(&animations_document(&animations)) {
            Err(OpsSchemaError::MotionValidation { path, .. }) => {
                assert!(path.contains("animations"), "{name}: {path}");
            }
            Ok(_) => panic!("{name}: expected a motion validation error"),
            Err(other) => panic!("{name}: expected a motion validation error, got {other}"),
        }
    }
}

#[test]
fn nodes_may_share_an_id_to_form_one_sequence() {
    let source = format!(
        r##"{{"version":"1.1","formatVersion":"1.1","children":[
            {{"type":"rectangle","id":"a","width":10,"height":10,
              "animations":[{{"trigger":"manual","id":"intro","durationMs":300,{FADE}}}]}},
            {{"type":"rectangle","id":"b","width":10,"height":10,
              "animations":[{{"trigger":"manual","id":"intro","delayMs":60,"durationMs":300,{FADE}}}]}}
        ]}}"##
    );
    assert!(load_str(&source).is_ok());
}

#[test]
fn editors_validate_an_animation_list_like_loading_does() {
    use jian_ops_schema::motion::validate_animations;
    let animations: Vec<NodeAnimation> = serde_json::from_value(serde_json::json!([
        {"trigger": "mount", "keyframes": [
            {"offset": 0, "values": {"opacity": 0}},
            {"offset": 1, "values": {"opacity": 1}}
        ], "durationMs": 300},
        {"trigger": "manual", "id": "pulse", "keyframes": [
            {"offset": 0, "values": {"scaleX": 1}},
            {"offset": 1, "values": {"scaleX": 1.1}}
        ], "durationMs": 200}
    ]))
    .expect("animations");
    assert_eq!(validate_animations(&animations, "$.animations"), Ok(()));

    let mut unsorted = animations.clone();
    unsorted[0].keyframes.swap(0, 1);
    let (path, _) = validate_animations(&unsorted, "$.animations").expect_err("unsorted");
    assert_eq!(path, "$.animations[0].keyframes");

    let mut duplicate = animations.clone();
    duplicate[0].id = Some("pulse".to_owned());
    let (path, reason) = validate_animations(&duplicate, "$.animations").expect_err("duplicate id");
    assert_eq!(path, "$.animations[1].id");
    assert!(reason.contains("used twice"), "{reason}");

    let mut anonymous = animations;
    anonymous[1].id = None;
    let (path, _) = validate_animations(&anonymous, "$.animations").expect_err("manual without id");
    assert_eq!(path, "$.animations[1].id");
}
