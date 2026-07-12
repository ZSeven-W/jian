use async_trait::async_trait;
use jian_core::action::services::{
    HttpRequest, HttpResponse, NetworkClient, ServiceError, StorageBackend,
};
use jian_core::expression::Expression;
use jian_core::gesture::{PointerEvent, PointerPhase};
use jian_core::render::image_store::ImageResolver;
use jian_core::render::CaptureBackend;
use jian_core::render::{collect_draws_with_state, DrawOp, ImageSource};
use jian_core::Runtime;
use jian_ops_schema::document::PenDocument;
use serde_json::json;
use std::rc::Rc;

struct Storage;
#[async_trait(?Send)]
impl StorageBackend for Storage {
    async fn get(&self, key: &str) -> Result<Option<serde_json::Value>, ServiceError> {
        Ok((key == "theme").then(|| json!("dark")))
    }
    async fn set(&self, _: &str, _: serde_json::Value) -> Result<(), ServiceError> {
        Ok(())
    }
    async fn delete(&self, _: &str) -> Result<(), ServiceError> {
        Ok(())
    }
    async fn clear(&self) -> Result<(), ServiceError> {
        Ok(())
    }
    async fn keys(&self) -> Result<Vec<String>, ServiceError> {
        Ok(vec![])
    }
}

struct Network;
#[async_trait(?Send)]
impl NetworkClient for Network {
    async fn request(&self, _: HttpRequest) -> Result<HttpResponse, String> {
        Ok(HttpResponse {
            status: 200,
            headers: Default::default(),
            body: json!("fetched"),
        })
    }
}

struct Images;
#[async_trait(?Send)]
impl ImageResolver for Images {
    async fn resolve(&self, url: &str) -> Result<Vec<u8>, String> {
        assert_eq!(url, "https://example.invalid/hero.png");
        Ok(vec![1, 2, 3])
    }
}

#[test]
fn responsive_runtime_substrate_runs_through_real_pump_dispatch_and_prepare() {
    let source = include_str!("fixtures/runtime_substrate.json");
    let schema: PenDocument = serde_json::from_str(source).unwrap();
    let mut runtime = Runtime::new_from_document(schema).unwrap();
    runtime.storage = Rc::new(Storage);
    runtime.network = Rc::new(Network);
    runtime.image_resolver = Rc::new(Images);
    runtime.build_layout((200.0, 160.0)).unwrap();

    let placeholder = collect_draws_with_state(
        runtime.document.as_ref().unwrap(),
        &runtime.layout,
        &runtime.state,
    );
    assert!(placeholder.iter().any(|op| matches!(op, DrawOp::Image {
        source: ImageSource::Url(key), ..
    } if key == "https://example.invalid/hero.png")));

    let storage = Expression::compile("$storage.theme").unwrap();
    assert!(storage.eval(&runtime.state, None, None).0.is_null());
    runtime.pump(1);
    assert_eq!(
        storage.eval(&runtime.state, None, None).0.as_str(),
        Some("dark")
    );
    runtime.set_viewport_size((320.0, 160.0));
    assert_eq!(
        Expression::compile("$viewport.width")
            .unwrap()
            .eval(&runtime.state, None, None)
            .0
            .as_f64(),
        Some(320.0)
    );

    runtime.pump(2);
    runtime.pump(3);
    let mut backend = CaptureBackend::new();
    runtime.prepare_frame(&mut backend, 0);
    assert_eq!(
        runtime
            .image_store
            .state("https://example.invalid/hero.png"),
        Some(jian_core::render::image_store::ImageState::Registered)
    );
    assert!(collect_draws_with_state(
        runtime.document.as_ref().unwrap(),
        &runtime.layout,
        &runtime.state
    )
    .iter()
    .any(
        |op| matches!(op, DrawOp::Image { source: ImageSource::Url(key), .. }
            if key == "https://example.invalid/hero.png")
    ));

    let button = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("button")
        .unwrap();
    let rect = runtime.layout.node_rect(button).unwrap();
    let point = jian_core::geometry::point(rect.min_x() + 2.0, rect.min_y() + 2.0);
    runtime.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Down, point));
    runtime.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Up, point));
    runtime.pump(4);
    assert_eq!(
        runtime.state.app_get("result").unwrap().as_str(),
        Some("fetched")
    );

    let replacement: PenDocument = serde_json::from_str(
        &source
            .replace("\"type\":\"string\"", "\"type\":\"int\"")
            .replace("\"default\":\"\"", "\"default\":7"),
    )
    .unwrap();
    runtime.replace_document(replacement).unwrap();
    assert_eq!(runtime.state.app_get("result").unwrap().as_i64(), Some(7));
}

#[test]
fn reload_merges_page_and_self_per_page_key_and_replaces_vars_from_staged_doc() {
    let before: PenDocument = serde_json::from_value(json!({
        "version":"1.2", "responsive":true,
        "variables":{"accent":{"type":"string","value":"old-default"}},
        "pages":[{"id":"main","name":"Main","state":{"count":{"type":"int","default":1}},
            "children":[{"type":"rectangle","id":"card","width":10,"height":10,
                "state":{"label":{"type":"string","default":"old"}}}]}], "children":[]
    }))
    .unwrap();
    let mut runtime = Runtime::new_from_document(before).unwrap();
    runtime.state.page_set("main", "count", json!(7));
    runtime.state.self_set("main", "card", "label", json!(99));
    runtime.state.vars_set("accent", json!("live-edit"));
    let after: PenDocument = serde_json::from_value(json!({
        "version":"1.2", "responsive":true,
        "variables":{"accent":{"type":"string","value":"new-default"}},
        "pages":[{"id":"main","name":"Main","state":{"count":{"type":"int","default":2}},
            "children":[{"type":"rectangle","id":"card","width":10,"height":10,
                "state":{"label":{"type":"string","default":"new"}}}]}], "children":[]
    }))
    .unwrap();
    runtime.replace_document(after).unwrap();
    assert_eq!(
        runtime.state.page_get("main", "count").unwrap().as_i64(),
        Some(7)
    );
    assert_eq!(
        runtime
            .state
            .self_get("main", "card", "label")
            .unwrap()
            .as_str(),
        Some("new")
    );
    assert_eq!(
        runtime.state.vars_get("accent").unwrap().as_str(),
        Some("new-default")
    );
    assert!(runtime
        .load_warnings()
        .iter()
        .any(|warning| warning.contains("$self[main/card]")));
}
