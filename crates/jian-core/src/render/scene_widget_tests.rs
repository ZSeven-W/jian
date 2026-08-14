use super::*;
use crate::Runtime;

fn doc_with(src: &str) -> Runtime {
    let mut rt = Runtime::new();
    rt.load_str(src).unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt
}

fn widget_draws(kind: &str, json: Value, width: f32, height: f32) -> Vec<DrawOp> {
    let bounds = rect(0.0, 0.0, width, height);
    let mut ops = Vec::new();
    emit_widget_visual(kind, bounds, bounds, &json, &mut ops);
    ops
}

fn rounded_paint(op: &DrawOp) -> &Paint {
    match op {
        DrawOp::RoundedRect { paint, .. } => paint,
        other => panic!("expected RoundedRect, got {other:?}"),
    }
}

fn rounded_radii(op: &DrawOp) -> BorderRadii {
    match op {
        DrawOp::RoundedRect { radii, .. } => *radii,
        other => panic!("expected RoundedRect, got {other:?}"),
    }
}

fn rounded_rect(op: &DrawOp) -> crate::geometry::Rect {
    match op {
        DrawOp::RoundedRect { rect, .. } => *rect,
        other => panic!("expected RoundedRect, got {other:?}"),
    }
}

fn rect_paint(op: &DrawOp) -> &Paint {
    match op {
        DrawOp::Rect { paint, .. } => paint,
        other => panic!("expected Rect, got {other:?}"),
    }
}

fn path_stroke(op: &DrawOp) -> &StrokeOp {
    match op {
        DrawOp::Path { paint, .. } => paint.stroke.as_ref().expect("path stroke"),
        other => panic!("expected Path, got {other:?}"),
    }
}

fn geometry_paint(op: &DrawOp) -> Option<&Paint> {
    match op {
        DrawOp::Rect { paint, .. }
        | DrawOp::RoundedRect { paint, .. }
        | DrawOp::Path { paint, .. } => Some(paint),
        _ => None,
    }
}

#[test]
fn checkbox_label_uses_full_control_layout_and_node_opacity() {
    let ops = widget_draws(
        "checkbox",
        serde_json::json!({
            "checked": true,
            "label": "Accept",
            "opacity": 0.5
        }),
        180.0,
        24.0,
    );

    assert_eq!(rounded_rect(&ops[0]), rect(0.0, 0.0, 24.0, 24.0));
    assert_eq!(rounded_paint(&ops[0]).opacity, 0.5);
    match &ops[1] {
        DrawOp::Path { commands, paint } => {
            let expected = [(5.76, 12.48), (10.08, 16.8), (18.24, 7.2)];
            assert_eq!(commands.len(), expected.len());
            for (command, (expected_x, expected_y)) in commands.iter().zip(expected) {
                let actual = match command {
                    PathCommand::MoveTo(point) | PathCommand::LineTo(point) => point,
                    other => panic!("unexpected checkbox path command {other:?}"),
                };
                assert!((actual.x - expected_x).abs() < 0.0001);
                assert!((actual.y - expected_y).abs() < 0.0001);
            }
            assert_eq!(paint.opacity, 0.5);
        }
        other => panic!("expected checkbox check path, got {other:?}"),
    }
    match &ops[2] {
        DrawOp::Text(run) => {
            assert_eq!(run.content, "Accept");
            assert_eq!(run.origin, point(32.0, 5.0));
            assert_eq!(run.font_size, 14.0);
            assert_eq!(run.max_width, 148.0);
            assert_eq!(run.color, Color::rgba(0x9c, 0xa3, 0xaf, 0x80));
        }
        other => panic!("expected adjacent checkbox label, got {other:?}"),
    }
}

#[test]
fn progress_indeterminate_is_deterministic_and_ignores_value() {
    let at_zero = widget_draws(
        "progress",
        serde_json::json!({ "max": 100, "value": 0, "indeterminate": true }),
        200.0,
        8.0,
    );
    let at_max = widget_draws(
        "progress",
        serde_json::json!({ "max": 100, "value": 100, "indeterminate": true }),
        200.0,
        8.0,
    );

    assert_eq!(at_zero.len(), 2);
    assert_eq!(at_max.len(), 2);
    assert_eq!(rounded_rect(&at_zero[0]), rect(0.0, 0.0, 200.0, 8.0));
    assert_eq!(rounded_rect(&at_zero[1]), rect(65.0, 0.0, 70.0, 8.0));
    assert_eq!(rounded_rect(&at_max[1]), rounded_rect(&at_zero[1]));
    assert_eq!(
        rounded_paint(&at_max[1]).fill,
        rounded_paint(&at_zero[1]).fill
    );
}

#[test]
fn progress_value_binding_preserves_number_and_controls_segment_width() {
    let rt = doc_with(
        r##"{ "version":"1.1", "formatVersion":"1.1", "id":"x",
             "app": { "name":"x", "version":"1", "id":"x" },
             "children": [
               { "type":"progress", "id":"pg", "width":200, "height":8,
                 "max":100, "value":10,
                 "bindings":{ "value":"$state.p" } }
             ]}"##,
    );
    rt.state.app_set("p", serde_json::json!(75));

    let ops = collect_draws_with_state(rt.document.as_ref().unwrap(), &rt.layout, &rt.state);
    assert_eq!(ops.len(), 2);
    assert_eq!(rounded_rect(&ops[1]), rect(0.0, 0.0, 150.0, 8.0));
}

#[test]
fn live_widget_state_is_authoritative_for_every_non_text_family() {
    use crate::widget_state::WidgetState;

    let runtime = Runtime::new();
    let mut states = crate::widget_state::WidgetStateStore::default();
    let nodes: Vec<PenNode> = vec![
        serde_json::from_value(serde_json::json!({
            "type":"switch", "id":"toggle", "checked":false
        }))
        .unwrap(),
        serde_json::from_value(serde_json::json!({
            "type":"slider", "id":"slider", "min":0, "max":100, "value":5
        }))
        .unwrap(),
        serde_json::from_value(serde_json::json!({
            "type":"select", "id":"select", "value":"a",
            "options":[{"value":"a","label":"A"},{"value":"b","label":"B"}]
        }))
        .unwrap(),
        serde_json::from_value(serde_json::json!({
            "type":"radio_group", "id":"radio", "value":"a",
            "options":[{"value":"a","label":"A"},{"value":"b","label":"B"}]
        }))
        .unwrap(),
        serde_json::from_value(serde_json::json!({
            "type":"tabs", "id":"tabs", "value":"a",
            "tabs":[{"value":"a","label":"A"},{"value":"b","label":"B"}],
            "children":[]
        }))
        .unwrap(),
    ];
    for node in &nodes {
        states.get_or_init(node, &runtime.state).unwrap();
    }
    match states.get_mut("toggle").unwrap() {
        WidgetState::Toggle { on } => *on = true,
        other => panic!("unexpected toggle state {other:?}"),
    }
    match states.get_mut("slider").unwrap() {
        WidgetState::Slider { value, .. } => *value = 75.0,
        other => panic!("unexpected slider state {other:?}"),
    }
    match states.get_mut("select").unwrap() {
        WidgetState::Select { value, .. } => *value = Some("b".to_owned()),
        other => panic!("unexpected select state {other:?}"),
    }
    match states.get_mut("radio").unwrap() {
        WidgetState::Radio { value, .. } => *value = Some("b".to_owned()),
        other => panic!("unexpected radio state {other:?}"),
    }
    match states.get_mut("tabs").unwrap() {
        WidgetState::Tabs { active, .. } => *active = Some("b".to_owned()),
        other => panic!("unexpected tabs state {other:?}"),
    }

    let theme = crate::render::widget_style::WidgetTheme::default();
    let ctx = WidgetRenderCtx {
        states: &states,
        theme: &theme,
        focused_id: None,
        now_ms: 0,
        caret_x: None,
    };
    let mut effective: Vec<Value> = nodes
        .iter()
        .map(|node| serde_json::to_value(node).unwrap())
        .collect();
    for json in &mut effective {
        apply_live_widget_state(json, &ctx);
    }

    assert_eq!(effective[0]["checked"], serde_json::json!(true));
    assert_eq!(effective[1]["value"], serde_json::json!(75.0));
    assert_eq!(effective[2]["value"], serde_json::json!("b"));
    assert_eq!(effective[3]["value"], serde_json::json!("b"));
    assert_eq!(effective[4]["value"], serde_json::json!("b"));
}

#[test]
fn tabs_layout_and_both_collectors_use_only_the_active_panel() {
    let rt = doc_with(
        r##"{ "version":"1.1", "formatVersion":"1.1", "id":"x",
             "app": { "name":"x", "version":"1", "id":"x" },
             "children": [
               { "type":"tabs", "id":"tabs", "width":200, "height":120,
                 "value":"b",
                 "fill":[{"type":"solid","color":"#120826"}],
                 "stroke":{"thickness":1,"fill":[{"type":"solid","color":"#f4f4f5"}]},
                 "tabs":[
                   {"value":"a","label":"Alpha"},
                   {"value":"b","label":"Beta"}
                 ],
                 "children":[
                   {"type":"rectangle","id":"panel-a","width":"fill_container",
                    "height":"fill_container","fill":[{"type":"solid","color":"#ff0000"}]},
                   {"type":"rectangle","id":"panel-b","width":"fill_container",
                    "height":"fill_container","fill":[{"type":"solid","color":"#00ff00"}]}
                 ] }
             ]}"##,
    );
    let doc = rt.document.as_ref().unwrap();
    for panel in ["panel-a", "panel-b"] {
        let key = doc.tree.get(panel).unwrap();
        assert_eq!(
            rt.layout.node_rect(key),
            Some(rect(0.0, 32.0, 200.0, 88.0)),
            "all tab panels share the same content cell"
        );
    }

    let flat = collect_draws(doc, &rt.layout);
    assert!(flat.iter().any(|op| matches!(
        op,
        DrawOp::RoundedRect { rect: pill, paint, .. }
            if *pill == rect(102.0, 2.0, 96.0, 28.0)
                && paint.fill == Some(Color::rgb(0x12, 0x08, 0x26))
    )));
    assert!(flat
        .iter()
        .any(|op| matches!(op, DrawOp::Text(run) if run.content == "Alpha")));
    assert!(flat
        .iter()
        .any(|op| matches!(op, DrawOp::Text(run) if run.content == "Beta")));
    assert!(flat.iter().any(|op| geometry_paint(op)
        .is_some_and(|paint| paint.fill == Some(Color::rgb(0x00, 0xff, 0x00)))));
    assert!(!flat.iter().any(|op| geometry_paint(op)
        .is_some_and(|paint| paint.fill == Some(Color::rgb(0xff, 0x00, 0x00)))));

    let structured =
        crate::render::collect_scene_paint_commands_with_state(doc, &rt.layout, &rt.state);
    let structured_draws: Vec<&DrawOp> = structured
        .iter()
        .filter_map(|command| match command {
            crate::render::ScenePaintCommand::Draw(op) => Some(op),
            _ => None,
        })
        .collect();
    assert!(structured_draws.iter().any(|op| geometry_paint(op)
        .is_some_and(|paint| paint.fill == Some(Color::rgb(0x00, 0xff, 0x00)))));
    assert!(!structured_draws.iter().any(|op| geometry_paint(op)
        .is_some_and(|paint| paint.fill == Some(Color::rgb(0xff, 0x00, 0x00)))));
}

#[test]
fn tabs_active_index_has_stable_fallbacks() {
    assert_eq!(
        active_tab_index(&serde_json::json!({
            "type":"tabs", "value":"missing",
            "tabs":[{"value":"a"},{"value":"b"}]
        })),
        Some(0)
    );
    assert_eq!(
        active_tab_index(&serde_json::json!({
            "type":"tabs", "tabs":[{"value":"a"},{"value":"b"}]
        })),
        Some(0)
    );
    assert_eq!(
        active_tab_index(&serde_json::json!({"type":"tabs", "tabs":[]})),
        None
    );
}

#[test]
fn node_opacity_covers_every_composite_part_and_text_only_once() {
    let cases = [
        (
            "switch",
            serde_json::json!({"checked":true,"opacity":0.25}),
            44.0,
            24.0,
        ),
        (
            "checkbox",
            serde_json::json!({"checked":true,"label":"Accept","opacity":0.25}),
            120.0,
            24.0,
        ),
        (
            "slider",
            serde_json::json!({"min":0,"max":100,"value":50,"opacity":0.25}),
            200.0,
            20.0,
        ),
        (
            "progress",
            serde_json::json!({"max":100,"value":40,"opacity":0.25}),
            200.0,
            8.0,
        ),
        (
            "select",
            serde_json::json!({
                "value":"a","options":[{"value":"a","label":"Alpha"}],
                "fill":[{"type":"solid","color":"#120826"}],"opacity":0.25
            }),
            180.0,
            40.0,
        ),
        (
            "radio_group",
            serde_json::json!({
                "value":"a","options":[{"value":"a","label":"Alpha"}],
                "opacity":0.25
            }),
            180.0,
            28.0,
        ),
        (
            "tabs",
            serde_json::json!({
                "value":"a","tabs":[{"value":"a","label":"Alpha"},{"value":"b","label":"Beta"}],
                "opacity":0.25
            }),
            200.0,
            120.0,
        ),
    ];
    for (kind, json, width, height) in cases {
        let ops = widget_draws(kind, json, width, height);
        assert!(!ops.is_empty(), "{kind} should paint at least one part");
        for op in &ops {
            if let Some(paint) = geometry_paint(op) {
                assert_eq!(paint.opacity, 0.25, "{kind} leaked opacity in {op:?}");
            }
            if let DrawOp::Text(run) = op {
                assert!(
                    run.color.a() <= 0x40,
                    "{kind} text alpha must receive node opacity once: {run:?}"
                );
            }
        }
    }

    let bounds = rect(0.0, 0.0, 180.0, 40.0);
    let input = serde_json::json!({
        "type":"text_input", "id":"field", "value":"Orion", "opacity":0.25,
        "fill":[{"type":"solid","color":"#120826"}]
    });
    let mut static_ops = Vec::new();
    emit_text_input(bounds, bounds, &input, &mut static_ops);
    for op in &static_ops {
        if let Some(paint) = geometry_paint(op) {
            assert_eq!(paint.opacity, 0.25);
        }
        if let DrawOp::Text(run) = op {
            assert_eq!(run.color.a(), 0x40);
        }
    }

    let states = crate::widget_state::WidgetStateStore::default();
    let theme = crate::render::widget_style::WidgetTheme::default();
    let ctx = WidgetRenderCtx {
        states: &states,
        theme: &theme,
        focused_id: Some("field"),
        now_ms: 0,
        caret_x: None,
    };
    let live = crate::text_input::TextInputState::with_text("Orion".to_owned());
    let mut live_ops = Vec::new();
    emit_live_text_input(bounds, &input, &live, &ctx, "field", &mut live_ops);
    for op in &live_ops {
        if let Some(paint) = geometry_paint(op) {
            assert_eq!(paint.opacity, 0.25);
        }
        if let DrawOp::Text(run) = op {
            assert_eq!(run.color.a(), 0x40);
        }
    }
}

#[test]
fn authored_widget_visuals_map_exact_colors_to_every_composite_family() {
    let dark = Color::rgb(0x12, 0x08, 0x26);
    let light = Color::rgb(0xf4, 0xf4, 0xf5);
    let authored = |extra: Value| {
        let mut json = serde_json::json!({
            "fill": [{ "type": "solid", "color": "#120826" }],
            "stroke": {
                "thickness": 2,
                "fill": [{ "type": "solid", "color": "#f4f4f5" }]
            }
        });
        json.as_object_mut()
            .expect("widget object")
            .extend(extra.as_object().expect("extra object").clone());
        json
    };

    let checkbox = widget_draws(
        "checkbox",
        authored(serde_json::json!({ "checked": true })),
        18.0,
        18.0,
    );
    assert_eq!(rounded_paint(&checkbox[0]).fill, Some(dark));
    assert_eq!(
        rounded_paint(&checkbox[0])
            .stroke
            .as_ref()
            .expect("checkbox border")
            .color,
        light
    );
    assert_eq!(
        path_stroke(&checkbox[1]).color,
        Color::rgb(0xff, 0xff, 0xff)
    );

    let slider = widget_draws(
        "slider",
        authored(serde_json::json!({ "min": 0, "max": 100, "value": 50 })),
        200.0,
        20.0,
    );
    assert_eq!(rounded_paint(&slider[0]).fill, Some(light));
    assert_eq!(rounded_paint(&slider[1]).fill, Some(dark));
    assert_eq!(
        rounded_paint(&slider[2]).fill,
        Some(Color::rgb(0xff, 0xff, 0xff))
    );
    assert_eq!(
        rounded_paint(&slider[2])
            .stroke
            .as_ref()
            .expect("slider thumb border")
            .color,
        light
    );

    let progress = widget_draws(
        "progress",
        authored(serde_json::json!({ "max": 100, "value": 40 })),
        200.0,
        8.0,
    );
    assert_eq!(rounded_paint(&progress[0]).fill, Some(light));
    assert_eq!(rounded_paint(&progress[1]).fill, Some(dark));

    let select = widget_draws(
        "select",
        authored(serde_json::json!({
            "value": "a",
            "options": [{ "value": "a", "label": "Alpha" }]
        })),
        180.0,
        40.0,
    );
    assert_eq!(rounded_paint(&select[0]).fill, Some(dark));
    assert_eq!(
        rounded_paint(&select[0])
            .stroke
            .as_ref()
            .expect("select border")
            .color,
        light
    );
    assert!(matches!(
        &select[1],
        DrawOp::Text(run) if run.color == Color::rgb(0xff, 0xff, 0xff)
    ));
    assert_eq!(
        path_stroke(&select[2]).color,
        Color::rgba(0xff, 0xff, 0xff, 0xa6)
    );

    let radio = widget_draws(
        "radio_group",
        authored(serde_json::json!({
            "value": "a",
            "options": [{ "value": "a", "label": "Alpha" }]
        })),
        180.0,
        28.0,
    );
    assert_eq!(rounded_paint(&radio[0]).fill, Some(dark));
    assert_eq!(
        rounded_paint(&radio[0])
            .stroke
            .as_ref()
            .expect("radio border")
            .color,
        light
    );
    assert_eq!(
        rounded_paint(&radio[1]).fill,
        Some(Color::rgb(0xff, 0xff, 0xff))
    );
    assert!(matches!(
        &radio[2],
        DrawOp::Text(run) if run.color == Color::rgb(0x00, 0x00, 0x00)
    ));

    let tabs = widget_draws(
        "tabs",
        authored(serde_json::json!({
            "value": "b",
            "tabs": [
                { "value": "a", "label": "Alpha" },
                { "value": "b", "label": "Beta" }
            ]
        })),
        200.0,
        120.0,
    );
    assert_eq!(rounded_paint(&tabs[0]).fill, Some(light));
    assert_eq!(rounded_paint(&tabs[1]).fill, Some(dark));
    assert!(matches!(
        &tabs[2],
        DrawOp::Text(run) if run.color == Color::rgba(0x00, 0x00, 0x00, 0xa6)
    ));
    assert!(matches!(
        &tabs[3],
        DrawOp::Text(run) if run.color == Color::rgb(0xff, 0xff, 0xff)
    ));
}

#[test]
fn fill_only_switch_keeps_authored_hue_when_inactive() {
    let switch = widget_draws(
        "switch",
        serde_json::json!({
            "checked": false,
            "fill": [{ "type": "solid", "color": "#7c3aed" }]
        }),
        44.0,
        24.0,
    );

    assert_eq!(
        rounded_paint(&switch[0]).fill,
        Some(Color::rgba(0x7c, 0x3a, 0xed, 0x59))
    );
    assert_eq!(
        rounded_paint(&switch[1]).fill,
        Some(Color::rgb(0xff, 0xff, 0xff))
    );
}

#[test]
fn unstyled_select_is_transparent_with_neutral_readable_foreground() {
    let select = widget_draws(
        "select",
        serde_json::json!({ "placeholder": "Choose" }),
        180.0,
        40.0,
    );

    let surface = rounded_paint(&select[0]);
    assert_eq!(surface.fill, None);
    assert!(surface.stroke.is_none());
    assert!(matches!(
        &select[1],
        DrawOp::Text(run) if run.color == Color::rgba(0x9c, 0xa3, 0xaf, 0xa6)
    ));
    assert_eq!(
        path_stroke(&select[2]).color,
        Color::rgba(0x9c, 0xa3, 0xaf, 0xa6)
    );
}

#[test]
fn static_text_inputs_share_authored_and_transparent_widget_colors() {
    let bounds = rect(0.0, 0.0, 180.0, 40.0);
    let mut dark = Vec::new();
    emit_text_input(
        bounds,
        bounds,
        &serde_json::json!({
            "type": "text_input",
            "value": "Orion",
            "fill": [{ "type": "solid", "color": "#120826" }]
        }),
        &mut dark,
    );
    assert_eq!(
        geometry_paint(&dark[0]).expect("input surface").fill,
        Some(Color::rgb(0x12, 0x08, 0x26))
    );
    assert!(matches!(
        &dark[1],
        DrawOp::Text(run) if run.color == Color::rgb(0xff, 0xff, 0xff)
    ));
    assert_eq!(
        rect_paint(&dark[2]).fill,
        Some(Color::rgb(0xff, 0xff, 0xff))
    );

    let mut unstyled = Vec::new();
    emit_text_input(
        bounds,
        bounds,
        &serde_json::json!({
            "type": "text_input",
            "placeholder": "Search"
        }),
        &mut unstyled,
    );
    assert_eq!(unstyled.len(), 2, "transparent input emits no surface op");
    assert!(matches!(
        &unstyled[0],
        DrawOp::Text(run) if run.color == Color::rgba(0x66, 0x66, 0x66, 0xff)
    ));
    assert_eq!(
        rect_paint(&unstyled[1]).fill,
        Some(Color::rgb(0x33, 0x33, 0x33))
    );
}

#[test]
fn static_and_live_text_inputs_share_intrinsic_and_authored_zero_radius() {
    let bounds = rect(0.0, 0.0, 180.0, 40.0);
    let authored = serde_json::json!({
        "type": "text_input",
        "id": "field",
        "value": "Orion",
        "fill": [{ "type": "solid", "color": "#120826" }]
    });
    let square = serde_json::json!({
        "type": "text_input",
        "id": "field",
        "value": "Orion",
        "cornerRadius": 0,
        "fill": [{ "type": "solid", "color": "#120826" }]
    });

    let mut static_intrinsic = Vec::new();
    emit_text_input(bounds, bounds, &authored, &mut static_intrinsic);
    assert_eq!(
        rounded_radii(&static_intrinsic[0]),
        BorderRadii::uniform(6.0)
    );
    let mut static_square = Vec::new();
    emit_text_input(bounds, bounds, &square, &mut static_square);
    assert!(matches!(static_square[0], DrawOp::Rect { .. }));

    let states = crate::widget_state::WidgetStateStore::default();
    let theme = crate::render::widget_style::WidgetTheme::default();
    let ctx = WidgetRenderCtx {
        states: &states,
        theme: &theme,
        focused_id: None,
        now_ms: 0,
        caret_x: None,
    };
    let live = crate::text_input::TextInputState::with_text("Orion".to_owned());
    let mut live_intrinsic = Vec::new();
    emit_live_text_input(bounds, &authored, &live, &ctx, "field", &mut live_intrinsic);
    assert_eq!(rounded_radii(&live_intrinsic[0]), BorderRadii::uniform(6.0));
    let mut live_square = Vec::new();
    emit_live_text_input(bounds, &square, &live, &ctx, "field", &mut live_square);
    assert!(matches!(live_square[0], DrawOp::Rect { .. }));
}

#[test]
fn widget_tracks_distinguish_absent_radius_from_authored_zero() {
    let authored_switch = widget_draws(
        "switch",
        serde_json::json!({ "checked": true, "cornerRadius": 3 }),
        44.0,
        24.0,
    );
    assert_eq!(
        rounded_radii(&authored_switch[0]),
        BorderRadii::uniform(3.0)
    );

    let square_switch = widget_draws(
        "switch",
        serde_json::json!({ "checked": true, "cornerRadius": 0 }),
        44.0,
        24.0,
    );
    assert_eq!(rounded_radii(&square_switch[0]), BorderRadii::zero());

    let intrinsic_switch =
        widget_draws("switch", serde_json::json!({ "checked": true }), 44.0, 24.0);
    assert_eq!(
        rounded_radii(&intrinsic_switch[0]),
        BorderRadii::uniform(12.0)
    );

    let slider = widget_draws(
        "slider",
        serde_json::json!({
            "min": 0, "max": 100, "value": 50, "cornerRadius": 1
        }),
        200.0,
        20.0,
    );
    assert_eq!(rounded_radii(&slider[0]), BorderRadii::uniform(1.0));
    assert_eq!(rounded_radii(&slider[1]), BorderRadii::uniform(1.0));

    let progress = widget_draws(
        "progress",
        serde_json::json!({ "max": 100, "value": 40, "cornerRadius": 1 }),
        200.0,
        8.0,
    );
    assert_eq!(rounded_radii(&progress[0]), BorderRadii::uniform(1.0));
    assert_eq!(rounded_radii(&progress[1]), BorderRadii::uniform(1.0));

    let checkbox = widget_draws(
        "checkbox",
        serde_json::json!({ "checked": false, "cornerRadius": 0 }),
        18.0,
        18.0,
    );
    assert_eq!(rounded_radii(&checkbox[0]), BorderRadii::zero());

    let intrinsic_checkbox = widget_draws(
        "checkbox",
        serde_json::json!({ "checked": false }),
        18.0,
        18.0,
    );
    assert_eq!(
        rounded_radii(&intrinsic_checkbox[0]),
        BorderRadii::uniform(4.0)
    );
}
