use jian_core::document::NodeTree;
use jian_core::geometry::{point, rect};
use jian_core::spatial::{NodeBBox, SpatialIndex};
use jian_ops_schema::node::PenNode;
use serde_json::json;

fn node_rect(id: &str) -> PenNode {
    serde_json::from_value(json!({"type":"rectangle","id":id})).unwrap()
}

#[test]
fn hit_inside_only() {
    let mut tree = NodeTree::new();
    let a = tree.insert_subtree(node_rect("a"), None);
    let b = tree.insert_subtree(node_rect("b"), None);

    let mut idx = SpatialIndex::new();
    idx.rebuild([
        NodeBBox {
            key: a,
            rect: rect(0.0, 0.0, 100.0, 100.0),
        },
        NodeBBox {
            key: b,
            rect: rect(200.0, 0.0, 100.0, 100.0),
        },
    ]);

    assert_eq!(idx.hit(point(50.0, 50.0)), vec![a]);
    assert_eq!(idx.hit(point(250.0, 50.0)), vec![b]);
    assert!(idx.hit(point(150.0, 50.0)).is_empty());
}

#[test]
fn rect_query_returns_overlapping() {
    let mut tree = NodeTree::new();
    let a = tree.insert_subtree(node_rect("a"), None);
    let b = tree.insert_subtree(node_rect("b"), None);
    let c = tree.insert_subtree(node_rect("c"), None);

    let mut idx = SpatialIndex::new();
    idx.rebuild([
        NodeBBox {
            key: a,
            rect: rect(0.0, 0.0, 100.0, 100.0),
        },
        NodeBBox {
            key: b,
            rect: rect(120.0, 0.0, 100.0, 100.0),
        },
        NodeBBox {
            key: c,
            rect: rect(50.0, 50.0, 100.0, 100.0),
        },
    ]);

    // viewport strictly narrower than 80 so the right edge at 40+79=119 does
    // not touch b's left edge at x=120 (rstar envelope intersection is inclusive).
    let viewport = rect(40.0, 40.0, 79.0, 80.0);
    let got = idx.query_rect(viewport);
    assert_eq!(got.len(), 2); // a and c intersect, b does not
}
