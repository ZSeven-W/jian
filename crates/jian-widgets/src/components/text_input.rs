use crate::{Painter, Point2D, Rect, TextLayout, Tokens};

const FONT_FAMILY: &str = "Inter";
const DEFAULT_PAD_X: f32 = 8.0;

pub struct TextInputView<'a> {
    pub state: &'a jian_core::text_input::TextInputState,
    pub placeholder: &'a str,
    pub focused: bool,
    pub font_size: f32,
    pub now_ms: u64,
    pub pad_x: f32,
}

impl TextInputView<'_> {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        let font_size = self.resolved_font_size(t);
        let pad_x = self.resolved_pad_x();
        let text = self.state.text();
        let shift = self.horizontal_shift(p, rect, font_size, pad_x);
        let base_x = rect.origin.x + pad_x - shift;
        let text_y = rect.origin.y + (rect.size.y - font_size) / 2.0;

        p.save();
        p.clip_rect(rect);

        if let Some((start, end)) = self.state.highlight_range() {
            let start = jian_core::text_input::prev_char_boundary(text, start);
            let end = jian_core::text_input::prev_char_boundary(text, end);
            let x0 = p.measure_text(&text[..start], font_size);
            let x1 = p.measure_text(&text[..end], font_size);
            p.fill_round_rect(
                Rect::xywh(
                    base_x + x0,
                    text_y - 2.0,
                    (x1 - x0).max(1.0),
                    font_size + 4.0,
                ),
                3.0,
                t.primary.with_alpha(0.35),
            );
        }

        if text.is_empty() {
            if !self.placeholder.is_empty() {
                self.draw_text(
                    p,
                    self.placeholder,
                    Point2D::new(base_x, text_y),
                    font_size,
                    t.muted_foreground,
                );
            }
        } else {
            self.draw_text(
                p,
                text,
                Point2D::new(base_x, text_y),
                font_size,
                t.foreground,
            );
        }

        if let Some(composition) = self.state.composition() {
            if !composition.text.is_empty() {
                let caret_x = base_x + p.measure_text(&text[..self.safe_caret()], font_size);
                let origin = Point2D::new(caret_x, text_y);
                self.draw_text(p, &composition.text, origin, font_size, t.foreground);
                let width = p.measure_text(&composition.text, font_size).max(1.0);
                let underline_y = text_y + font_size + 2.0;
                p.stroke_line(
                    Point2D::new(caret_x, underline_y),
                    Point2D::new(caret_x + width, underline_y),
                    t.foreground,
                    1.0,
                );
            }
        }

        if self.focused
            && self.state.highlight_range().is_none()
            && self.state.caret_visible(self.now_ms)
        {
            let caret_x = base_x + p.measure_text(&text[..self.safe_caret()], font_size);
            let caret_h = font_size + 3.0;
            p.fill_rect(
                Rect::xywh(
                    caret_x,
                    rect.origin.y + (rect.size.y - caret_h) / 2.0,
                    1.5,
                    caret_h,
                ),
                t.foreground,
            );
        }

        p.restore();
    }

    pub fn byte_offset_at(&self, p: &mut dyn Painter, rect: Rect, point: Point2D) -> usize {
        let text = self.state.text();
        let font_size = self.resolved_font_size(&Tokens::default());
        let pad_x = self.resolved_pad_x();
        let shift = self.horizontal_shift(p, rect, font_size, pad_x);
        let target_x = point.x - (rect.origin.x + pad_x - shift);
        if target_x <= 0.0 {
            return 0;
        }

        let mut x = 0.0;
        for (byte, ch) in text.char_indices() {
            let mut buf = [0; 4];
            let s = ch.encode_utf8(&mut buf);
            let w = p.measure_text(s, font_size);
            if target_x < x + w / 2.0 {
                return byte;
            }
            x += w;
        }
        text.len()
    }

    fn draw_text(
        &self,
        p: &mut dyn Painter,
        content: &str,
        origin: Point2D,
        font_size: f32,
        color: crate::Color,
    ) {
        let layout =
            TextLayout::single_run(content, FONT_FAMILY, font_size, color.to_jian(), origin);
        p.draw_text(&layout, origin);
    }

    fn horizontal_shift(&self, p: &mut dyn Painter, rect: Rect, font_size: f32, pad_x: f32) -> f32 {
        let text = self.state.text();
        let caret_px = p.measure_text(&text[..self.safe_caret()], font_size);
        let visible_w = (rect.size.x - 2.0 * pad_x).max(0.0);
        (caret_px - visible_w).max(0.0)
    }

    fn safe_caret(&self) -> usize {
        jian_core::text_input::prev_char_boundary(self.state.text(), self.state.caret())
    }

    fn resolved_font_size(&self, t: &Tokens) -> f32 {
        if self.font_size > 0.0 {
            self.font_size
        } else {
            t.density.font_size()
        }
    }

    fn resolved_pad_x(&self) -> f32 {
        if self.pad_x > 0.0 {
            self.pad_x
        } else {
            DEFAULT_PAD_X
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    #[test]
    fn placeholder_paints_with_muted_foreground() {
        let state = jian_core::text_input::TextInputState::default();
        let view = TextInputView {
            state: &state,
            placeholder: "Name",
            focused: false,
            font_size: 13.0,
            now_ms: 0,
            pad_x: 8.0,
        };
        let t = Tokens::dark();
        let mut p = CapturePainter::default();

        view.paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 30.0), &t);

        let (content, _, color) = p.texts().next().expect("placeholder should paint");
        assert_eq!(content, "Name");
        assert_eq!(color, t.muted_foreground.to_jian());
    }

    #[test]
    fn selected_text_paints_primary_selection_rect() {
        let mut state = jian_core::text_input::TextInputState::with_text("abcd");
        state.select_all();
        let view = TextInputView {
            state: &state,
            placeholder: "",
            focused: true,
            font_size: 13.0,
            now_ms: 0,
            pad_x: 8.0,
        };
        let t = Tokens::dark();
        let mut p = CapturePainter::default();

        view.paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 30.0), &t);

        assert_eq!(p.fills_with(t.primary.with_alpha(0.35)), 1);
    }

    #[test]
    fn hidden_blink_phase_does_not_paint_caret() {
        let state = jian_core::text_input::TextInputState::with_text("abc");
        let view = TextInputView {
            state: &state,
            placeholder: "",
            focused: true,
            font_size: 13.0,
            now_ms: 750,
            pad_x: 8.0,
        };
        let t = Tokens::dark();
        let mut p = CapturePainter::default();

        view.paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 30.0), &t);

        assert!(!p
            .ops
            .iter()
            .any(|op| matches!(op, PaintOp::FillRect(_, color) if *color == t.foreground)));
    }

    #[test]
    fn byte_offset_at_uses_measured_character_midpoints() {
        let state = jian_core::text_input::TextInputState::with_text("abcd");
        let view = TextInputView {
            state: &state,
            placeholder: "",
            focused: true,
            font_size: 10.0,
            now_ms: 0,
            pad_x: 8.0,
        };
        let mut p = CapturePainter::default();
        let point = Point2D::new(8.0 + 5.5 + 5.5 + 2.0, 10.0);

        assert_eq!(
            view.byte_offset_at(&mut p, Rect::xywh(0.0, 0.0, 120.0, 30.0), point),
            2
        );
    }
}
