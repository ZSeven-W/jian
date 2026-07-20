use super::*;
use jian_ops_schema::node::PenNode;
use jian_ops_schema::pack::initial_layout::{InitialLayoutSnapshot, PackedRect};
use jian_ops_schema::pack::manifest::DefaultViewport;
use serde_json::json;
use std::collections::BTreeMap;

fn rect_node(id: &str) -> PenNode {
    serde_json::from_value(json!({"type":"rectangle","id":id})).unwrap()
}

fn frame_node(id: &str, children: Vec<PenNode>) -> PenNode {
    let mut v = json!({"type":"frame","id":id});
    v["children"] = serde_json::Value::Array(
        children
            .into_iter()
            .map(|c| serde_json::to_value(c).unwrap())
            .collect(),
    );
    serde_json::from_value(v).unwrap()
}

fn snapshot(pairs: &[(&str, [f32; 4])]) -> InitialLayoutSnapshot {
    let mut rects = BTreeMap::new();
    for (id, [x, y, w, h]) in pairs {
        rects.insert(
            (*id).to_string(),
            PackedRect {
                x: *x,
                y: *y,
                w: *w,
                h: *h,
            },
        );
    }
    InitialLayoutSnapshot {
        viewport: DefaultViewport {
            width: 800.0,
            height: 600.0,
        },
        rects,
    }
}

#[test]
fn preload_serves_node_rect_without_compute() {
    let mut tree = NodeTree::new();
    tree.insert_subtree(
        frame_node("root", vec![rect_node("a"), rect_node("b")]),
        None,
    );
    let snap = snapshot(&[
        ("a", [10.0, 20.0, 100.0, 50.0]),
        ("b", [10.0, 80.0, 100.0, 50.0]),
    ]);
    let mut engine = LayoutEngine::new();
    let n = engine.preload_initial(&snap, &tree);
    assert_eq!(n, 2);
    assert!(engine.has_preload());

    let key_a = tree.get("a").unwrap();
    let key_b = tree.get("b").unwrap();
    assert_eq!(engine.node_rect(key_a), Some(rect(10.0, 20.0, 100.0, 50.0)));
    assert_eq!(engine.node_rect(key_b), Some(rect(10.0, 80.0, 100.0, 50.0)));
}

#[test]
fn preload_drops_ids_absent_from_doc() {
    let mut tree = NodeTree::new();
    tree.insert_subtree(rect_node("a"), None);
    let snap = snapshot(&[("a", [1.0, 2.0, 3.0, 4.0]), ("ghost", [9.0, 9.0, 9.0, 9.0])]);
    let mut engine = LayoutEngine::new();
    // Only the doc-resident id resolves; "ghost" is silently
    // dropped (newer doc, older pack — not a panic case).
    assert_eq!(engine.preload_initial(&snap, &tree), 1);
}

#[test]
fn build_clears_preload() {
    let mut tree = NodeTree::new();
    tree.insert_subtree(rect_node("a"), None);
    let snap = snapshot(&[("a", [1.0, 2.0, 3.0, 4.0])]);
    let mut engine = LayoutEngine::new();
    engine.preload_initial(&snap, &tree);
    assert!(engine.has_preload());

    // A real taffy compute supersedes the preload.
    let _ = engine.build(&tree).expect("taffy build");
    assert!(!engine.has_preload());
}

#[test]
fn preload_replaces_prior_snapshot() {
    let mut tree = NodeTree::new();
    tree.insert_subtree(rect_node("a"), None);

    let mut engine = LayoutEngine::new();
    engine.preload_initial(&snapshot(&[("a", [1.0, 2.0, 3.0, 4.0])]), &tree);
    engine.preload_initial(&snapshot(&[("a", [50.0, 60.0, 70.0, 80.0])]), &tree);
    let key_a = tree.get("a").unwrap();
    assert_eq!(engine.node_rect(key_a), Some(rect(50.0, 60.0, 70.0, 80.0)));
}

fn compute_single_child(child: PenNode) -> Rect {
    let root = frame_node("root", vec![child]);
    let mut tree = NodeTree::new();
    tree.insert_subtree(root, None);
    let mut engine = LayoutEngine::new();
    let roots = engine.build(&tree).expect("taffy build");
    let root_id = *roots.first().expect("root id");
    engine.compute(root_id, (400.0, 100.0)).expect("compute");
    let key = tree.get("input").expect("input key");
    engine.node_rect(key).expect("input rect")
}

fn text_input_node(value: serde_json::Value) -> PenNode {
    serde_json::from_value(value).unwrap()
}

#[test]
fn fit_content_text_input_with_leading_icon_measures_input_anatomy() {
    use measure::{EstimateBackend, MeasureBackend, MeasureRequest, StyledRun};

    let input = text_input_node(json!({
        "type":"text_input",
        "id":"input",
        "width":"fit_content",
        "height":"fit_content",
        "placeholder":"Search",
        "leadingIcon":"search",
        "fontSize":14
    }));
    let rect = compute_single_child(input);
    let run = StyledRun {
        text: "Search",
        font_family: None,
        font_size: 14.0,
        font_weight: 400,
        font_style: FontStyleKind::Normal,
        letter_spacing: 0.0,
    };
    let text = EstimateBackend.measure(&MeasureRequest {
        runs: &[run],
        line_height: 0.0,
        max_width: None,
    });

    assert!(
        rect.size.width >= text.width + 36.0,
        "leading-icon text_input should reserve 36px chrome plus text width, got {}",
        rect.size.width
    );
    assert!(
        rect.size.height >= 36.0,
        "text_input height should reserve vertical padding and icon box, got {}",
        rect.size.height
    );
}

#[test]
fn fit_content_text_input_without_icon_measures_horizontal_padding() {
    use measure::{EstimateBackend, MeasureBackend, MeasureRequest, StyledRun};

    let input = text_input_node(json!({
        "type":"text_input",
        "id":"input",
        "width":"fit_content",
        "height":"fit_content",
        "placeholder":"Find",
        "fontSize":14
    }));
    let rect = compute_single_child(input);
    let run = StyledRun {
        text: "Find",
        font_family: None,
        font_size: 14.0,
        font_weight: 400,
        font_style: FontStyleKind::Normal,
        letter_spacing: 0.0,
    };
    let text = EstimateBackend.measure(&MeasureRequest {
        runs: &[run],
        line_height: 0.0,
        max_width: None,
    });

    assert!(
            (rect.size.width - (text.width + 16.0)).abs() <= 0.5,
            "plain text_input should measure exactly 16px horizontal padding plus text width, got {} vs text {}",
            rect.size.width,
            text.width
        );
}

#[test]
fn numeric_sized_text_input_keeps_authored_size() {
    let input = text_input_node(json!({
        "type":"text_input",
        "id":"input",
        "width":120,
        "height":44,
        "placeholder":"A much longer placeholder",
        "leadingIcon":"search",
        "fontSize":14
    }));
    let rect = compute_single_child(input);
    assert_eq!(rect.size.width, 120.0);
    assert_eq!(rect.size.height, 44.0);
}

// Carried over from main's inline module when this file took it over
// (line-height semantics fix); it must not be lost to the split.
#[test]
fn explicit_height_multiline_text_measure_rejects_pixel_like_line_height() {
    let text: PenNode = serde_json::from_value(json!({
        "type":"text",
        "id":"label",
        "width":180,
        "height":52,
        "textGrowth":"fixed-width-height",
        "content":"First line\nSecond line",
        "fontSize":14,
        "lineHeight":17
    }))
    .unwrap();

    let measure = text_measure_for(&text).expect("text measure");
    assert_eq!(
        measure.line_height, 0.0,
        "explicit box height must not make pixel-like lineHeight a multiplier"
    );
    assert_eq!(measure.runs[0].text, "First line\nSecond line");
}
