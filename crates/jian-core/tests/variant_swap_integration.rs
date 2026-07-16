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

// Regression: a committed variant swap must rotate image ownership like a
// normal document mount — images that only exist in the newly selected
// variant get admitted (registered + requested), and images owned solely by
// the previous variant are released. Without this, a rotation-driven swap
// left the new variant's images as permanent placeholders.
#[test]
fn variant_swap_admits_new_variant_images_and_releases_stale_ones() {
    use jian_core::render::image_store::{ImageAdmission, ImageResolver};
    use std::rc::Rc;

    struct KeyedImages;
    #[async_trait::async_trait(?Send)]
    impl ImageResolver for KeyedImages {
        fn admission(&self, source: &str) -> Result<Option<ImageAdmission>, String> {
            Ok(Some(ImageAdmission {
                key: format!("asset:{source}"),
                request_source: source.to_owned(),
                requires_network: false,
            }))
        }
        async fn resolve(&self, _source: &str) -> Result<Vec<u8>, String> {
            Ok(vec![1, 2, 3])
        }
    }

    let source: PenDocument = serde_json::from_str(
        r#"{"version":"1.2","responsive":true,"children":[
          {"type":"frame","id":"home-d","screen":"/","width":800,"height":600,"children":[
            {"type":"image","id":"hero","src":"desktop.png","width":20,"height":20},
            {"type":"image","id":"logo","src":"shared.png","width":10,"height":10}]},
          {"type":"frame","id":"home-m","screen":"/","breakpoint":{"minWidth":0,"maxWidth":480},
           "width":320,"height":600,"children":[
            {"type":"image","id":"hero","src":"mobile.png","width":20,"height":20},
            {"type":"image","id":"logo","src":"shared.png","width":10,"height":10}]}]}"#,
    )
    .unwrap();
    let (projected, _) = jian_ops_schema::screen_projection::project_screens(&source);
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
    let mut runtime = Runtime::new_from_document(mounted.clone()).unwrap();
    runtime.image_resolver = Rc::new(KeyedImages);
    // Re-mount so the initial admission runs through the keyed resolver.
    runtime.replace_document(mounted).unwrap();
    runtime.configure_variant_source(normalized, "/", variants);

    assert!(
        runtime.image_store.state("asset:desktop.png").is_some(),
        "initial mount admits the desktop variant image"
    );
    assert_eq!(runtime.image_store.state("asset:mobile.png"), None);

    assert!(runtime.switch_variant("home-m@0-480").unwrap());
    assert!(
        runtime.image_store.state("asset:mobile.png").is_some(),
        "a committed variant swap must admit the new variant's images"
    );
    assert_eq!(
        runtime.image_store.state("asset:desktop.png"),
        None,
        "images owned only by the previous variant are released"
    );
    assert!(
        runtime.image_store.state("asset:shared.png").is_some(),
        "an image present in both variants survives the ownership rotation"
    );

    // Swapping back re-admits the original variant's image and releases the
    // one owned only by the variant being left.
    assert!(runtime.switch_variant("home-d").unwrap());
    assert!(runtime.image_store.state("asset:desktop.png").is_some());
    assert_eq!(runtime.image_store.state("asset:mobile.png"), None);
    assert!(runtime.image_store.state("asset:shared.png").is_some());
}
