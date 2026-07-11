use jian_core::render::image_store::{ImageState, ImageStore};

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
fn backend_generation_bump_readmits_registered_images() {
    let mut store = ImageStore::with_budgets(64, 128);
    store.admit_resolver("a", 3);
    store.resolve("a", vec![1, 2, 3]);
    store.mark_registered("a", 0).unwrap();
    assert_eq!(store.state("a"), Some(ImageState::Registered));
    store.backend_generation_changed(1);
    assert_eq!(store.state("a"), Some(ImageState::Pending));
}
