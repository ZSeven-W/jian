use jian_core::widget_state::WidgetState;
use jian_core::Runtime;
use jian_ops_schema::PenDocument;

fn runtime() -> Runtime {
    let source: PenDocument =
        serde_json::from_str(include_str!("fixtures/responsive_variants.json")).unwrap();
    let (projected, warnings) = jian_ops_schema::screen_projection::project_screens(&source);
    assert!(warnings.iter().any(|warning| matches!(
        warning,
        jian_ops_schema::screen_projection::ProjectionWarning::PromotedDefault { .. }
    )));
    let (normalized, variants) = projected.unwrap();
    let desktop = normalized
        .pages
        .as_ref()
        .unwrap()
        .iter()
        .find(|page| page.id == "home-d")
        .unwrap()
        .clone();
    let mut mounted = normalized.clone();
    mounted.pages = Some(vec![desktop]);
    let mut runtime = Runtime::new_from_document(mounted).unwrap();
    runtime.configure_variant_source(normalized, "/", variants);
    runtime
}

fn seed_and_read(runtime: &mut Runtime, expected: &str) {
    let key = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("field")
        .unwrap();
    let node = runtime.document.as_ref().unwrap().tree.nodes[key]
        .schema
        .clone();
    match runtime.widget_states.get_or_init(&node, &runtime.state) {
        Some(WidgetState::TextInput(text)) => assert_eq!(text.text(), expected),
        _ => panic!("missing text input state"),
    }
}

#[test]
fn sequential_variant_resizes_preserve_app_and_isolate_widgets() {
    let mut runtime = runtime();
    runtime.state.app_set("shared", serde_json::json!(7));
    seed_and_read(&mut runtime, "desktop");
    assert!(runtime.switch_variant("home-m@0-480").unwrap());
    seed_and_read(&mut runtime, "mobile");
    assert!(runtime.switch_variant("home-t@481-1024").unwrap());
    seed_and_read(&mut runtime, "tablet");
    assert!(runtime.switch_variant("home-d").unwrap());
    seed_and_read(&mut runtime, "desktop");
    assert_eq!(
        runtime.state.app_get("shared").unwrap().0,
        serde_json::json!(7)
    );
}
