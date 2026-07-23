//! Sibling unit tests for `layout_scene.rs` (800-line cap
//! convention).

use super::*;

#[test]
fn empty_scene_has_no_active_page() {
    let scene = LayoutScene::default();
    assert!(scene.pages.is_empty());
    assert!(scene.active_page().is_none());
}

#[test]
fn active_page_indexes_into_pages() {
    let scene = LayoutScene {
        pages: vec![
            ScenePage {
                id: "a".into(),
                name: "A".into(),
                children: Vec::new(),
            },
            ScenePage {
                id: "b".into(),
                name: "B".into(),
                children: Vec::new(),
            },
        ],
        active_page_index: 1,
    };
    assert_eq!(scene.active_page().map(|p| p.id.as_str()), Some("b"));
}

#[test]
fn find_locates_a_nested_node() {
    let mut leaf = SceneNode::leaf("deep", NodeKind::Rect);
    leaf.bounds = Rect::xywh(0.0, 0.0, 10.0, 10.0);
    let mut group = SceneNode::leaf("g", NodeKind::Group);
    group.children = vec![leaf];
    let page = ScenePage {
        id: "p".into(),
        name: "P".into(),
        children: vec![group],
    };
    assert_eq!(page.find("deep").map(|n| n.id.as_str()), Some("deep"));
    assert!(page.find("missing").is_none());
}

#[test]
fn aggregate_bounds_unions_children_for_unbounded_container() {
    let mut a = SceneNode::leaf("a", NodeKind::Rect);
    a.bounds = Rect::xywh(10.0, 10.0, 20.0, 20.0);
    let mut b = SceneNode::leaf("b", NodeKind::Rect);
    b.bounds = Rect::xywh(50.0, 5.0, 10.0, 40.0);
    let mut group = SceneNode::leaf("g", NodeKind::Group);
    group.children = vec![a, b];
    // Unbounded group → union of children: x 10..60, y 5..45.
    assert_eq!(group.aggregate_bounds(), Rect::xywh(10.0, 5.0, 50.0, 40.0));
}

#[test]
fn aggregate_bounds_uses_precomputed_unbounded_container_bounds() {
    let mut child = SceneNode::leaf("child", NodeKind::Rect);
    child.bounds = Rect::xywh(10.0, 10.0, 20.0, 20.0);
    let mut group = SceneNode::leaf("g", NodeKind::Group);
    group.children = vec![child];
    group.aggregate_bounds_cache = Rect::xywh(1.0, 2.0, 300.0, 400.0);

    assert_eq!(
        group.aggregate_bounds(),
        Rect::xywh(1.0, 2.0, 300.0, 400.0),
        "layout-built scenes should answer aggregate bounds from the cached subtree rect"
    );
}

#[test]
fn aggregate_bounds_keeps_own_bounds_when_bounded() {
    let mut frame = SceneNode::leaf("f", NodeKind::Frame);
    frame.bounds = Rect::xywh(0.0, 0.0, 100.0, 200.0);
    let mut child = SceneNode::leaf("c", NodeKind::Rect);
    child.bounds = Rect::xywh(0.0, 0.0, 999.0, 999.0);
    frame.children = vec![child];
    assert_eq!(frame.aggregate_bounds(), Rect::xywh(0.0, 0.0, 100.0, 200.0));
}

#[test]
fn translate_nodes_moves_matching_subtree_once() {
    let mut child = SceneNode::leaf("child", NodeKind::Rect);
    child.bounds = Rect::xywh(10.0, 20.0, 30.0, 40.0);
    let mut parent = SceneNode::leaf("parent", NodeKind::Group);
    parent.bounds = Rect::xywh(0.0, 0.0, 100.0, 100.0);
    parent.children = vec![child];
    let mut scene = LayoutScene {
        pages: vec![ScenePage {
            id: "p".into(),
            name: "P".into(),
            children: vec![parent],
        }],
        active_page_index: 0,
    };

    assert!(scene.translate_nodes(&["parent".into(), "child".into()], 5.0, 7.0));
    let page = scene.active_page().expect("active page");
    let parent = page.find("parent").expect("parent");
    let child = page.find("child").expect("child");
    assert_eq!(parent.bounds.origin, Point2D::new(5.0, 7.0));
    assert_eq!(child.bounds.origin, Point2D::new(15.0, 27.0));
}

#[test]
fn translate_nodes_moves_path_absolute_geometry() {
    let mut path = SceneNode::leaf("path", NodeKind::Path);
    path.bounds = Rect::xywh(1.0, 2.0, 30.0, 40.0);
    path.points = vec![Point2D::new(3.0, 4.0)];
    path.path_anchors = vec![SceneAnchor {
        pos: Point2D::new(5.0, 6.0),
        handle_in: Some(Point2D::new(7.0, 8.0)),
        handle_out: Some(Point2D::new(9.0, 10.0)),
        point_type: ScenePointType::Corner,
    }];
    let mut scene = LayoutScene {
        pages: vec![ScenePage {
            id: "p".into(),
            name: "P".into(),
            children: vec![path],
        }],
        active_page_index: 0,
    };

    assert!(scene.translate_nodes(&["path".into()], 11.0, 13.0));
    let path = scene.active_page().and_then(|p| p.find("path")).unwrap();
    assert_eq!(path.bounds.origin, Point2D::new(12.0, 15.0));
    assert_eq!(path.points[0], Point2D::new(14.0, 17.0));
    assert_eq!(path.path_anchors[0].pos, Point2D::new(16.0, 19.0));
    assert_eq!(
        path.path_anchors[0].handle_in,
        Some(Point2D::new(18.0, 21.0))
    );
    assert_eq!(
        path.path_anchors[0].handle_out,
        Some(Point2D::new(20.0, 23.0))
    );
}

#[test]
fn set_node_fill_patches_matching_node_and_reports_match() {
    let child = SceneNode::leaf("child", NodeKind::Rect);
    let mut parent = SceneNode::leaf("parent", NodeKind::Group);
    parent.children = vec![child];
    let mut scene = LayoutScene {
        pages: vec![ScenePage {
            id: "p".into(),
            name: "P".into(),
            children: vec![parent],
        }],
        active_page_index: 0,
    };

    let red = Color {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    // Patches a nested node (opacity 1.0 → no alpha scaling).
    assert!(scene.set_node_fill(&["child".into()], red));
    let child = scene.active_page().and_then(|p| p.find("child")).unwrap();
    assert_eq!(child.fill, Some(red));
    // A non-matching id reports no match (caller then rebuilds).
    assert!(!scene.set_node_fill(&["missing".into()], red));
}

#[test]
fn legacy_set_node_fill_bakes_node_opacity_into_alpha() {
    let mut node = SceneNode::leaf("n", NodeKind::Rect);
    node.opacity = 0.5;
    let mut scene = LayoutScene {
        pages: vec![ScenePage {
            id: "p".into(),
            name: "P".into(),
            children: vec![node],
        }],
        active_page_index: 0,
    };

    let opaque = Color {
        r: 0.2,
        g: 0.4,
        b: 0.6,
        a: 1.0,
    };
    assert!(scene.set_node_fill(&["n".into()], opaque));
    let fill = scene
        .active_page()
        .and_then(|p| p.find("n"))
        .unwrap()
        .fill
        .unwrap();
    // Alpha scaled by the node's cumulative opacity; rgb unchanged.
    assert_eq!(fill.a, 0.5);
    assert_eq!((fill.r, fill.g, fill.b), (0.2, 0.4, 0.6));
}

#[test]
fn layered_set_node_fill_keeps_authored_alpha_for_group_compositing() {
    let mut node = SceneNode::leaf("n", NodeKind::Rect);
    node.opacity = 0.5;
    node.fill_layers = vec![SceneFillLayer::Solid {
        color: Color::BLACK,
        blend_mode: ImageBlendMode::Multiply,
    }];
    let mut scene = LayoutScene {
        pages: vec![ScenePage {
            id: "p".into(),
            name: "P".into(),
            children: vec![node],
        }],
        active_page_index: 0,
    };
    let opaque = Color {
        r: 0.2,
        g: 0.4,
        b: 0.6,
        a: 1.0,
    };

    assert!(scene.set_node_fill(&["n".into()], opaque));
    let node = scene.active_page().and_then(|p| p.find("n")).unwrap();
    assert_eq!(node.fill.map(|fill| fill.a), Some(0.5));
    assert!(matches!(
        node.fill_layers.as_slice(),
        [SceneFillLayer::Solid { color, blend_mode }]
            if *color == opaque && *blend_mode == ImageBlendMode::Multiply
    ));
}

#[test]
fn set_node_stroke_color_only_patches_an_existing_stroke() {
    let bare = SceneNode::leaf("bare", NodeKind::Rect);
    let mut styled = SceneNode::leaf("styled", NodeKind::Rect);
    styled.stroke = Some(SceneStroke {
        color: Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        },
        width: 2.0,
        sides: None,
        align: SceneStrokeAlign::Center,
    });
    let mut scene = LayoutScene {
        pages: vec![ScenePage {
            id: "p".into(),
            name: "P".into(),
            children: vec![bare, styled],
        }],
        active_page_index: 0,
    };

    let blue = Color {
        r: 0.0,
        g: 0.0,
        b: 1.0,
        a: 1.0,
    };
    // No stroke → width unknown → not patchable, caller must rebuild.
    assert!(!scene.set_node_stroke_color(&["bare".into()], blue));
    // Existing stroke → colour repainted, width preserved.
    assert!(scene.set_node_stroke_color(&["styled".into()], blue));
    let stroke = scene
        .active_page()
        .and_then(|p| p.find("styled"))
        .unwrap()
        .stroke
        .as_ref()
        .unwrap();
    assert_eq!(stroke.color, blue);
    assert_eq!(stroke.width, 2.0);
}

#[test]
fn leaf_node_clears_paint_fields() {
    let n = SceneNode::leaf("n1", NodeKind::Rect);
    assert_eq!(n.bounds, Rect::ZERO);
    assert!(n.fill.is_none());
    assert!(n.stroke.is_none());
    assert!(n.children.is_empty());
    assert_eq!(n.fill_type, SceneFillType::Solid);
}

#[test]
fn content_bounds_unions_top_level_nodes() {
    let mut a = SceneNode::leaf("a", NodeKind::Rect);
    a.bounds = Rect::xywh(10.0, 20.0, 30.0, 40.0); // → x[10,40] y[20,60]
    let mut b = SceneNode::leaf("b", NodeKind::Rect);
    b.bounds = Rect::xywh(100.0, 0.0, 50.0, 10.0); // → x[100,150] y[0,10]
    let scene = LayoutScene {
        pages: vec![ScenePage {
            id: "p".into(),
            name: "P".into(),
            children: vec![a, b],
        }],
        active_page_index: 0,
    };
    let bounds = scene.content_bounds().expect("non-empty page has bounds");
    // Union: x[10,150] y[0,60] → origin (10,0) size (140,60).
    assert_eq!(bounds, Rect::xywh(10.0, 0.0, 140.0, 60.0));
}

#[test]
fn content_bounds_none_for_empty_page() {
    let scene = LayoutScene {
        pages: vec![ScenePage {
            id: "p".into(),
            name: "P".into(),
            children: vec![],
        }],
        active_page_index: 0,
    };
    assert!(scene.content_bounds().is_none());
}

#[test]
fn visual_bounds_include_only_unclipped_descendant_overflow() {
    let mut child = SceneNode::leaf("overflow", NodeKind::Rect);
    child.bounds = Rect::xywh(250.0, 30.0, 50.0, 20.0);

    let mut hidden_child = SceneNode::leaf("hidden-overflow", NodeKind::Rect);
    hidden_child.bounds = Rect::xywh(900.0, 30.0, 50.0, 20.0);
    hidden_child.hidden = true;

    let mut open = SceneNode::leaf("open", NodeKind::Frame);
    open.bounds = Rect::xywh(0.0, 0.0, 100.0, 100.0);
    open.children.push(child);
    open.children.push(hidden_child.clone());
    assert_eq!(open.aggregate_bounds(), open.bounds);
    assert_eq!(open.visual_bounds(), Rect::xywh(0.0, 0.0, 300.0, 100.0));

    let mut clipped = open.clone();
    clipped.clip_content = true;
    assert_eq!(clipped.aggregate_bounds(), clipped.bounds);
    assert_eq!(clipped.visual_bounds(), clipped.bounds);
    let clipped_bounds = clipped.bounds;

    let scene_for = |node| LayoutScene {
        pages: vec![ScenePage {
            id: "p".into(),
            name: "P".into(),
            children: vec![node],
        }],
        active_page_index: 0,
    };
    assert_eq!(
        scene_for(open).content_bounds(),
        Some(Rect::xywh(0.0, 0.0, 300.0, 100.0))
    );
    assert_eq!(scene_for(clipped).content_bounds(), Some(clipped_bounds));
    assert_eq!(scene_for(hidden_child).content_bounds(), None);
}
