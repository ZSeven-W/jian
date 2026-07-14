use jian_core::geometry::rect;
use jian_core::render::{
    collect_scene_paint_commands_with_state, DrawOp, RichTextGrowth, ScenePaintCommand,
};

#[test]
fn production_scene_stream_balances_clip_transform_and_effect_layers() {
    let mut runtime = jian_core::Runtime::new();
    runtime
        .load_str(
            r##"{
              "version":"1.2",
              "responsive":true,
              "children":[{
                "type":"frame","id":"clip","width":80,"height":60,"clipContent":true,
                "children":[{
                  "type":"rectangle","id":"styled","x":50,"y":10,"width":60,"height":20,
                  "rotation":90,
                  "fill":[{"type":"solid","color":"#ff0000"}],
                  "effects":[
                    {"type":"blur","radius":4},
                    {"type":"shadow","offsetX":6,"offsetY":3,"blur":5,"spread":2,"color":"#00000080"}
                  ]
                }]
              }]
            }"##,
        )
        .unwrap();
    runtime.build_layout((160.0, 100.0)).unwrap();

    let commands = collect_scene_paint_commands_with_state(
        runtime.document.as_ref().unwrap(),
        &runtime.layout,
        &runtime.state,
    );
    assert!(commands
        .iter()
        .any(|command| matches!(command, ScenePaintCommand::PushClip(_))));
    assert!(commands
        .iter()
        .any(|command| matches!(command, ScenePaintCommand::PushTransform(_))));
    assert!(commands
        .iter()
        .any(|command| matches!(command, ScenePaintCommand::ApplyBlur(4.0))));
    assert!(commands
        .iter()
        .any(|command| matches!(command, ScenePaintCommand::ApplyShadow(_))));

    let layers = commands
        .iter()
        .filter(|command| matches!(command, ScenePaintCommand::PushLayer(_)))
        .count();
    let layer_pops = commands
        .iter()
        .filter(|command| matches!(command, ScenePaintCommand::PopLayer))
        .count();
    let state_pushes = commands
        .iter()
        .filter(|command| {
            matches!(
                command,
                ScenePaintCommand::PushClip(_) | ScenePaintCommand::PushTransform(_)
            )
        })
        .count();
    let state_pops = commands
        .iter()
        .filter(|command| matches!(command, ScenePaintCommand::Pop))
        .count();
    assert_eq!(layers, 2);
    assert_eq!(layer_pops, layers);
    assert_eq!(state_pops, state_pushes);

    let blur = commands
        .iter()
        .position(|command| matches!(command, ScenePaintCommand::ApplyBlur(4.0)))
        .unwrap();
    let shadow = commands
        .iter()
        .position(|command| matches!(command, ScenePaintCommand::ApplyShadow(_)))
        .unwrap();
    assert!(matches!(
        commands.get(blur + 1),
        Some(ScenePaintCommand::PushLayer(_))
    ));
    assert!(matches!(
        commands.get(shadow + 1),
        Some(ScenePaintCommand::PushLayer(_))
    ));
    assert!(blur < shadow);
    let ScenePaintCommand::PushLayer(composed_bounds) = commands[blur + 1] else {
        unreachable!()
    };
    assert!(composed_bounds.min_x() <= 21.0);
    assert!(composed_bounds.max_x() >= 145.0);
}

#[test]
fn composed_effect_layers_expand_from_each_nested_child() {
    let mut runtime = jian_core::Runtime::new();
    runtime
        .load_str(
            r##"{
              "version":"1.2","responsive":true,
              "children":[{
                "type":"rectangle","id":"composed","x":40,"y":30,"width":20,"height":20,
                "fill":[{"type":"solid","color":"#ff0000"}],
                "effects":[
                  {"type":"blur","radius":4},
                  {"type":"shadow","offsetX":12,"offsetY":0,"blur":5,"spread":0,"color":"#000000"}
                ]
              }]
            }"##,
        )
        .unwrap();
    runtime.build_layout((140.0, 90.0)).unwrap();

    let commands = collect_scene_paint_commands_with_state(
        runtime.document.as_ref().unwrap(),
        &runtime.layout,
        &runtime.state,
    );
    let layers = commands
        .iter()
        .filter_map(|command| match command {
            ScenePaintCommand::PushLayer(bounds) => Some(*bounds),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        layers,
        vec![rect(13.0, 3.0, 86.0, 74.0), rect(25.0, 15.0, 62.0, 50.0),],
        "outer layers must add their own outset to the already-expanded nested child"
    );
}

#[test]
fn ancestor_effect_bounds_include_rotated_stroked_descendant_effects() {
    let mut runtime = jian_core::Runtime::new();
    runtime
        .load_str(
            r##"{
              "version":"1.2","responsive":true,
              "children":[{
                "type":"frame","id":"effect-root","width":30,"height":30,
                "effects":[{"type":"blur","radius":1}],
                "children":[{
                  "type":"rectangle","id":"child","x":80,"y":20,"width":40,"height":10,
                  "rotation":45,
                  "stroke":{"thickness":10,"fill":[{"type":"solid","color":"#ff0000"}]},
                  "effects":[{"type":"shadow","offsetX":20,"offsetY":0,"blur":2,"spread":0,"color":"#000000"}]
                }]
              }]
            }"##,
        )
        .unwrap();
    runtime.build_layout((200.0, 100.0)).unwrap();

    let commands = collect_scene_paint_commands_with_state(
        runtime.document.as_ref().unwrap(),
        &runtime.layout,
        &runtime.state,
    );
    let root_blur = commands
        .iter()
        .position(|command| matches!(command, ScenePaintCommand::ApplyBlur(1.0)))
        .unwrap();
    let ScenePaintCommand::PushLayer(bounds) = commands[root_blur + 1] else {
        panic!("root blur must immediately consume a bounded layer");
    };

    assert!(
        bounds.max_x() > 150.0,
        "ancestor layer must contain the rotated descendant shadow tail: {bounds:?}"
    );
    assert!(
        bounds.min_y() < -8.0,
        "ancestor layer must contain rotated stroke/effect overflow: {bounds:?}"
    );
}

#[test]
fn production_pixel_fixture_emits_exact_clip_transform_and_draw_geometry() {
    let mut runtime = jian_core::Runtime::new();
    runtime
        .load_str(
            r##"{
              "version":"1.2","responsive":true,
              "children":[{"type":"frame","id":"viewport","width":"fill_container","height":"fill_container",
                "children":[
                  {"type":"frame","id":"clip","x":5,"y":5,"width":60,"height":45,"clipContent":true,
                   "children":[{"type":"rectangle","id":"wide","x":40,"y":10,"width":50,"height":20,
                     "fill":[{"type":"solid","color":"#ff0000"}]}]},
                  {"type":"rectangle","id":"rotated","x":80,"y":60,"width":40,"height":10,"rotation":90,
                   "fill":[{"type":"solid","color":"#ff0000"}]},
                  {"type":"rectangle","id":"later","x":140,"y":10,"width":20,"height":20,
                   "fill":[{"type":"solid","color":"#00ff00"}]}
                ]}]
            }"##,
        )
        .unwrap();
    runtime.build_layout((180.0, 120.0)).unwrap();
    let document = runtime.document.as_ref().unwrap();

    assert_eq!(
        runtime.layout.node_rect(document.tree.by_id["clip"]),
        Some(rect(5.0, 5.0, 60.0, 45.0))
    );
    assert_eq!(
        runtime.layout.node_rect(document.tree.by_id["wide"]),
        Some(rect(45.0, 15.0, 50.0, 20.0))
    );
    let commands =
        collect_scene_paint_commands_with_state(document, &runtime.layout, &runtime.state);
    assert_eq!(
        commands.len(),
        7,
        "unexpected command stream: {commands:#?}"
    );
    assert!(
        matches!(commands[0], ScenePaintCommand::PushClip(bounds) if bounds == rect(5.0, 5.0, 60.0, 45.0))
    );
    assert!(
        matches!(commands[1], ScenePaintCommand::Draw(DrawOp::Rect { rect: bounds, .. }) if bounds == rect(45.0, 15.0, 50.0, 20.0))
    );
    assert!(matches!(commands[2], ScenePaintCommand::Pop));
    let ScenePaintCommand::PushTransform(transform) = commands[3] else {
        panic!("rotation must precede its draw: {commands:#?}");
    };
    for (actual, expected) in [
        (transform.m11, 0.0),
        (transform.m12, 1.0),
        (transform.m21, -1.0),
        (transform.m22, 0.0),
        (transform.m31, 165.0),
        (transform.m32, -35.0),
    ] {
        assert!((actual - expected).abs() < 0.001);
    }
    assert!(
        matches!(commands[4], ScenePaintCommand::Draw(DrawOp::Rect { rect: bounds, .. }) if bounds == rect(80.0, 60.0, 40.0, 10.0))
    );
    assert!(matches!(commands[5], ScenePaintCommand::Pop));
    assert!(
        matches!(commands[6], ScenePaintCommand::Draw(DrawOp::Rect { rect: bounds, .. }) if bounds == rect(140.0, 10.0, 20.0, 20.0))
    );
}

#[test]
fn production_pixel_fixtures_emit_exact_layers_and_text_growth() {
    let mut runtime = jian_core::Runtime::new();
    runtime
        .load_str(
            r##"{
              "version":"1.2","responsive":true,
              "children":[
                {"type":"rectangle","id":"blurred","x":20,"y":25,"width":25,"height":25,
                 "fill":[{"type":"solid","color":"#ff0000"}],
                 "effects":[{"type":"blur","radius":4}]},
                {"type":"rectangle","id":"shadowed","x":80,"y":25,"width":20,"height":20,
                 "fill":[{"type":"solid","color":"#ff0000"}],
                 "effects":[{"type":"shadow","offsetX":12,"offsetY":0,"blur":0,"spread":4,"color":"#000000"}]}
              ]
            }"##,
        )
        .unwrap();
    runtime.build_layout((160.0, 90.0)).unwrap();
    let document = runtime.document.as_ref().unwrap();
    let commands =
        collect_scene_paint_commands_with_state(document, &runtime.layout, &runtime.state);
    assert!(
        matches!(commands.as_slice(), [
        ScenePaintCommand::ApplyBlur(sigma),
        ScenePaintCommand::PushLayer(blur_bounds),
        ScenePaintCommand::Draw(DrawOp::Rect { rect: blurred, .. }),
        ScenePaintCommand::PopLayer,
        ScenePaintCommand::ApplyShadow(shadow),
        ScenePaintCommand::PushLayer(shadow_bounds),
        ScenePaintCommand::Draw(DrawOp::Rect { rect: shadowed, .. }),
        ScenePaintCommand::PopLayer,
    ] if *sigma == 4.0
        && *blur_bounds == rect(8.0, 13.0, 49.0, 49.0)
        && *blurred == rect(20.0, 25.0, 25.0, 25.0)
        && shadow.dx == 12.0
        && shadow.dy == 0.0
        && shadow.blur == 0.0
        && shadow.spread == 4.0
        && *shadow_bounds == rect(76.0, 21.0, 40.0, 28.0)
        && *shadowed == rect(80.0, 25.0, 20.0, 20.0)),
        "unexpected effect command stream: {commands:#?}"
    );

    runtime
        .load_str_and_relayout(
            r##"{
              "version":"1.2","responsive":true,
              "children":[{"type":"text","id":"copy","x":10,"y":10,"width":60,"height":24,
                "content":"MMMMMMMMMMMM","fontSize":24,"textGrowth":"fixed-width-height",
                "fill":[{"type":"solid","color":"#000000"}]}]
            }"##,
        )
        .unwrap();
    let document = runtime.document.as_ref().unwrap();
    let commands =
        collect_scene_paint_commands_with_state(document, &runtime.layout, &runtime.state);
    assert!(
        matches!(commands.as_slice(), [ScenePaintCommand::RichText { run, plan }]
        if run.origin.x == 10.0
            && run.origin.y == 10.0
            && run.max_width == 0.0
            && plan.bounds == rect(10.0, 10.0, 60.0, 24.0)
            && plan.growth == RichTextGrowth::FixedWidthHeight)
    );
}
