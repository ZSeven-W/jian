use jian_core::render::{collect_draws_with_state, DrawOp};

#[test]
fn native_facing_nonresponsive_text_keeps_authored_wrap_width() {
    for text_growth in [None, Some("auto"), Some("fixed-width-height")] {
        let growth_property = text_growth
            .map(|growth| format!(r#", "textGrowth":"{growth}""#))
            .unwrap_or_default();
        let source = format!(
            r##"{{
              "version":"1.2",
              "children":[{{
                "type":"text",
                "id":"copy",
                "x":7,
                "y":11,
                "width":80,
                "height":24,
                "content":"legacy native wrapping remains stable",
                "fontSize":16
                {growth_property}
              }}]
            }}"##
        );

        let mut runtime = jian_core::Runtime::new();
        runtime.load_str(&source).unwrap();
        runtime.build_layout((240.0, 120.0)).unwrap();

        let text = collect_draws_with_state(
            runtime.document.as_ref().unwrap(),
            &runtime.layout,
            &runtime.state,
        )
        .into_iter()
        .find_map(|draw| match draw {
            DrawOp::Text(run) => Some(run),
            _ => None,
        })
        .expect("the text node should emit a native-facing DrawOp");

        assert_eq!(text.origin.x, 7.0, "growth={text_growth:?}");
        assert_eq!(text.origin.y, 11.0, "growth={text_growth:?}");
        assert_eq!(text.max_width, 80.0, "growth={text_growth:?}");
    }
}
