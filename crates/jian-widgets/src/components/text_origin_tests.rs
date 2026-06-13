use crate::components::button::{Button, ButtonVariant};
use crate::components::dialog::Dialog;
use crate::components::menu::{Menu, MenuItem, MenuState};
use crate::components::select::{Select, SelectItem, SelectState};
use crate::components::text_area::TextArea;
use crate::components::text_input::TextInputView;
use crate::test_support::CapturePainter;
use crate::{Point2D, Rect, Tokens};

fn assert_zero_relative_text_runs(p: &CapturePainter) {
    let origins: Vec<_> = p.text_run_origins().collect();
    assert!(
        !origins.is_empty(),
        "widget should paint at least one text run"
    );
    assert!(
        origins
            .iter()
            .all(|origin| *origin == Point2D::new(0.0, 0.0)),
        "text runs must be relative to the draw_text origin: {origins:?}"
    );
}

#[test]
fn components_pass_text_position_as_draw_origin_only() {
    let t = Tokens::dark();
    let mut p = CapturePainter::default();

    Button {
        label: "Save",
        icon_d: None,
        variant: ButtonVariant::Ghost,
        enabled: true,
        hovered: false,
        pressed: false,
        font_size: 12.0,
    }
    .paint(&mut p, Rect::xywh(10.0, 20.0, 80.0, 28.0), &t);

    Dialog {
        title: "Export",
        width: 120.0,
        height: 80.0,
    }
    .paint(&mut p, Rect::xywh(0.0, 0.0, 300.0, 200.0), &t);

    let menu_state = MenuState { hover: Some(0) };
    let menu_items = [MenuItem {
        label: "Delete",
        icon_d: None,
        danger: true,
        disabled: false,
        separator_above: false,
    }];
    Menu {
        state: &menu_state,
        items: &menu_items,
    }
    .paint(
        &mut p,
        Point2D::new(30.0, 40.0),
        Rect::xywh(0.0, 0.0, 240.0, 240.0),
        &t,
    );

    let select_state = SelectState {
        open: true,
        hover: Some(0),
        pressed: None,
        scroll: Default::default(),
    };
    let select_items = [SelectItem {
        label: "Option",
        selected: false,
        disabled: false,
    }];
    Select {
        state: &select_state,
        items: &select_items,
    }
    .paint(
        &mut p,
        Rect::xywh(20.0, 30.0, 100.0, 28.0),
        Rect::xywh(0.0, 0.0, 240.0, 240.0),
        &t,
    );

    let input_state = jian_core::text_input::TextInputState::with_text("abc");
    TextInputView {
        state: &input_state,
        placeholder: "",
        focused: true,
        font_size: 12.0,
        now_ms: 0,
        pad_x: 8.0,
    }
    .paint(&mut p, Rect::xywh(20.0, 80.0, 140.0, 28.0), &t);

    let area_state = jian_core::text_input::TextInputState::with_text("line");
    TextArea {
        state: &area_state,
        placeholder: "",
        focused: true,
        font_size: 12.0,
        now_ms: 0,
        pad_x: 8.0,
        max_visible_lines: 3,
    }
    .paint(&mut p, Rect::xywh(20.0, 120.0, 140.0, 80.0), &t);

    assert_zero_relative_text_runs(&p);
}
