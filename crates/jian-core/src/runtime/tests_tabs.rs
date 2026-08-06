#[test]
fn tabs_click_switches_bound_panel_spatial_and_focus_together() {
    use crate::geometry::{point, rect};
    use crate::gesture::pointer::{PointerEvent, PointerPhase};
    use std::collections::HashSet;

    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{
              "version":"1.1","formatVersion":"1.1",
              "state":{"active":{"type":"string","default":"second"}},
              "children":[
                {"type":"tabs","id":"tabs","width":300,"height":160,"value":"first",
                 "bindings":{"bind:value":"$state.active"},
                 "tabs":[
                   {"value":"first","label":"First"},
                   {"value":"second","label":"Second"},
                   {"value":"third","label":"Third"}
                 ],
                 "children":[
                   {"type":"frame","id":"first-panel","width":300,"height":128,"children":[
                     {"type":"rectangle","id":"first-action","width":30,"height":20,
                      "gestures":{"focusable":true}}
                   ]},
                   {"type":"frame","id":"second-panel","width":300,"height":128,"children":[
                     {"type":"rectangle","id":"second-action","width":30,"height":20,
                      "gestures":{"focusable":true}}
                   ]},
                   {"type":"frame","id":"third-panel","width":300,"height":128,"children":[
                     {"type":"rectangle","id":"third-action","width":30,"height":20,
                      "gestures":{"focusable":true}}
                   ]}
                 ]}
              ]
            }"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();

    let spatial_ids = |rt: &Runtime| -> HashSet<String> {
        rt.spatial
            .query_rect(rect(-1000.0, -1000.0, 4000.0, 4000.0))
            .into_iter()
            .map(|key| {
                crate::document::tree::node_schema_id(
                    &rt.document.as_ref().unwrap().tree.nodes[key].schema,
                )
                .to_owned()
            })
            .collect()
    };
    let focus_ids = |rt: &Runtime| -> Vec<String> {
        rt.focus
            .chain()
            .iter()
            .map(|key| {
                crate::document::tree::node_schema_id(
                    &rt.document.as_ref().unwrap().tree.nodes[*key].schema,
                )
                .to_owned()
            })
            .collect()
    };

    // The persisted binding wins on the first layout, before any paint.
    let ids = spatial_ids(&rt);
    assert!(ids.contains("second-action"));
    assert!(!ids.contains("first-action"));
    assert!(!ids.contains("third-action"));
    assert_eq!(focus_ids(&rt), vec!["tabs", "second-action"]);

    let tabs_key = rt.document.as_ref().unwrap().tree.get("tabs").unwrap();
    let tabs_rect = rt.node_scene_rect(tabs_key).expect("tabs laid out");
    let third_cell = point(
        tabs_rect.min_x() + tabs_rect.size.width * 5.0 / 6.0,
        tabs_rect.min_y() + 16.0,
    );
    rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Down, third_cell));
    rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Up, third_cell));

    assert_eq!(
        rt.state
            .app_get("active")
            .and_then(|value| value.as_str().map(str::to_owned))
            .as_deref(),
        Some("third")
    );
    let ids = spatial_ids(&rt);
    assert!(ids.contains("third-action"));
    assert!(!ids.contains("first-action"));
    assert!(!ids.contains("second-action"));
    assert_eq!(focus_ids(&rt), vec!["tabs", "third-action"]);

    // A tap below the intrinsic 32px strip is panel interaction, not a
    // tab change, even when its x coordinate lies in a different cell.
    let panel_point = point(tabs_rect.min_x() + 10.0, tabs_rect.min_y() + 64.0);
    rt.dispatch_pointer(PointerEvent::simple(2, PointerPhase::Down, panel_point));
    rt.dispatch_pointer(PointerEvent::simple(2, PointerPhase::Up, panel_point));
    assert_eq!(
        rt.state
            .app_get("active")
            .and_then(|value| value.as_str().map(str::to_owned))
            .as_deref(),
        Some("third")
    );
}

#[test]
fn tabs_keyboard_switch_rebuilds_spatial_and_focus_in_the_same_turn() {
    use crate::geometry::rect;
    use crate::gesture::pointer::Modifiers;
    use std::collections::HashSet;

    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{
              "version":"1.1","formatVersion":"1.1",
              "children":[
                {"type":"tabs","id":"tabs","width":200,"height":100,"value":"first",
                 "tabs":[
                   {"value":"first","label":"First"},
                   {"value":"second","label":"Second"}
                 ],
                 "children":[
                   {"type":"rectangle","id":"first-action","width":20,"height":20,
                    "gestures":{"focusable":true}},
                   {"type":"rectangle","id":"second-action","width":20,"height":20,
                    "gestures":{"focusable":true}}
                 ]}
              ]
            }"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.focus_next().unwrap();
    assert_eq!(rt.focused_widget_id().as_deref(), Some("tabs"));

    rt.dispatch_keyboard("ArrowRight", Modifiers::empty());

    let indexed: HashSet<String> = rt
        .spatial
        .query_rect(rect(-1000.0, -1000.0, 4000.0, 4000.0))
        .into_iter()
        .map(|key| {
            crate::document::tree::node_schema_id(
                &rt.document.as_ref().unwrap().tree.nodes[key].schema,
            )
            .to_owned()
        })
        .collect();
    assert!(indexed.contains("second-action"));
    assert!(!indexed.contains("first-action"));
    let focus_ids: Vec<&str> = rt
        .focus
        .chain()
        .iter()
        .map(|key| {
            crate::document::tree::node_schema_id(
                &rt.document.as_ref().unwrap().tree.nodes[*key].schema,
            )
        })
        .collect();
    assert_eq!(focus_ids, vec!["tabs", "second-action"]);
}

#[test]
fn empty_tabs_have_no_panel_and_ignore_bar_clicks() {
    use crate::geometry::{point, rect};
    use crate::gesture::pointer::{PointerEvent, PointerPhase};

    let mut rt = Runtime::new_from_document(
        serde_json::from_str::<PenDocument>(
            r#"{
              "version":"1.1","formatVersion":"1.1",
              "children":[
                {"type":"tabs","id":"tabs","width":200,"height":80,
                 "tabs":[],"bindings":{"bind:value":"$state.active"},
                 "children":[
                   {"type":"rectangle","id":"orphan","width":20,"height":20,
                    "gestures":{"focusable":true}}
                 ]}
              ]
            }"#,
        )
        .unwrap(),
    )
    .unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();

    let indexed_ids: Vec<String> = rt
        .spatial
        .query_rect(rect(-1000.0, -1000.0, 4000.0, 4000.0))
        .into_iter()
        .map(|key| {
            crate::document::tree::node_schema_id(
                &rt.document.as_ref().unwrap().tree.nodes[key].schema,
            )
            .to_owned()
        })
        .collect();
    assert!(indexed_ids.contains(&"tabs".to_owned()));
    assert!(!indexed_ids.contains(&"orphan".to_owned()));
    assert_eq!(rt.focus.chain().len(), 1);

    let tabs_key = rt.document.as_ref().unwrap().tree.get("tabs").unwrap();
    let tabs_rect = rt.node_scene_rect(tabs_key).unwrap();
    let bar = point(tabs_rect.min_x() + 20.0, tabs_rect.min_y() + 16.0);
    rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Down, bar));
    rt.dispatch_pointer(PointerEvent::simple(1, PointerPhase::Up, bar));
    assert!(rt.state.app_get("active").is_none());
}
