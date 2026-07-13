use jian_core::render::image_store::{
    canonical_url_key, data_url_key, decode_data_url, read_confined_local,
};
use jian_core::render::image_store::{ImageState, ImageStore};
use jian_core::render::{DecodeError, DrawOp, RenderBackend};
use jian_core::{geometry::Affine2, geometry::Rect, geometry::Size};

#[test]
fn reservation_budget_defers_fifo_and_release_promotes_oldest() {
    let mut store = ImageStore::with_budgets(5, 128);
    store.admit_resolver("a", 5);
    store.admit_resolver("b", 3);
    assert_eq!(store.state("a"), Some(ImageState::Pending));
    assert_eq!(store.state("b"), Some(ImageState::Deferred));
    store.fail("a", "failed");
    assert_eq!(store.state("b"), Some(ImageState::Pending));
}

#[test]
fn canonical_keys_and_data_urls_are_bounded_and_stable() {
    let dir = tempfile::tempdir().unwrap();
    let asset = dir.path().join("hero.bin");
    std::fs::write(&asset, b"hero").unwrap();
    assert_eq!(
        canonical_url_key("hero.bin", dir.path()).unwrap(),
        asset.canonicalize().unwrap().to_string_lossy()
    );
    let source = "data:image/png;base64,aGVybw==";
    assert_eq!(decode_data_url(source).unwrap(), b"hero");
    assert_eq!(data_url_key(source), data_url_key(source));
    assert_ne!(
        data_url_key(source),
        data_url_key("data:image/png;base64,b3RoZXI=")
    );
}

#[cfg(unix)]
#[test]
fn local_open_verifies_the_opened_path_is_inside_asset_root() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    assert!(read_confined_local(outside.path(), root.path())
        .unwrap_err()
        .contains("escapes asset root"));
}

#[test]
fn network_revocation_fails_unregistered_but_keeps_registered() {
    let mut store = ImageStore::with_budgets(64, 128);
    store.admit_resolver("pending", 3);
    store.admit_resolver("bytes", 3);
    store.resolve("bytes", vec![1, 2, 3]).unwrap();
    store.admit_resolver("registered", 3);
    store.resolve("registered", vec![1, 2, 3]).unwrap();
    store.mark_registered("registered", 0).unwrap();
    store.admit_resolver("fills-budget", 58);
    store.admit_resolver("deferred", 4);
    assert_eq!(store.state("deferred"), Some(ImageState::Deferred));
    let warnings = store.revoke_network();
    assert_eq!(warnings.len(), 4);
    assert_eq!(store.state("pending"), Some(ImageState::Failed));
    assert_eq!(store.state("bytes"), Some(ImageState::Failed));
    assert_eq!(store.state("registered"), Some(ImageState::Registered));
    assert_eq!(store.state("deferred"), Some(ImageState::Failed));
}

#[test]
fn reload_ownership_retags_survivors_and_releases_stale_entries() {
    let mut store = ImageStore::with_budgets(64, 128);
    for key in ["keep", "stale"] {
        store.admit_resolver(key, 3);
        store.resolve(key, vec![1, 2, 3]).unwrap();
        store.mark_registered(key, 0).unwrap();
    }
    store.begin_reload_ownership();
    store.admit_resolver("keep", 3);
    store.finish_reload_ownership();
    assert_eq!(store.state("keep"), Some(ImageState::Registered));
    assert_eq!(store.state("stale"), None);
    assert!(store.has_pending_work());
}

#[test]
fn backend_generation_bump_readmits_registered_images() {
    let mut store = ImageStore::with_budgets(64 * 1024 * 1024, 128);
    store.admit_resolver("a", 3);
    store.resolve("a", vec![1, 2, 3]).unwrap();
    store.mark_registered("a", 0).unwrap();
    assert_eq!(store.state("a"), Some(ImageState::Registered));
    store.backend_generation_changed(1);
    assert_eq!(store.state("a"), Some(ImageState::Pending));
}

#[test]
fn backend_generation_bump_budgets_remote_refetches_and_defers_fifo() {
    const MAX_TRANSFER: usize = 64 * 1024 * 1024;
    let mut store = ImageStore::with_budgets(MAX_TRANSFER, 128);
    for key in ["first", "second"] {
        store.admit_resolver(key, MAX_TRANSFER);
        assert_eq!(store.state(key), Some(ImageState::Pending));
        store.resolve(key, vec![1, 2, 3]).unwrap();
        store.mark_registered(key, 0).unwrap();
    }

    store.backend_generation_changed(1);

    assert_eq!(store.state("first"), Some(ImageState::Pending));
    assert_eq!(store.state("second"), Some(ImageState::Deferred));
    assert_eq!(store.pending_keys(), ["first"]);

    store.fail("first", "release transfer reservation");
    assert_eq!(store.state("second"), Some(ImageState::Pending));
    assert_eq!(store.pending_keys(), ["second"]);
}

#[test]
fn multiple_inline_images_share_the_aggregate_transfer_budget() {
    let mut store = ImageStore::with_budgets(5, 128);
    store.admit_inline("first", vec![1, 2, 3]);
    store.admit_inline("second", vec![4, 5, 6]);
    assert_eq!(store.state("first"), Some(ImageState::Bytes));
    assert_eq!(store.state("second"), Some(ImageState::Deferred));
    store.release_ref("first");
    assert_eq!(store.state("second"), Some(ImageState::Bytes));
}

#[test]
fn resolved_bytes_deferred_by_actual_size_promote_without_a_second_request() {
    let mut store = ImageStore::with_budgets(5, 128);
    store.admit_resolver("blocker", 4);
    store.admit_resolver("image", 1);
    store.resolve("image", vec![1, 2, 3]).unwrap();
    assert_eq!(store.state("image"), Some(ImageState::Deferred));
    store.fail("blocker", "release reservation");
    assert_eq!(store.state("image"), Some(ImageState::Bytes));
    assert!(!store.pending_keys().iter().any(|key| key == "image"));
}

#[test]
fn oversized_resolver_response_transitions_to_failed() {
    let mut store = ImageStore::with_budgets(65 * 1024 * 1024, 128);
    store.admit_resolver("large", 64 * 1024 * 1024);
    let error = store
        .resolve("large", vec![0; 64 * 1024 * 1024 + 1])
        .unwrap_err();
    assert!(error.contains("64 MiB"));
    assert_eq!(store.state("large"), Some(ImageState::Failed));
}

struct RejectPinnedBackend {
    registered: Vec<String>,
    released: Vec<String>,
}

impl RenderBackend for RejectPinnedBackend {
    type Surface = ();
    fn new_surface(&mut self, _size: Size) {}
    fn begin_frame(&mut self, _surface: &mut (), _clear: u32) {}
    fn end_frame(&mut self, _surface: &mut ()) {}
    fn push_clip(&mut self, _rect: Rect) {}
    fn push_transform(&mut self, _m: &Affine2) {}
    fn pop(&mut self) {}
    fn push_layer(&mut self, _bounds: Rect) {}
    fn pop_layer(&mut self) {}
    fn apply_blur(&mut self, _sigma: f32) {}
    fn apply_shadow(&mut self, _shadow: &jian_core::render::ShadowSpec) {}
    fn draw(&mut self, _op: &DrawOp) {}
    fn register_image(&mut self, key: &str, _bytes: &[u8]) -> Result<(), DecodeError> {
        self.registered.push(key.to_owned());
        Ok(())
    }
    fn release_image(&mut self, key: &str) {
        self.released.push(key.to_owned());
        self.registered.retain(|candidate| candidate != key);
    }
}

#[test]
fn pinned_budget_rejection_releases_registered_backend_image_and_clears_bytes() {
    let mut store = ImageStore::with_budgets(64, 2);
    store.admit_resolver("image", 3);
    store.resolve("image", vec![1, 2, 3]).unwrap();
    let mut backend = RejectPinnedBackend {
        registered: Vec::new(),
        released: Vec::new(),
    };
    let warnings = store.prepare_frame(&mut backend, 0);
    assert_eq!(warnings.len(), 1);
    assert!(backend.registered.is_empty());
    assert_eq!(backend.released, ["image"]);
    assert_eq!(store.state("image"), Some(ImageState::Failed));

    // Re-admission must schedule a new resolve instead of promoting stale bytes.
    store.admit_resolver("image", 3);
    assert_eq!(store.state("image"), Some(ImageState::Pending));
    assert_eq!(store.pending_keys(), ["image"]);
}
