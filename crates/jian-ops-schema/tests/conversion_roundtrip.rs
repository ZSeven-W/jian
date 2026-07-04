use jian_ops_schema::conversion::{ConversionKind, ConversionSpec};
use jian_ops_schema::PenDocument;

#[test]
fn conversion_field_roundtrips() {
    let json = r#"{
      "version": "1",
      "children": [],
      "conversion": { "entries": [{
        "kind": "component", "key": "src/Button.tsx#Button",
        "sourcePath": "src/Button.tsx", "sourceHash": "abc123", "nodeId": "n42",
        "nodeIds": {"button-source": "n42", "source-label": "n43"}
      }]}
    }"#;
    let doc: PenDocument = serde_json::from_str(json).expect("parse");
    let spec: &ConversionSpec = doc.conversion.as_ref().expect("conversion present");
    assert_eq!(spec.entries.len(), 1);
    assert_eq!(spec.entries[0].kind, ConversionKind::Component);
    assert_eq!(spec.entries[0].key, "src/Button.tsx#Button");
    let back = serde_json::to_value(&doc).unwrap();
    assert_eq!(
        back["conversion"]["entries"][0]["sourcePath"],
        "src/Button.tsx"
    );
    assert_eq!(
        back["conversion"]["entries"][0]["nodeIds"]["source-label"],
        "n43"
    );
}

#[test]
fn absent_conversion_stays_absent() {
    let doc: PenDocument = serde_json::from_str(r#"{"version":"1","children":[]}"#).unwrap();
    assert!(doc.conversion.is_none());
    let back = serde_json::to_value(&doc).unwrap();
    assert!(back.get("conversion").is_none());
}
