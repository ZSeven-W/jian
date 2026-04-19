//! Verify that v0.x real files load without any Jian extensions present,
//! and all extension fields default to None.

use jian_ops_schema::{load_str, PenDocument};

fn load(path: &str) -> PenDocument {
    let raw = std::fs::read_to_string(format!(
        "{}/tests/corpus/{}",
        env!("CARGO_MANIFEST_DIR"),
        path
    ))
    .unwrap();
    let r = load_str(&raw).unwrap_or_else(|e| panic!("compat load {} failed: {}", path, e));
    r.value
}

#[test]
fn v0_minimal_no_extensions() {
    let d = load("minimal.op");
    assert!(d.app.is_none());
    assert!(d.routes.is_none());
    assert!(d.state.is_none());
    assert!(d.lifecycle.is_none());
    assert!(d.logic_modules.is_none());
    assert!(d.format_version.is_none());
    assert_eq!(d.version, "0.8.0");
}

#[test]
fn v0_rectangle_no_extensions() {
    let d = load("rectangle.op");
    assert!(d.children.len() == 1);
    assert!(d.state.is_none());
}

#[test]
fn v0_nested_frame_no_extensions() {
    let d = load("nested-frame.op");
    if let jian_ops_schema::node::PenNode::Frame(ref f) = d.children[0] {
        assert!(f.events.is_none());
        assert!(f.bindings.is_none());
        assert!(f.state.is_none());
    } else {
        panic!("expected Frame");
    }
}

#[test]
fn v0_with_variables_no_extensions() {
    let d = load("with-variables.op");
    assert!(d.variables.is_some());
    assert!(d.state.is_none());
    assert!(d.app.is_none());
}
