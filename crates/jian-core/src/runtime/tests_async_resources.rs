#[test]
fn reload_cancels_pending_fetch_and_compensates_loading_before_merge() {
    struct PendingNetwork;
    #[async_trait::async_trait(?Send)]
    impl crate::action::services::NetworkClient for PendingNetwork {
        async fn request(
            &self,
            _request: crate::action::services::HttpRequest,
        ) -> Result<crate::action::services::HttpResponse, String> {
            std::future::pending().await
        }
    }
    let schema: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","responsive":true,"app":{"name":"t","version":"1","id":"t","capabilities":["network"]},
            "state":{"loading":{"type":"bool","default":false},"failed":{"type":"bool","default":false}},
            "children":[{"type":"rectangle","id":"button","width":30,"height":30,
             "bindings":{"width":"$app.loading ? 90 : 30"},
             "events":{"onTap":[{"fetch":{"url":"'https://example.invalid'","loading":"$app.loading","on_error":[{"set":{"$app.failed":"true"}}]}}]}}]}"#,
        )
        .unwrap();
    let mut runtime = Runtime::new_from_document(schema.clone()).unwrap();
    runtime.network = Rc::new(PendingNetwork);
    runtime.build_layout((100.0, 100.0)).unwrap();
    for phase in [
        crate::gesture::PointerPhase::Down,
        crate::gesture::PointerPhase::Up,
    ] {
        runtime.dispatch_pointer(PointerEvent::simple(
            1,
            phase,
            crate::geometry::point(5.0, 5.0),
        ));
    }
    assert_eq!(
        runtime.state.app_get("loading").unwrap().as_bool(),
        Some(true)
    );
    runtime
        .load_str_and_relayout(&serde_json::to_string(&schema).unwrap())
        .unwrap();
    assert_eq!(
        runtime.state.app_get("loading").unwrap().as_bool(),
        Some(false)
    );
    assert_eq!(
        runtime.state.app_get("failed").unwrap().as_bool(),
        Some(false)
    );
    let button = runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("button")
        .unwrap();
    assert_eq!(runtime.layout.node_rect(button).unwrap().size.width, 30.0);
}

#[test]
fn failed_exact_reload_stage_keeps_pending_fetch_and_loading_state() {
    struct PendingNetwork;
    #[async_trait::async_trait(?Send)]
    impl crate::action::services::NetworkClient for PendingNetwork {
        async fn request(
            &self,
            _request: crate::action::services::HttpRequest,
        ) -> Result<crate::action::services::HttpResponse, String> {
            std::future::pending().await
        }
    }
    let source = r#"{"version":"1.2","responsive":true,
          "app":{"name":"t","version":"1","id":"t","capabilities":["network"]},
          "state":{"loading":{"type":"bool","default":false}},
          "children":[{"type":"rectangle","id":"button","width":30,"height":30,
          "events":{"onTap":[{"fetch":{"url":"'https://example.invalid'","loading":"$app.loading"}}]}}]}"#;
    let mut runtime = Runtime::new();
    runtime.load_str(source).unwrap();
    runtime.network = Rc::new(PendingNetwork);
    runtime.build_layout((100.0, 100.0)).unwrap();
    for phase in [
        crate::gesture::PointerPhase::Down,
        crate::gesture::PointerPhase::Up,
    ] {
        runtime.dispatch_pointer(PointerEvent::simple(
            1,
            phase,
            crate::geometry::point(5.0, 5.0),
        ));
    }
    assert_eq!(
        runtime.state.app_get("loading").unwrap().as_bool(),
        Some(true)
    );
    assert!(!runtime.task_queue.is_empty());

    runtime.layout.inject_staged_build_failure();
    assert!(runtime.load_str_and_relayout(source).is_err());

    assert_eq!(
        runtime.state.app_get("loading").unwrap().as_bool(),
        Some(true)
    );
    assert!(!runtime.task_queue.is_empty());
    assert!(runtime
        .document
        .as_ref()
        .unwrap()
        .tree
        .get("button")
        .is_some());
}

#[test]
fn reload_transfers_surviving_image_owner_before_canceling_stale_request() {
    struct CancellationObserver {
        owner: Rc<RefCell<Option<Rc<Cell<u64>>>>>,
        seen_generation: Rc<Cell<Option<u64>>>,
    }

    impl Drop for CancellationObserver {
        fn drop(&mut self) {
            if let Some(owner) = self.owner.borrow().as_ref() {
                self.seen_generation.set(Some(owner.get()));
            }
        }
    }

    struct PendingImages {
        owner: Rc<RefCell<Option<Rc<Cell<u64>>>>>,
        seen_generation: Rc<Cell<Option<u64>>>,
    }

    #[async_trait::async_trait(?Send)]
    impl crate::render::image_store::ImageResolver for PendingImages {
        async fn resolve(&self, url: &str) -> Result<Vec<u8>, String> {
            let observer = url.ends_with("stale.png").then(|| CancellationObserver {
                owner: self.owner.clone(),
                seen_generation: self.seen_generation.clone(),
            });
            std::future::pending::<()>().await;
            drop(observer);
            Ok(Vec::new())
        }
    }

    fn images(sources: &[&str]) -> PenDocument {
        serde_json::from_value(serde_json::json!({
            "version": "1.2",
            "app": {
                "name": "images",
                "version": "1",
                "id": "images",
                "capabilities": ["network"]
            },
            "children": sources
                .iter()
                .enumerate()
                .map(|(index, source)| serde_json::json!({
                    "type": "image",
                    "id": format!("image-{index}"),
                    "src": source,
                    "width": 10,
                    "height": 10
                }))
                .collect::<Vec<_>>()
        }))
        .unwrap()
    }

    let keep = "https://example.invalid/keep.png";
    let stale = "https://example.invalid/stale.png";
    let owner = Rc::new(RefCell::new(None));
    let seen_generation = Rc::new(Cell::new(None));
    let mut runtime = Runtime::new_from_document(images(&[keep, stale])).unwrap();
    runtime.image_resolver = Rc::new(PendingImages {
        owner: owner.clone(),
        seen_generation: seen_generation.clone(),
    });
    runtime.pump(0);
    *owner.borrow_mut() = Some(
        runtime
            .image_requests
            .get(keep)
            .unwrap()
            .owner_generation
            .clone(),
    );

    runtime.replace_document(images(&[keep])).unwrap();

    assert_eq!(seen_generation.get(), Some(runtime.document_generation));
    assert_eq!(
        runtime
            .image_requests
            .get(keep)
            .unwrap()
            .owner_generation
            .get(),
        runtime.document_generation
    );
    assert!(!runtime.image_requests.contains_key(stale));
}

#[test]
fn runtime_drop_aborts_websocket_sessions_synchronously() {
    struct Session(Rc<Cell<bool>>);
    #[async_trait::async_trait(?Send)]
    impl crate::action::services::WebSocketSession for Session {
        fn abort(&self) {
            self.0.set(true);
        }
        async fn send(&self, _: String) -> Result<(), String> {
            Ok(())
        }
        async fn close(&self) -> Result<(), String> {
            Ok(())
        }
    }
    let aborted = Rc::new(Cell::new(false));
    {
        let runtime = Runtime::new();
        runtime.ws_sessions.borrow_mut().insert(
            "socket".into(),
            crate::action::context::WsHandle {
                session: Rc::new(Session(aborted.clone())),
                on_message: None,
                generation: 0,
            },
        );
    }
    assert!(aborted.get());
}

#[test]
fn responsive_storage_read_hydrates_through_expression_and_pump() {
    struct Store;
    #[async_trait::async_trait(?Send)]
    impl StorageBackend for Store {
        async fn get(
            &self,
            key: &str,
        ) -> Result<Option<serde_json::Value>, crate::action::services::ServiceError> {
            Ok((key == "theme").then(|| serde_json::json!("dark")))
        }
        async fn set(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<(), crate::action::services::ServiceError> {
            Ok(())
        }
        async fn delete(&self, _: &str) -> Result<(), crate::action::services::ServiceError> {
            Ok(())
        }
        async fn clear(&self) -> Result<(), crate::action::services::ServiceError> {
            Ok(())
        }
        async fn keys(&self) -> Result<Vec<String>, crate::action::services::ServiceError> {
            Ok(Vec::new())
        }
    }
    let schema: PenDocument = serde_json::from_str(
            r#"{"version":"1.2","responsive":true,"app":{"name":"t","version":"1","id":"t","capabilities":["storage"]},"children":[]}"#,
        )
        .unwrap();
    let mut runtime = Runtime::new_from_document(schema).unwrap();
    runtime.storage = Rc::new(Store);
    let expression = crate::expression::Expression::compile("$storage.theme").unwrap();
    assert!(expression.eval(&runtime.state, None, None).0.is_null());
    runtime.pump(1);
    assert_eq!(
        expression.eval(&runtime.state, None, None).0.as_str(),
        Some("dark")
    );
}
