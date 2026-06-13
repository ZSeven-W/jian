use super::*;
use crate::test_support::{CapturePainter, PaintOp};

#[test]
fn layout_preserves_blank_lines_and_byte_offsets() {
    let mut p = CapturePainter::default();
    let lines = TextArea::layout_lines(&mut p, "para1\n\npara2", 10.0, 200.0);

    assert_eq!(
        lines,
        vec![
            TextLine {
                text: "para1".to_owned(),
                start: 0,
                end: 5,
            },
            TextLine {
                text: String::new(),
                start: 6,
                end: 6,
            },
            TextLine {
                text: "para2".to_owned(),
                start: 7,
                end: 12,
            },
        ]
    );
}

#[test]
fn cjk_wraps_per_character_with_measured_width() {
    let mut p = CapturePainter::default();
    let lines = TextArea::layout_lines(&mut p, "中文测试段落", 10.0, 30.0);

    assert!(lines.len() >= 2, "got {lines:?}");
    assert!(lines.iter().all(|line| line.text.chars().count() <= 3));
}

#[test]
fn long_unbroken_tokens_wrap_by_character_with_byte_offsets() {
    let mut p = CapturePainter::default();
    let lines = TextArea::layout_lines(&mut p, "abcdefghij", 10.0, 18.0);

    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        vec!["abc", "def", "ghi", "j"]
    );
    assert_eq!(
        lines
            .iter()
            .map(|line| (line.start, line.end))
            .collect::<Vec<_>>(),
        vec![(0, 3), (3, 6), (6, 9), (9, 10)]
    );
    assert!(lines
        .iter()
        .all(|line| p.measure_text(&line.text, 10.0) <= 18.0));
}

#[test]
fn wrapped_spaces_are_preserved_with_byte_offsets() {
    let mut p = CapturePainter::default();
    let lines = TextArea::layout_lines(&mut p, "   ", 10.0, 6.0);

    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        vec![" ", " ", " "]
    );
    assert_eq!(
        lines
            .iter()
            .map(|line| (line.start, line.end))
            .collect::<Vec<_>>(),
        vec![(0, 1), (1, 2), (2, 3)]
    );
}

#[test]
fn cross_line_selection_paints_one_rect_per_line() {
    let mut state = jian_core::text_input::TextInputState::with_text("ab\ncd");
    state.set_caret(1, 0);
    state.drag_to(4, 0);
    let view = TextArea {
        state: &state,
        placeholder: "",
        focused: true,
        font_size: 10.0,
        now_ms: 0,
        pad_x: 8.0,
        max_visible_lines: 4,
    };
    let t = Tokens::dark();
    let mut p = CapturePainter::default();

    view.paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 80.0), &t);

    assert_eq!(p.fills_with(t.primary.with_alpha(0.35)), 2);
}

#[test]
fn blank_line_selection_paints_highlight() {
    let mut state = jian_core::text_input::TextInputState::with_text("a\n\nb");
    state.set_caret(2, 0);
    state.drag_to(3, 0);
    let view = TextArea {
        state: &state,
        placeholder: "",
        focused: true,
        font_size: 10.0,
        now_ms: 0,
        pad_x: 8.0,
        max_visible_lines: 4,
    };
    let t = Tokens::dark();
    let mut p = CapturePainter::default();

    view.paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 80.0), &t);

    assert_eq!(p.fills_with(t.primary.with_alpha(0.35)), 1);
}

#[test]
fn visible_window_keeps_caret_line_in_view() {
    let state = jian_core::text_input::TextInputState::with_text("a\nb\nc");
    let view = TextArea {
        state: &state,
        placeholder: "",
        focused: true,
        font_size: 10.0,
        now_ms: 0,
        pad_x: 8.0,
        max_visible_lines: 2,
    };
    let t = Tokens::dark();
    let mut p = CapturePainter::default();

    view.paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 80.0), &t);

    let texts: Vec<_> = p
        .texts()
        .map(|(content, _, _)| content.to_owned())
        .collect();
    assert_eq!(texts, vec!["b", "c"]);
}

#[test]
fn byte_offset_at_uses_wrapped_line_start_offsets() {
    let state = jian_core::text_input::TextInputState::with_text("ab\ncd");
    let view = TextArea {
        state: &state,
        placeholder: "",
        focused: true,
        font_size: 10.0,
        now_ms: 0,
        pad_x: 8.0,
        max_visible_lines: 4,
    };
    let mut p = CapturePainter::default();

    let byte = view.byte_offset_at(
        &mut p,
        Rect::xywh(0.0, 0.0, 120.0, 80.0),
        Point2D::new(8.0 + 5.5 + 2.0, 20.0),
        &Tokens::dark(),
    );

    assert_eq!(byte, 4);
}

#[test]
fn byte_offset_at_uses_supplied_density_for_wrapping() {
    let state = jian_core::text_input::TextInputState::with_text("中文");
    let view = TextArea {
        state: &state,
        placeholder: "",
        focused: true,
        font_size: 0.0,
        now_ms: 0,
        pad_x: 8.0,
        max_visible_lines: 4,
    };
    let touch = Tokens {
        density: crate::Density::Touch,
        ..Tokens::dark()
    };
    let mut p = CapturePainter::default();

    let byte = view.byte_offset_at(
        &mut p,
        Rect::xywh(0.0, 0.0, 44.0, 80.0),
        Point2D::new(8.0, PAD_Y + line_height(touch.density.font_size()) + 1.0),
        &touch,
    );

    assert_eq!(byte, "中".len());
}

#[test]
fn caret_line_prefers_later_line_at_soft_wrap_boundary() {
    let mut p = CapturePainter::default();
    let lines = TextArea::layout_lines(&mut p, "中文", 10.0, 10.0);

    assert_eq!(caret_line_index(&lines, "中".len()), Some(1));
}

#[test]
fn ime_composition_splits_prefix_preedit_and_suffix() {
    let mut state = jian_core::text_input::TextInputState::with_text("ab");
    state.set_caret(1, 0);
    state.set_composition("中", "中".len(), 0);
    let view = TextArea {
        state: &state,
        placeholder: "",
        focused: true,
        font_size: 10.0,
        now_ms: 0,
        pad_x: 8.0,
        max_visible_lines: 4,
    };
    let t = Tokens::dark();
    let mut p = CapturePainter::default();

    view.paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 80.0), &t);

    let texts: Vec<_> = p
        .texts()
        .map(|(content, origin, _)| (content.to_owned(), origin.x))
        .collect();
    assert_eq!(
        texts
            .iter()
            .map(|(content, _)| content.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "中", "b"]
    );
    assert!(texts[0].1 < texts[1].1);
    assert!(texts[1].1 < texts[2].1);
    assert!(p
        .ops
        .iter()
        .any(|op| matches!(op, PaintOp::StrokeLine(_, _, color, _) if *color == t.foreground)));
}
