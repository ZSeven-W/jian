use crate::{Painter, Point2D, Rect, TextLayout, Tokens};

const FONT_FAMILY: &str = "Inter";
const DEFAULT_PAD_X: f32 = 8.0;
const PAD_Y: f32 = 4.0;
const LINE_HEIGHT_MULT: f32 = 1.35;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLine {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

pub struct TextArea<'a> {
    pub state: &'a jian_core::text_input::TextInputState,
    pub placeholder: &'a str,
    pub focused: bool,
    pub font_size: f32,
    pub now_ms: u64,
    pub pad_x: f32,
    pub max_visible_lines: usize,
}

impl TextArea<'_> {
    pub fn paint(&self, p: &mut dyn Painter, rect: Rect, t: &Tokens) {
        let font_size = self.resolved_font_size(t);
        let pad_x = self.resolved_pad_x();
        let line_h = line_height(font_size);
        let content_w = (rect.size.x - 2.0 * pad_x).max(0.0);
        let text = self.state.text();
        let lines = Self::layout_lines(p, text, font_size, content_w);
        let visible_start = self.visible_start(&lines);
        let visible_count = self.visible_count(&lines);

        p.save();
        p.clip_rect(rect);

        if let Some((sel_start, sel_end)) = self.state.highlight_range() {
            for (visible_i, line) in lines
                .iter()
                .skip(visible_start)
                .take(visible_count)
                .enumerate()
            {
                if let Some((x0, x1)) = selection_x_range(p, line, sel_start, sel_end, font_size) {
                    let y = rect.origin.y + PAD_Y + visible_i as f32 * line_h;
                    p.fill_round_rect(
                        Rect::xywh(
                            rect.origin.x + pad_x + x0,
                            y - 2.0,
                            (x1 - x0).max(1.0),
                            line_h,
                        ),
                        3.0,
                        t.primary.with_alpha(0.35),
                    );
                }
            }
        }

        let composition = self.state.composition().filter(|c| !c.text.is_empty());
        let caret = safe_byte(text, self.state.caret());
        let composition_line = composition.and(caret_line_index(&lines, caret));

        if text.is_empty() && composition.is_none() {
            if !self.placeholder.is_empty() {
                draw_text(
                    p,
                    self.placeholder,
                    Point2D::new(rect.origin.x + pad_x, rect.origin.y + PAD_Y),
                    font_size,
                    t.muted_foreground,
                );
            }
        } else {
            for (visible_i, line) in lines
                .iter()
                .skip(visible_start)
                .take(visible_count)
                .enumerate()
            {
                let line_i = visible_start + visible_i;
                if line.text.is_empty() && composition_line != Some(line_i) {
                    continue;
                }
                let origin = Point2D::new(
                    rect.origin.x + pad_x,
                    rect.origin.y + PAD_Y + visible_i as f32 * line_h,
                );
                if let Some(composition) = composition.filter(|_| composition_line == Some(line_i))
                {
                    draw_line_with_composition(
                        p,
                        line,
                        origin,
                        font_size,
                        t.foreground,
                        composition,
                        caret,
                    );
                } else {
                    draw_text(p, &line.text, origin, font_size, t.foreground);
                }
            }
        }

        if self.focused
            && self.state.highlight_range().is_none()
            && self.state.caret_visible(self.now_ms)
        {
            if let Some(mut origin) = caret_origin(
                p,
                rect,
                pad_x,
                line_h,
                font_size,
                &lines,
                visible_start,
                visible_count,
                caret,
            ) {
                if let Some(composition) = self.state.composition() {
                    if !composition.text.is_empty() {
                        let cursor = safe_byte(&composition.text, composition.cursor);
                        origin.x += p.measure_text(&composition.text[..cursor], font_size);
                    }
                }
                p.fill_rect(
                    Rect::xywh(origin.x, origin.y, 1.5, font_size + 3.0),
                    t.foreground,
                );
            }
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
        let font_size = self.resolved_font_size(t);
        let pad_x = self.resolved_pad_x();
        let line_h = line_height(font_size);
        let content_w = (rect.size.x - 2.0 * pad_x).max(0.0);
        let lines = Self::layout_lines(p, self.state.text(), font_size, content_w);
        let visible_start = self.visible_start(&lines);
        let visible_count = self.visible_count(&lines);
        if lines.is_empty() || visible_count == 0 {
            return 0;
        }

        let row = ((point.y - (rect.origin.y + PAD_Y)) / line_h).floor() as isize;
        let row = row.clamp(0, visible_count.saturating_sub(1) as isize) as usize;
        let line = &lines[visible_start + row];
        byte_offset_in_line(p, line, point.x - (rect.origin.x + pad_x), font_size)
    }

    pub fn layout_lines(
        p: &mut dyn Painter,
        text: &str,
        font_size: f32,
        max_width: f32,
    ) -> Vec<TextLine> {
        if text.is_empty() {
            return vec![TextLine {
                text: String::new(),
                start: 0,
                end: 0,
            }];
        }

        let mut out = Vec::new();
        let mut offset = 0;
        for part in text.split_inclusive('\n') {
            let body = part.strip_suffix('\n').unwrap_or(part);
            wrap_segment(p, body, offset, font_size, max_width, &mut out);
            offset += part.len();
        }
        if text.ends_with('\n') {
            out.push(TextLine {
                text: String::new(),
                start: text.len(),
                end: text.len(),
            });
        }
        if out.is_empty() {
            out.push(TextLine {
                text: String::new(),
                start: 0,
                end: 0,
            });
        }
        out
    }

    fn visible_start(&self, lines: &[TextLine]) -> usize {
        let max = self.visible_count(lines);
        if lines.len() <= max {
            return 0;
        }
        let caret = safe_byte(self.state.text(), self.state.caret());
        let caret_line = caret_line_index(lines, caret).unwrap_or(lines.len() - 1);
        caret_line
            .saturating_add(1)
            .saturating_sub(max)
            .min(lines.len() - max)
    }

    fn visible_count(&self, lines: &[TextLine]) -> usize {
        let requested = if self.max_visible_lines == 0 {
            lines.len()
        } else {
            self.max_visible_lines
        };
        requested.min(lines.len())
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

fn line_height(font_size: f32) -> f32 {
    font_size * LINE_HEIGHT_MULT
}

fn draw_text(
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

fn draw_line_with_composition(
    p: &mut dyn Painter,
    line: &TextLine,
    origin: Point2D,
    font_size: f32,
    color: crate::Color,
    composition: &jian_core::text_input::Composition,
    caret: usize,
) {
    let rel = caret.saturating_sub(line.start).min(line.text.len());
    let rel = jian_core::text_input::prev_char_boundary(&line.text, rel);
    let prefix = &line.text[..rel];
    let suffix = &line.text[rel..];
    let mut x = origin.x;

    if !prefix.is_empty() {
        draw_text(p, prefix, Point2D::new(x, origin.y), font_size, color);
        x += p.measure_text(prefix, font_size);
    }

    draw_text(
        p,
        &composition.text,
        Point2D::new(x, origin.y),
        font_size,
        color,
    );
    let composition_w = p.measure_text(&composition.text, font_size).max(1.0);
    let underline_y = origin.y + font_size + 2.0;
    p.stroke_line(
        Point2D::new(x, underline_y),
        Point2D::new(x + composition_w, underline_y),
        color,
        1.0,
    );
    x += composition_w;

    if !suffix.is_empty() {
        draw_text(p, suffix, Point2D::new(x, origin.y), font_size, color);
    }
}

fn wrap_segment(
    p: &mut dyn Painter,
    segment: &str,
    segment_start: usize,
    font_size: f32,
    max_width: f32,
    out: &mut Vec<TextLine>,
) {
    if segment.is_empty() {
        out.push(TextLine {
            text: String::new(),
            start: segment_start,
            end: segment_start,
        });
        return;
    }
    if max_width <= 0.0 || p.measure_text(segment, font_size) <= max_width {
        out.push(TextLine {
            text: segment.to_owned(),
            start: segment_start,
            end: segment_start + segment.len(),
        });
        return;
    }

    let chars: Vec<(usize, char)> = segment.char_indices().collect();
    let mut current = String::new();
    let mut current_start = 0;
    let mut current_end = 0;
    let mut i = 0;

    while i < chars.len() {
        let (byte, ch) = chars[i];
        if is_cjk(ch) {
            let token = ch.to_string();
            let token_end = byte + ch.len_utf8();
            append_wrapped_token(
                p,
                &mut current,
                &mut current_start,
                &mut current_end,
                Token {
                    text: &token,
                    start: byte,
                    end: token_end,
                },
                segment_start,
                font_size,
                max_width,
                out,
            );
            i += 1;
        } else if ch == ' ' {
            let token_end = byte + ch.len_utf8();
            append_wrapped_token(
                p,
                &mut current,
                &mut current_start,
                &mut current_end,
                Token {
                    text: " ",
                    start: byte,
                    end: token_end,
                },
                segment_start,
                font_size,
                max_width,
                out,
            );
            i += 1;
        } else {
            let start = byte;
            let mut end = byte;
            let mut word = String::new();
            while i < chars.len() && chars[i].1 != ' ' && !is_cjk(chars[i].1) {
                let (b, c) = chars[i];
                word.push(c);
                end = b + c.len_utf8();
                i += 1;
            }
            append_wrapped_token(
                p,
                &mut current,
                &mut current_start,
                &mut current_end,
                Token {
                    text: &word,
                    start,
                    end,
                },
                segment_start,
                font_size,
                max_width,
                out,
            );
        }
    }

    push_current(&mut current, current_start, current_end, segment_start, out);
}

struct Token<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

#[allow(clippy::too_many_arguments)]
fn append_wrapped_token(
    p: &mut dyn Painter,
    current: &mut String,
    current_start: &mut usize,
    current_end: &mut usize,
    token: Token<'_>,
    segment_start: usize,
    font_size: f32,
    max_width: f32,
    out: &mut Vec<TextLine>,
) {
    let mut probe = current.clone();
    probe.push_str(token.text);
    if p.measure_text(&probe, font_size) > max_width {
        push_current(current, *current_start, *current_end, segment_start, out);
        append_token_to_empty_line(
            p,
            current,
            current_start,
            current_end,
            token,
            segment_start,
            font_size,
            max_width,
            out,
        );
    } else {
        if current.is_empty() {
            *current_start = token.start;
        }
        *current = probe;
        *current_end = token.end;
    }
}

#[allow(clippy::too_many_arguments)]
fn append_token_to_empty_line(
    p: &mut dyn Painter,
    current: &mut String,
    current_start: &mut usize,
    current_end: &mut usize,
    token: Token<'_>,
    segment_start: usize,
    font_size: f32,
    max_width: f32,
    out: &mut Vec<TextLine>,
) {
    if token.text.is_empty() {
        return;
    }
    if max_width <= 0.0 || p.measure_text(token.text, font_size) <= max_width {
        current.push_str(token.text);
        *current_start = token.start;
        *current_end = token.end;
        return;
    }

    for (byte, ch) in token.text.char_indices() {
        let token_byte = token.start + byte;
        let token_end = token_byte + ch.len_utf8();
        let mut probe = current.clone();
        probe.push(ch);
        if !current.is_empty() && p.measure_text(&probe, font_size) > max_width {
            push_current(current, *current_start, *current_end, segment_start, out);
            current.push(ch);
            *current_start = token_byte;
            *current_end = token_end;
        } else {
            if current.is_empty() {
                *current_start = token_byte;
            }
            *current = probe;
            *current_end = token_end;
        }
    }
}

fn push_current(
    current: &mut String,
    current_start: usize,
    current_end: usize,
    segment_start: usize,
    out: &mut Vec<TextLine>,
) {
    if current.is_empty() {
        return;
    }
    out.push(TextLine {
        text: std::mem::take(current),
        start: segment_start + current_start,
        end: segment_start + current_end,
    });
}

fn selection_x_range(
    p: &mut dyn Painter,
    line: &TextLine,
    sel_start: usize,
    sel_end: usize,
    font_size: f32,
) -> Option<(f32, f32)> {
    if line.start == line.end {
        return (sel_start <= line.start && line.start < sel_end)
            .then_some((0.0, font_size.max(1.0)));
    }

    let start = sel_start.max(line.start);
    let end = sel_end.min(line.end);
    if start >= end {
        return None;
    }
    let rel_start = start - line.start;
    let rel_end = end - line.start;
    Some((
        p.measure_text(&line.text[..rel_start], font_size),
        p.measure_text(&line.text[..rel_end], font_size),
    ))
}

fn byte_offset_in_line(p: &mut dyn Painter, line: &TextLine, x: f32, font_size: f32) -> usize {
    if x <= 0.0 || line.text.is_empty() {
        return line.start;
    }
    let mut cursor_x = 0.0;
    for (byte, ch) in line.text.char_indices() {
        let mut buf = [0; 4];
        let s = ch.encode_utf8(&mut buf);
        let width = p.measure_text(s, font_size);
        if x < cursor_x + width / 2.0 {
            return line.start + byte;
        }
        cursor_x += width;
    }
    line.end
}

#[allow(clippy::too_many_arguments)]
fn caret_origin(
    p: &mut dyn Painter,
    rect: Rect,
    pad_x: f32,
    line_h: f32,
    font_size: f32,
    lines: &[TextLine],
    visible_start: usize,
    visible_count: usize,
    caret: usize,
) -> Option<Point2D> {
    let line_i = caret_line_index(lines, caret)?;
    if line_i < visible_start || line_i >= visible_start + visible_count {
        return None;
    }
    let line = &lines[line_i];
    let rel = caret.saturating_sub(line.start).min(line.text.len());
    let rel = jian_core::text_input::prev_char_boundary(&line.text, rel);
    let x = p.measure_text(&line.text[..rel], font_size);
    let visible_i = line_i - visible_start;
    Some(Point2D::new(
        rect.origin.x + pad_x + x,
        rect.origin.y + PAD_Y + visible_i as f32 * line_h,
    ))
}

fn caret_line_index(lines: &[TextLine], caret: usize) -> Option<usize> {
    lines.iter().enumerate().position(|(i, line)| {
        if line.start == line.end {
            return caret == line.start;
        }
        let end_is_soft_wrap = lines.get(i + 1).is_some_and(|next| next.start == line.end);
        if end_is_soft_wrap {
            caret >= line.start && caret < line.end
        } else {
            caret >= line.start && caret <= line.end
        }
    })
}

fn safe_byte(text: &str, byte: usize) -> usize {
    jian_core::text_input::prev_char_boundary(text, byte)
}

fn is_cjk(ch: char) -> bool {
    let c = ch as u32;
    (0x4e00..=0x9fff).contains(&c)
        || (0x3400..=0x4dbf).contains(&c)
        || (0x3040..=0x30ff).contains(&c)
        || (0xac00..=0xd7af).contains(&c)
        || (0x3000..=0x303f).contains(&c)
        || (0xff00..=0xffef).contains(&c)
}

#[cfg(test)]
mod tests;
