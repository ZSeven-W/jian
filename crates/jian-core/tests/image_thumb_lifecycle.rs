use jian_core::Runtime;

const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xd9];

fn document_with_thumb(paint_id: u64) -> String {
    format!(r#"{{"version":"1.0.0","imageThumbs":{{"{paint_id}":"/9j/2Q=="}},"children":[]}}"#)
}

#[test]
fn runtime_document_transitions_activate_pending_thumbnail_seeds() {
    let old_id = 9_200_001;
    let initial_id = 9_200_002;
    let replacement_id = 9_200_003;
    let load_str_id = 9_200_004;
    jian_ops_schema::image_thumbs::clear_registry();
    jian_ops_schema::image_thumbs::store_thumb(old_id, vec![1, 2, 3]);

    let loaded = jian_ops_schema::load_str(&document_with_thumb(initial_id))
        .expect("parse initial runtime document");
    assert!(jian_ops_schema::image_thumbs::thumb_for(old_id).is_some());
    assert!(jian_ops_schema::image_thumbs::thumb_for(initial_id).is_none());

    let mut runtime = Runtime::new_from_document(loaded.value).expect("build runtime");
    assert!(jian_ops_schema::image_thumbs::thumb_for(old_id).is_none());
    assert_eq!(
        &*jian_ops_schema::image_thumbs::thumb_for(initial_id).expect("initial seed activated"),
        JPEG
    );

    let replacement = jian_ops_schema::load_str(&document_with_thumb(replacement_id))
        .expect("parse replacement runtime document");
    runtime
        .replace_document(replacement.value)
        .expect("replace runtime document");
    assert!(jian_ops_schema::image_thumbs::thumb_for(initial_id).is_none());
    assert_eq!(
        &*jian_ops_schema::image_thumbs::thumb_for(replacement_id)
            .expect("replacement seed activated"),
        JPEG
    );

    runtime
        .load_str(&document_with_thumb(load_str_id))
        .expect("parse and replace through Runtime::load_str");
    assert!(jian_ops_schema::image_thumbs::thumb_for(replacement_id).is_none());
    assert_eq!(
        &*jian_ops_schema::image_thumbs::thumb_for(load_str_id).expect("load_str seed activated"),
        JPEG
    );
}
