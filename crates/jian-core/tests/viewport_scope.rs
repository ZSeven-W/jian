use jian_core::expression::Expression;
use jian_core::Runtime;
use jian_ops_schema::document::PenDocument;

#[test]
fn responsive_viewport_scope_tracks_runtime_resize_but_legacy_stays_unknown() {
    let responsive: PenDocument =
        serde_json::from_str(r#"{"version":"1.2","responsive":true,"children":[]}"#).unwrap();
    let mut runtime = Runtime::new_from_document(responsive).unwrap();
    let expression = Expression::compile("$viewport.width").unwrap();
    assert_eq!(
        expression.eval(&runtime.state, None, None).0.as_f64(),
        Some(800.0)
    );
    runtime.set_viewport_size((320.0, 480.0));
    assert_eq!(
        expression.eval(&runtime.state, None, None).0.as_f64(),
        Some(320.0)
    );

    let legacy: PenDocument = serde_json::from_str(r#"{"version":"1.2","children":[]}"#).unwrap();
    let runtime = Runtime::new_from_document(legacy).unwrap();
    assert!(expression.eval(&runtime.state, None, None).0.is_null());
}
