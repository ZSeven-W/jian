use crate::components::badge::{Badge, BadgeVariant};
use crate::components::button::{Button, ButtonVariant};
use crate::components::dialog::Dialog;
use crate::components::menu::{Menu, MenuItem, MenuState};
use crate::components::select::{Select, SelectItem, SelectState};
use crate::components::select_trigger::SelectTrigger;
use crate::components::tabs::Tabs;
use crate::components::text_area::TextArea;
use crate::components::text_input::TextInputView;
use crate::components::toggle_group::ToggleGroup;
use crate::components::tooltip::Tooltip;
use crate::test_support::{CapturePainter, PaintOp};
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
        icon_paths: None,
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
        baseline_delta_y: 0.0,
        mask: None,
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

fn assert_label_origin(p: &CapturePainter, label: &str, expected: Point2D) {
    let (_, origin, _) = p
        .texts()
        .find(|(content, _, _)| *content == label)
        .unwrap_or_else(|| panic!("missing label {label:?}"));
    assert!(
        (origin.x - expected.x).abs() <= 0.01 && (origin.y - expected.y).abs() <= 0.01,
        "{label:?} origin {origin:?} should be top-left aligned at {expected:?}"
    );
}

fn has_clip(p: &CapturePainter, expected: Rect) -> bool {
    p.ops
        .iter()
        .any(|op| matches!(op, PaintOp::ClipRect(rect) if *rect == expected))
}

fn assert_clips_stay_inside(p: &CapturePainter, outer: Rect) {
    for clip in p.ops.iter().filter_map(|op| match op {
        PaintOp::ClipRect(rect) => Some(*rect),
        _ => None,
    }) {
        assert!(
            clip.origin.x >= outer.origin.x
                && clip.origin.y >= outer.origin.y
                && clip.origin.x + clip.size.x <= outer.origin.x + outer.size.x
                && clip.origin.y + clip.size.y <= outer.origin.y + outer.size.y,
            "clip {clip:?} escapes {outer:?}"
        );
    }
}

fn has_roomier_inner_clip(p: &CapturePainter, outer: Rect, text_advance: f32) -> bool {
    p.ops.iter().any(
        |op| matches!(op, PaintOp::ClipRect(rect) if *rect != outer && rect.size.x > text_advance),
    )
}

#[test]
fn single_line_controls_use_top_left_centered_text_origins() {
    let t = Tokens::dark();

    let mut button = CapturePainter::default();
    Button {
        label: "Run",
        icon_paths: None,
        variant: ButtonVariant::Primary,
        enabled: true,
        hovered: false,
        pressed: false,
        font_size: 13.0,
    }
    .paint(&mut button, Rect::xywh(10.0, 20.0, 80.0, 30.0), &t);
    assert_label_origin(&button, "Run", Point2D::new(39.275, 28.5));

    let menu_state = MenuState::default();
    let menu_items = [MenuItem {
        label: "Open",
        icon_d: None,
        danger: false,
        disabled: false,
        separator_above: false,
    }];
    let mut menu = CapturePainter::default();
    Menu {
        state: &menu_state,
        items: &menu_items,
    }
    .paint(
        &mut menu,
        Point2D::new(10.0, 20.0),
        Rect::xywh(0.0, 0.0, 240.0, 160.0),
        &t,
    );
    assert_label_origin(&menu, "Open", Point2D::new(20.0, 28.5));

    let mut select = CapturePainter::default();
    SelectTrigger {
        icon_paths: None,
        label: "Kit",
        placeholder: "Select…",
        hovered: false,
        pressed: false,
        enabled: true,
        font_size: 12.0,
        bordered: true,
    }
    .paint(&mut select, Rect::xywh(10.0, 20.0, 140.0, 28.0), &t);
    assert_label_origin(&select, "Kit", Point2D::new(18.0, 28.0));

    let mut tabs = CapturePainter::default();
    Tabs {
        labels: &["One", "Two"],
        active: 0,
        hover: None,
    }
    .paint(&mut tabs, Rect::xywh(0.0, 20.0, 160.0, 32.0), &t);
    assert_label_origin(&tabs, "One", Point2D::new(30.1, 30.0));

    let mut toggle = CapturePainter::default();
    ToggleGroup {
        options: &["On", "Off"],
        icons: None,
        active: 0,
        hover: None,
        font_size: 13.0,
    }
    .paint(&mut toggle, Rect::xywh(0.0, 20.0, 120.0, 24.0), &t);
    assert_label_origin(&toggle, "On", Point2D::new(22.85, 25.5));

    let mut badge = CapturePainter::default();
    Badge {
        label: "New",
        icon_d: None,
        variant: BadgeVariant::Default,
        radius: 0.0,
        font_size: 11.0,
    }
    .paint(&mut badge, Rect::xywh(0.0, 20.0, 60.0, 20.0), &t);
    assert_label_origin(&badge, "New", Point2D::new(20.925, 24.5));

    let mut tooltip = CapturePainter::default();
    Tooltip { label: "Copy" }.paint(&mut tooltip, Rect::xywh(0.0, 20.0, 60.0, 24.0), &t);
    assert_label_origin(&tooltip, "Copy", Point2D::new(16.8, 26.0));
}

#[test]
fn tiny_button_clips_an_overlong_icon_label_to_its_control() {
    let t = Tokens::dark();
    let rect = Rect::xywh(10.0, 20.0, 30.0, 18.0);
    let icon: &[&str] = &["M4 12h16"];
    let mut p = CapturePainter::default();

    Button {
        label: "A button label that cannot fit",
        icon_paths: Some(icon),
        variant: ButtonVariant::Outline,
        enabled: true,
        hovered: false,
        pressed: false,
        font_size: 13.0,
    }
    .paint(&mut p, rect, &t);

    assert!(has_clip(&p, rect), "icon and label need a shared clip");
    assert_clips_stay_inside(&p, rect);
}

#[test]
fn tiny_badge_clips_an_overlong_icon_label_to_its_control() {
    let t = Tokens::dark();
    let rect = Rect::xywh(10.0, 20.0, 40.0, 16.0);
    let mut p = CapturePainter::default();

    Badge {
        label: "A badge label that cannot fit",
        icon_d: Some("M4 12h16"),
        variant: BadgeVariant::Outline,
        radius: 0.0,
        font_size: 11.0,
    }
    .paint(&mut p, rect, &t);

    assert!(has_clip(&p, rect), "icon and label need a shared clip");
    assert_clips_stay_inside(&p, rect);
}

#[test]
fn tiny_toggle_cells_clip_overlong_icon_labels_per_segment() {
    let t = Tokens::dark();
    let rect = Rect::xywh(10.0, 20.0, 60.0, 18.0);
    let first = Rect::xywh(10.0, 20.0, 30.0, 18.0);
    let second = Rect::xywh(40.0, 20.0, 30.0, 18.0);
    let icon_a: &[&str] = &["M4 12h16"];
    let icon_b: &[&str] = &["M12 4v16"];
    let icons: &[&[&str]] = &[icon_a, icon_b];
    let mut p = CapturePainter::default();

    ToggleGroup {
        options: &["First option cannot fit", "Second option cannot fit"],
        icons: Some(icons),
        active: 0,
        hover: None,
        font_size: 13.0,
    }
    .paint(&mut p, rect, &t);

    assert!(has_clip(&p, first), "first segment needs a content clip");
    assert!(has_clip(&p, second), "second segment needs a content clip");
    assert_clips_stay_inside(&p, rect);
}

#[test]
fn icon_labels_reserve_clip_room_for_normal_glyph_overhang() {
    let t = Tokens::dark();
    let icon: &[&str] = &["M4 12h16"];

    let button_rect = Rect::xywh(0.0, 0.0, 120.0, 30.0);
    let mut button = CapturePainter::default();
    Button {
        label: "Go",
        icon_paths: Some(icon),
        variant: ButtonVariant::Outline,
        enabled: true,
        hovered: false,
        pressed: false,
        font_size: 13.0,
    }
    .paint(&mut button, button_rect, &t);
    assert!(has_roomier_inner_clip(&button, button_rect, 14.3));

    let badge_rect = Rect::xywh(0.0, 0.0, 80.0, 20.0);
    let mut badge = CapturePainter::default();
    Badge {
        label: "Go",
        icon_d: Some("M4 12h16"),
        variant: BadgeVariant::Outline,
        radius: 0.0,
        font_size: 11.0,
    }
    .paint(&mut badge, badge_rect, &t);
    assert!(has_roomier_inner_clip(&badge, badge_rect, 12.1));

    let toggle_rect = Rect::xywh(0.0, 0.0, 100.0, 24.0);
    let icons: &[&[&str]] = &[icon];
    let mut toggle = CapturePainter::default();
    ToggleGroup {
        options: &["Go"],
        icons: Some(icons),
        active: 0,
        hover: None,
        font_size: 13.0,
    }
    .paint(&mut toggle, toggle_rect, &t);
    assert!(has_roomier_inner_clip(&toggle, toggle_rect, 14.3));
}
