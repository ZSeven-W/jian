use async_trait::async_trait;
use jian_core::action::services::{
    HttpRequest, HttpResponse, NetworkClient, ServiceError, StorageBackend,
};
use jian_core::expression::Expression;
use jian_core::gesture::{PointerEvent, PointerPhase};
use jian_core::render::CaptureBackend;
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

#[test]
fn responsive_runtime_substrate_runs_through_real_pump_dispatch_and_prepare() {
    let source = include_str!("fixtures/runtime_substrate.json");
    let schema: PenDocument = serde_json::from_str(source).unwrap();
    let mut runtime = Runtime::new_from_document(schema).unwrap();
    runtime.storage = Rc::new(Storage);
    runtime.network = Rc::new(Network);
    runtime.build_layout((200.0, 160.0)).unwrap();

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

    runtime
        .image_store
        .admit_resolver("https://example.invalid/hero.png", 3);
    runtime
        .image_store
        .resolve("https://example.invalid/hero.png", vec![1, 2, 3]);
    let mut backend = CaptureBackend::new();
    runtime.prepare_frame(&mut backend, 0);
    assert_eq!(
        runtime
            .image_store
            .state("https://example.invalid/hero.png"),
        Some(jian_core::render::image_store::ImageState::Registered)
    );

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
    runtime.pump(2);
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
