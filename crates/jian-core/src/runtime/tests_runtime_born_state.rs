// App keys created by actions without a declaration survive document
// reloads (Preview swaps a materialised copy of the document on every
// binding reflow); keys a removed declaration owned are still pruned.

#[test]
fn runtime_born_app_key_survives_reload() {
    let src = r#"{"version":"1.1","formatVersion":"1.1","children":[
        {"type":"frame","id":"root","width":100,"height":100}]}"#;
    let mut rt =
        Runtime::new_from_document(serde_json::from_str::<PenDocument>(src).unwrap()).unwrap();
    rt.state.app_set("qty", serde_json::json!(2));
    rt.replace_document(serde_json::from_str::<PenDocument>(src).unwrap())
        .unwrap();
    assert_eq!(
        rt.state.app_get("qty").and_then(|v| v.as_i64()),
        Some(2),
        "a value written to an undeclared key must not vanish on reload"
    );
}

#[test]
fn removed_declaration_still_prunes_its_key() {
    let declared = r#"{"version":"1.1","formatVersion":"1.1",
        "state":{"gone":{"type":"int","default":1}},
        "children":[{"type":"frame","id":"root","width":100,"height":100}]}"#;
    let undeclared = r#"{"version":"1.1","formatVersion":"1.1","children":[
        {"type":"frame","id":"root","width":100,"height":100}]}"#;
    let mut rt =
        Runtime::new_from_document(serde_json::from_str::<PenDocument>(declared).unwrap())
            .unwrap();
    rt.state.app_set("gone", serde_json::json!(5));
    rt.replace_document(serde_json::from_str::<PenDocument>(undeclared).unwrap())
        .unwrap();
    assert_eq!(rt.state.app_get("gone").map(|v| v.0), None);
}
