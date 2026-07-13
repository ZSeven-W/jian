use async_trait::async_trait;
use jian_core::action::services::{
    HttpRequest, HttpResponse, NetworkClient, ServiceError, StorageBackend,
};
use jian_core::expression::Expression;
use jian_core::geometry::{Affine2, Rect, Size};
use jian_core::gesture::{PointerEvent, PointerPhase};
use jian_core::render::image_store::{ImageAdmission, ImageResolver, ImageState};
use jian_core::render::{
    collect_draws_with_state, DecodeError, DrawOp, ImageSource, RenderBackend, ShadowSpec,
};
use jian_core::Runtime;
use jian_ops_schema::document::PenDocument;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

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

struct TrustedRelativeImages {
    requests: Rc<std::cell::RefCell<Vec<String>>>,
}

#[derive(Default)]
struct ControlledImageState {
    calls: Cell<usize>,
    aborted: Cell<usize>,
    ready: RefCell<BTreeMap<usize, Result<Vec<u8>, String>>>,
    wakers: RefCell<BTreeMap<usize, Waker>>,
}

#[derive(Clone, Default)]
struct ControlledImages(Rc<ControlledImageState>);

struct ControlledImageFuture {
    id: usize,
    state: Rc<ControlledImageState>,
    completed: bool,
}

impl Future for ControlledImageFuture {
    type Output = Result<Vec<u8>, String>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let result = self.state.ready.borrow_mut().remove(&self.id);
        if let Some(result) = result {
            self.completed = true;
            Poll::Ready(result)
        } else {
            self.state
                .wakers
                .borrow_mut()
                .insert(self.id, context.waker().clone());
            Poll::Pending
        }
    }
}

impl Drop for ControlledImageFuture {
    fn drop(&mut self) {
        if !self.completed {
            self.state.aborted.set(self.state.aborted.get() + 1);
        }
    }
}

impl ControlledImages {
    fn complete(&self, id: usize, bytes: Vec<u8>) {
        self.0.ready.borrow_mut().insert(id, Ok(bytes));
        if let Some(waker) = self.0.wakers.borrow_mut().remove(&id) {
            waker.wake();
        }
    }
}

#[async_trait(?Send)]
impl ImageResolver for ControlledImages {
    fn admission(&self, source: &str) -> Result<Option<ImageAdmission>, String> {
        Ok(Some(ImageAdmission {
            key: format!("asset:{source}"),
            request_source: source.to_owned(),
            requires_network: false,
        }))
    }

    async fn resolve(&self, _source: &str) -> Result<Vec<u8>, String> {
        let id = self.0.calls.get();
        self.0.calls.set(id + 1);
        ControlledImageFuture {
            id,
            state: self.0.clone(),
            completed: false,
        }
        .await
    }
}

fn image_document(source: &str) -> PenDocument {
    serde_json::from_value(json!({
        "version":"1.2", "responsive":true,
        "children":[{"type":"image","id":"hero","src":source,"width":20,"height":20}]
    }))
    .unwrap()
}

fn empty_document() -> PenDocument {
    serde_json::from_value(json!({
        "version":"1.2", "responsive":true,
        "children":[{"type":"rectangle","id":"empty","width":20,"height":20}]
    }))
    .unwrap()
}

#[async_trait(?Send)]
impl ImageResolver for TrustedRelativeImages {
    fn admission(&self, source: &str) -> Result<Option<ImageAdmission>, String> {
        Ok((source == "hero.png").then(|| ImageAdmission {
            key: "asset:trusted:hero".into(),
            request_source: source.into(),
            requires_network: false,
        }))
    }

    async fn resolve(&self, source: &str) -> Result<Vec<u8>, String> {
        self.requests.borrow_mut().push(source.to_owned());
        Ok(vec![1, 2, 3])
    }
}

#[derive(Default)]
struct RegisteredCapture {
    images: std::collections::BTreeMap<String, std::sync::Arc<Vec<u8>>>,
    served: Vec<ImageSource>,
}
impl RenderBackend for RegisteredCapture {
    type Surface = ();
    fn new_surface(&mut self, _: Size) {}
    fn begin_frame(&mut self, _: &mut (), _: u32) {}
    fn end_frame(&mut self, _: &mut ()) {}
    fn push_clip(&mut self, _: Rect) {}
    fn push_transform(&mut self, _: &Affine2) {}
    fn pop(&mut self) {}
    fn push_layer(&mut self, _: Rect) {}
    fn pop_layer(&mut self) {}
    fn apply_blur(&mut self, _: f32) {}
    fn apply_shadow(&mut self, _: &ShadowSpec) {}
    fn draw(&mut self, op: &DrawOp) {
        if let DrawOp::Image {
            source: ImageSource::Url(key),
            ..
        } = op
        {
            if let Some(bytes) = self.images.get(key) {
                self.served.push(ImageSource::Bytes(bytes.clone()));
                return;
            }
        }
        if let DrawOp::Image { source, .. } = op {
            self.served.push(source.clone());
        }
    }
    fn register_image(&mut self, key: &str, bytes: &[u8]) -> Result<(), DecodeError> {
        self.images
            .insert(key.to_owned(), std::sync::Arc::new(bytes.to_vec()));
        Ok(())
    }
    fn release_image(&mut self, key: &str) {
        self.images.remove(key);
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
    let mut backend = RegisteredCapture::default();
    runtime.prepare_frame(&mut backend, 0);
    assert_eq!(
        runtime
            .image_store
            .state("https://example.invalid/hero.png"),
        Some(jian_core::render::image_store::ImageState::Registered)
    );
    let registered_draws = collect_draws_with_state(
        runtime.document.as_ref().unwrap(),
        &runtime.layout,
        &runtime.state,
    );
    for draw in &registered_draws {
        backend.draw(draw);
    }
    assert!(backend.served.iter().any(
        |source| matches!(source, ImageSource::Bytes(bytes) if bytes.as_slice() == [1, 2, 3])
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
fn host_admitted_relative_image_uses_resolver_store_prepare_and_keyed_draw() {
    let requests = Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut runtime = Runtime::new();
    runtime.image_resolver = Rc::new(TrustedRelativeImages {
        requests: requests.clone(),
    });
    runtime
        .load_str(
            r#"{"version":"1.2","responsive":true,"children":[{"type":"image","id":"hero","src":"hero.png","width":20,"height":20}]}"#,
        )
        .unwrap();
    runtime.build_layout((100.0, 100.0)).unwrap();

    assert_eq!(
        runtime.state.image_key("hero.png").as_deref(),
        Some("asset:trusted:hero")
    );
    assert_eq!(
        runtime.image_store.state("asset:trusted:hero"),
        Some(ImageState::Pending)
    );

    runtime.pump(1);
    runtime.pump(2);
    assert_eq!(requests.borrow().as_slice(), ["hero.png"]);

    let mut backend = RegisteredCapture::default();
    runtime.prepare_frame(&mut backend, 0);
    assert_eq!(
        runtime.image_store.state("asset:trusted:hero"),
        Some(ImageState::Registered)
    );
    let draws = collect_draws_with_state(
        runtime.document.as_ref().unwrap(),
        &runtime.layout,
        &runtime.state,
    );
    assert!(draws.iter().any(|draw| matches!(draw, DrawOp::Image {
        source: ImageSource::Url(key), ..
    } if key == "asset:trusted:hero")));
}

#[test]
fn pending_same_key_reload_transfers_the_original_resolver_task() {
    let resolver = ControlledImages::default();
    let mut runtime = Runtime::new();
    runtime.image_resolver = Rc::new(resolver.clone());
    runtime
        .replace_document(image_document("hero.png"))
        .unwrap();
    runtime.pump(1);
    assert_eq!(resolver.0.calls.get(), 1);

    runtime
        .replace_document(image_document("hero.png"))
        .unwrap();
    assert_eq!(resolver.0.calls.get(), 1);
    assert_eq!(resolver.0.aborted.get(), 0);

    resolver.complete(0, vec![1, 2, 3]);
    runtime.pump(2);
    runtime.pump(3);
    assert_eq!(
        runtime.image_store.state("asset:hero.png"),
        Some(ImageState::Bytes)
    );
    assert_eq!(resolver.0.calls.get(), 1);
    assert_eq!(resolver.0.aborted.get(), 0);
}

#[test]
fn reload_removing_image_cancels_original_resolver_task() {
    let resolver = ControlledImages::default();
    let mut runtime = Runtime::new();
    runtime.image_resolver = Rc::new(resolver.clone());
    runtime
        .replace_document(image_document("hero.png"))
        .unwrap();
    runtime.pump(1);
    runtime.replace_document(empty_document()).unwrap();

    assert_eq!(resolver.0.calls.get(), 1);
    assert_eq!(resolver.0.aborted.get(), 1);
    assert_eq!(runtime.image_store.state("asset:hero.png"), None);
}

#[test]
fn stale_queued_completion_cannot_satisfy_a_new_same_key_request() {
    let resolver = ControlledImages::default();
    let mut runtime = Runtime::new();
    runtime.image_resolver = Rc::new(resolver.clone());
    runtime
        .replace_document(image_document("hero.png"))
        .unwrap();
    runtime.pump(1);
    resolver.complete(0, vec![1, 2, 3]);
    // Polls the resolver and queues its completion after the completion-drain
    // phase, deliberately leaving it queued for the reload sequence.
    runtime.pump(2);

    runtime.replace_document(empty_document()).unwrap();
    runtime
        .replace_document(image_document("hero.png"))
        .unwrap();
    runtime.pump(3);
    assert_eq!(resolver.0.calls.get(), 2);
    assert_eq!(
        runtime.image_store.state("asset:hero.png"),
        Some(ImageState::Pending)
    );

    resolver.complete(1, vec![4, 5, 6]);
    runtime.pump(4);
    runtime.pump(5);
    assert_eq!(
        runtime.image_store.state("asset:hero.png"),
        Some(ImageState::Bytes)
    );
}

#[test]
fn host_admitted_image_is_pending_for_non_responsive_document() {
    let requests = Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut runtime = Runtime::new();
    runtime.image_resolver = Rc::new(TrustedRelativeImages { requests });
    runtime
        .load_str(
            r#"{"version":"1.2","children":[{"type":"image","id":"hero","src":"hero.png","width":20,"height":20}]}"#,
        )
        .unwrap();

    assert_eq!(
        runtime.state.image_key("hero.png").as_deref(),
        Some("asset:trusted:hero")
    );
    assert_eq!(
        runtime.image_store.state("asset:trusted:hero"),
        Some(ImageState::Pending)
    );
}

#[test]
fn reload_merges_page_and_self_per_page_key_and_replaces_vars_from_staged_doc() {
    let before: PenDocument = serde_json::from_value(json!({
        "version":"1.2", "responsive":true,
        "variables":{"accent":{"type":"string","value":"old-default"}},
        "pages":[{"id":"main","name":"Main","state":{"count":{"type":"int","default":1}},
            "children":[{"type":"rectangle","id":"card","width":10,"height":10,
                "state":{"label":{"type":"string","default":"old"}}}]},
            {"id":"other","name":"Other","state":{"count":{"type":"string","default":"old-other"}},
             "children":[{"type":"rectangle","id":"card2","width":10,"height":10,
                "state":{"label":{"type":"string","default":"old-2"}}}]}], "children":[]
    }))
    .unwrap();
    let mut runtime = Runtime::new_from_document(before).unwrap();
    runtime.state.page_set("main", "count", json!(7));
    runtime.state.self_set("main", "card", "label", json!(99));
    runtime.state.vars_set("accent", json!("live-edit"));
    runtime.state.page_set("other", "count", json!("bad"));
    runtime
        .state
        .self_set("other", "card2", "label", json!(false));
    let after: PenDocument = serde_json::from_value(json!({
        "version":"1.2", "responsive":true,
        "variables":{"accent":{"type":"string","value":"new-default"}},
        "pages":[{"id":"main","name":"Main","state":{"count":{"type":"int","default":2}},
            "children":[{"type":"rectangle","id":"card","width":10,"height":10,
                "state":{"label":{"type":"string","default":"new"}}}]},
            {"id":"other","name":"Other","state":{"count":{"type":"int","default":8}},
             "children":[{"type":"rectangle","id":"card2","width":10,"height":10,
                "state":{"label":{"type":"string","default":"new-2"}}}]}], "children":[]
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
    assert_eq!(
        runtime.state.page_get("other", "count").unwrap().as_i64(),
        Some(8)
    );
    assert_eq!(
        runtime
            .state
            .self_get("other", "card2", "label")
            .unwrap()
            .as_str(),
        Some("new-2")
    );
    assert!(runtime
        .load_warnings()
        .iter()
        .any(|warning| warning.contains("$self[main/card]")));
}

#[test]
fn relative_image_draw_uses_canonical_key_and_non_image_urls_are_not_admitted() {
    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("hero.bin");
    std::fs::write(&image_path, [1, 2, 3]).unwrap();
    let schema: PenDocument = serde_json::from_value(json!({
        "version":"1.2", "responsive":true,
        "app":{"name":"x","version":"1","id":"x","capabilities":["network"]},
        "children":[{"type":"image","id":"hero","src":"hero.bin","width":20,"height":20},
          {"type":"rectangle","id":"button","width":20,"height":20,
           "events":{"onTap":[{"fetch":{"url":"'https://api.example/x'"}}]}}]
    }))
    .unwrap();
    let mut runtime = Runtime::new_from_document(schema).unwrap();
    runtime.set_image_document_dir(dir.path());
    runtime.build_layout((100.0, 100.0)).unwrap();
    let canonical = image_path
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let draws = collect_draws_with_state(
        runtime.document.as_ref().unwrap(),
        &runtime.layout,
        &runtime.state,
    );
    assert!(draws.iter().any(|draw| matches!(draw, DrawOp::Image { source: ImageSource::Url(key), .. } if key == &canonical)));
    assert!(runtime.state.image_key("'https://api.example/x'").is_none());
}

struct OversizedImages;
#[async_trait(?Send)]
impl ImageResolver for OversizedImages {
    async fn resolve(&self, _: &str) -> Result<Vec<u8>, String> {
        Ok(vec![0; 64 * 1024 * 1024 + 1])
    }
}

#[test]
fn oversized_resolver_completion_warns_through_runtime_pump() {
    let schema: PenDocument = serde_json::from_value(json!({
        "version":"1.2", "responsive":true,
        "app":{"name":"x","version":"1","id":"x","capabilities":["network"]},
        "children":[{"type":"image","id":"hero","src":"https://example.invalid/large.png","width":1,"height":1}]
    })).unwrap();
    let mut runtime = Runtime::new_from_document(schema).unwrap();
    runtime.image_resolver = Rc::new(OversizedImages);
    runtime.pump(1);
    runtime.pump(2);
    assert!(runtime
        .load_warnings()
        .iter()
        .any(|warning| warning.contains("64 MiB")));
}
