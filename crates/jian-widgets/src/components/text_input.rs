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
        // Top-left origin: hosts that render TextInputView wrap the backend in a
        // `BaselineAdjustingBackend` that shifts draw_text to the baseline, so
        // this stays top-relative (unlike the label components, which emit the
        // baseline directly).
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

        let composition = self.state.composition().filter(|c| !c.text.is_empty());

        if let Some(composition) = composition {
            let caret = self.safe_caret();
            let prefix = &text[..caret];
            let suffix = &text[caret..];
            let mut x = base_x;
            if !prefix.is_empty() {
                self.draw_text(p, prefix, Point2D::new(x, text_y), font_size, t.foreground);
                x += p.measure_text(prefix, font_size);
            }

            self.draw_text(
                p,
                &composition.text,
                Point2D::new(x, text_y),
                font_size,
                t.foreground,
            );
            let composition_w = p.measure_text(&composition.text, font_size).max(1.0);
            let underline_y = text_y + font_size + 2.0;
            p.stroke_line(
                Point2D::new(x, underline_y),
                Point2D::new(x + composition_w, underline_y),
                t.foreground,
                1.0,
            );
            x += composition_w;

            if !suffix.is_empty() {
                self.draw_text(p, suffix, Point2D::new(x, text_y), font_size, t.foreground);
            }
        } else if text.is_empty() {
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

        if self.focused
            && self.state.highlight_range().is_none()
            && self.state.caret_visible(self.now_ms)
        {
            let caret_x = self.visual_caret_x(p, base_x, font_size);
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

    pub fn byte_offset_at(
        &self,
        p: &mut dyn Painter,
        rect: Rect,
        point: Point2D,
        t: &Tokens,
    ) -> usize {
        let text = self.state.text();
        let font_size = self.resolved_font_size(t);
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
        let layout = TextLayout::single_run(
            content,
            FONT_FAMILY,
            font_size,
            color.to_jian(),
            Point2D::new(0.0, 0.0),
        );
        p.draw_text(&layout, origin);
    }

    fn horizontal_shift(&self, p: &mut dyn Painter, rect: Rect, font_size: f32, pad_x: f32) -> f32 {
        let caret_px = self.visual_caret_x(p, 0.0, font_size);
        let visible_w = (rect.size.x - 2.0 * pad_x).max(0.0);
        (caret_px - visible_w).max(0.0)
    }

    fn visual_caret_x(&self, p: &mut dyn Painter, base_x: f32, font_size: f32) -> f32 {
        let text = self.state.text();
        let caret = self.safe_caret();
        let mut x = base_x + p.measure_text(&text[..caret], font_size);
        if let Some(composition) = self.state.composition() {
            if !composition.text.is_empty() {
                let cursor = jian_core::text_input::prev_char_boundary(
                    &composition.text,
                    composition.cursor,
                );
                x += p.measure_text(&composition.text[..cursor], font_size);
            }
        }
        x
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
    fn ime_composition_splits_prefix_preedit_and_suffix() {
        let mut state = jian_core::text_input::TextInputState::with_text("ab");
        state.set_caret(1, 0);
        state.set_composition("中", "中".len(), 0);
        let view = TextInputView {
            state: &state,
            placeholder: "",
            focused: true,
            font_size: 10.0,
            now_ms: 0,
            pad_x: 8.0,
        };
        let t = Tokens::dark();
        let mut p = CapturePainter::default();

        view.paint(&mut p, Rect::xywh(0.0, 0.0, 120.0, 30.0), &t);

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
            view.byte_offset_at(
                &mut p,
                Rect::xywh(0.0, 0.0, 120.0, 30.0),
                point,
                &Tokens::dark()
            ),
            2
        );
    }

    #[test]
    fn byte_offset_at_uses_supplied_density_font_size() {
        let state = jian_core::text_input::TextInputState::with_text("ab");
        let view = TextInputView {
            state: &state,
            placeholder: "",
            focused: true,
            font_size: 0.0,
            now_ms: 0,
            pad_x: 8.0,
        };
        let touch = Tokens {
            density: crate::Density::Touch,
            ..Tokens::dark()
        };
        let mut p = CapturePainter::default();
        let point = Point2D::new(8.0 + 11.0, 10.0);

        assert_eq!(
            view.byte_offset_at(&mut p, Rect::xywh(0.0, 0.0, 120.0, 30.0), point, &touch),
            1
        );
    }
}
