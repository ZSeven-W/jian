// Text-edit write-back: `$app.` and `$state.` bind:value targets are
// the same scope, and every user edit (typed text, paste, Backspace,
// IME commit) fires `onChange` exactly when the value moved.

fn text_binding_doc(bindings: &str, events: &str) -> Runtime {
    let json = format!(
        r#"{{"version":"1.1","formatVersion":"1.1",
              "state":{{"name":{{"type":"string","default":""}},
                       "changes":{{"type":"int","default":0}},
                       "echo":{{"type":"string","default":""}}}},
              "children":[{{"type":"frame","id":"root","children":[
                {{"type":"text_input","id":"name"{bindings}{events}}}]}}]}}"#
    );
    let mut rt = Runtime::new_from_document(serde_json::from_str::<PenDocument>(&json).unwrap())
        .unwrap();
    rt.build_layout((400.0, 300.0)).unwrap();
    rt.focus_next().unwrap();
    rt
}

fn app_str(rt: &Runtime, key: &str) -> Option<String> {
    rt.state
        .app_get(key)
        .and_then(|v| v.as_str().map(str::to_owned))
}

fn app_int(rt: &Runtime, key: &str) -> i64 {
    rt.state.app_get(key).and_then(|v| v.as_i64()).unwrap_or(-1)
}

const BIND_APP: &str = r#","bindings":{"bind:value":"$app.name"}"#;
const ON_CHANGE: &str = r#","events":{"onChange":[
    {"set":{"$app.echo":"$event.value","$app.changes":"$app.changes + 1"}}]}"#;

#[test]
fn app_prefixed_bind_value_writes_back_like_state_prefix() {
    use crate::gesture::pointer::Modifiers;
    for bind in [BIND_APP, r#","bindings":{"bind:value":"$state.name"}"#] {
        let mut rt = text_binding_doc(bind, "");
        assert!(rt.dispatch_text_input("Alice测试").unwrap());
        assert_eq!(app_str(&rt, "name").as_deref(), Some("Alice测试"), "{bind}");
        rt.dispatch_keyboard("Backspace", "Backspace", false, Modifiers::empty());
        assert_eq!(app_str(&rt, "name").as_deref(), Some("Alice测"), "{bind}");
    }
}

#[test]
fn on_change_alone_fires_for_typed_text_and_backspace() {
    use crate::gesture::pointer::Modifiers;
    let mut rt = text_binding_doc("", ON_CHANGE);
    assert!(rt.dispatch_text_input("Al").unwrap());
    assert_eq!(app_str(&rt, "echo").as_deref(), Some("Al"));
    assert_eq!(app_int(&rt, "changes"), 1);
    rt.dispatch_keyboard("Backspace", "Backspace", false, Modifiers::empty());
    assert_eq!(app_str(&rt, "echo").as_deref(), Some("A"));
    assert_eq!(app_int(&rt, "changes"), 2);
}

#[test]
fn binding_and_on_change_together_fire_once_per_edit() {
    let mut rt = text_binding_doc(BIND_APP, ON_CHANGE);
    assert!(rt.dispatch_text_input("Bo").unwrap());
    assert!(rt.dispatch_text_input("b").unwrap());
    assert_eq!(app_str(&rt, "name").as_deref(), Some("Bob"));
    assert_eq!(app_str(&rt, "echo").as_deref(), Some("Bob"));
    assert_eq!(app_int(&rt, "changes"), 2, "one Change per edit, no echo loop");
}

#[test]
fn ime_preedit_is_silent_and_commit_fires_change() {
    use crate::gesture::ime::{ImeEvent, ImeKind};
    let mut rt = text_binding_doc(BIND_APP, ON_CHANGE);
    rt.dispatch_ime(ImeEvent {
        kind: ImeKind::CompositionStart,
        text: String::new(),
    })
    .unwrap();
    rt.dispatch_ime(ImeEvent {
        kind: ImeKind::CompositionUpdate { selection: None },
        text: "ni".into(),
    })
    .unwrap();
    assert_eq!(app_int(&rt, "changes"), 0, "preedit is not a value");
    assert_eq!(app_str(&rt, "name").as_deref(), Some(""));
    rt.dispatch_ime(ImeEvent {
        kind: ImeKind::CompositionEnd,
        text: "你".into(),
    })
    .unwrap();
    assert_eq!(app_str(&rt, "name").as_deref(), Some("你"));
    assert_eq!(app_int(&rt, "changes"), 1);
}

#[test]
fn a_no_op_edit_does_not_fire_change() {
    use crate::gesture::pointer::Modifiers;
    let mut rt = text_binding_doc("", ON_CHANGE);
    // Backspace in an empty field changes nothing.
    rt.dispatch_keyboard("Backspace", "Backspace", false, Modifiers::empty());
    assert_eq!(app_int(&rt, "changes"), 0);
}
