//! ColorPicker — a floating HSV colour picker: a saturation×value gradient box,
//! a rainbow hue strip with handles, a current swatch, R/G/B readout boxes and a
//! hex bar. Self-contained: the caller supplies (hue, sat, val), a title, and the
//! eyedropper / close icon paths, and gets paint + a pure hit-test + the drag
//! math (`sv_at` / `hue_at`). The host keeps the anchor (where the panel floats),
//! the persistent drag state, and the close / eyedropper / commit dispatch.
//!
//! The SV box is software-painted as a grid of `fill_rect` cells (the `Painter`
//! has no 2D gradient), then framed — cheap and faithful at this size.

use crate::{Color, Painter, Point2D, Rect, TextLayout, Tokens};

pub const PICKER_WIDTH: f32 = 240.0;
pub const PICKER_HEIGHT: f32 = 336.0;

const FONT_FAMILY: &str = "Inter";
const PAD: f32 = 8.0;
const HEADER_HEIGHT: f32 = 28.0;
const SV_HEIGHT: f32 = 140.0;
const ROW_GAP: f32 = 8.0;
const HUE_HEIGHT: f32 = 12.0;
const HUE_HANDLE_R: f32 = 7.0;
const SV_HANDLE_R: f32 = 6.0;
const SWATCH_SIZE: f32 = 28.0;
const RGB_BOX_W: f32 = 56.0;
const RGB_BOX_H: f32 = 32.0;
const HEX_HEIGHT: f32 = 28.0;

/// What a press inside the picker landed on. The host maps `SvBox` / `HueSlider`
/// to a persistent drag and dispatches `Eyedropper` / `Close`; `Inside` swallows
/// the press so an in-panel click keeps the picker open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorPickerHit {
    SvBox,
    HueSlider,
    Eyedropper,
    Close,
    Inside,
}

/// A floating HSV colour picker. `hue` is 0..360, `sat` / `val` are 0..1.
pub struct ColorPicker<'a> {
    pub hue: f32,
    pub sat: f32,
    pub val: f32,
    /// Header title (e.g. "Fill" / "Stroke") — the host owns the wording.
    pub title: &'a str,
    /// lucide icon path(s) for the eyedropper button.
    pub eyedropper_icon: &'a [&'a str],
    /// lucide icon path(s) for the close (×) chip.
    pub close_icon: &'a [&'a str],
}

impl ColorPicker<'_> {
    pub fn paint(&self, p: &mut dyn Painter, panel: Rect, t: &Tokens) {
        p.fill_round_rect(panel, 8.0, t.card);
        p.stroke_round_rect(panel, 8.0, t.border, 1.0);

        // Header title.
        let title = TextLayout::single_run(
            self.title,
            FONT_FAMILY,
            13.0,
            t.foreground.to_jian(),
            Point2D::new(0.0, 0.0),
        );
        p.draw_text(
            &title,
            Point2D::new(panel.origin.x + PAD, header_top(panel) + 19.0),
        );

        paint_sv_box(p, sv_rect(panel), self.hue);
        paint_hue_strip(p, hue_rect(panel));

        // Eyedropper + current swatch.
        let eye = eyedropper_rect(panel);
        let eye_tl = Point2D::new(eye.origin.x, eye.origin.y + 4.0);
        for d in self.eyedropper_icon {
            p.stroke_svg_path(d, eye_tl, 16.0, t.muted_foreground, 1.5);
        }
        let swatch = swatch_rect(panel);
        let cur = hsv_to_rgb(self.hue, self.sat, self.val);
        p.fill_round_rect(swatch, 14.0, cur);
        p.stroke_round_rect(swatch, 14.0, t.border, 1.0);

        // R/G/B numeric boxes.
        let (r, g, b) = (
            (cur.r * 255.0).round() as u8,
            (cur.g * 255.0).round() as u8,
            (cur.b * 255.0).round() as u8,
        );
        let rgb_y = sv_rect(panel).origin.y + SV_HEIGHT + ROW_GAP + 36.0 + ROW_GAP;
        for (i, (label, value)) in [("R", r), ("G", g), ("B", b)].iter().enumerate() {
            let bx = panel.origin.x + PAD + (i as f32) * (RGB_BOX_W + 8.0);
            let box_rect = Rect::xywh(bx, rgb_y, RGB_BOX_W, RGB_BOX_H);
            p.stroke_round_rect(box_rect, 6.0, t.border, 1.0);
            let val_str = value.to_string();
            let text_w = p.measure_text(&val_str, 14.0);
            let text = TextLayout::single_run(
                &val_str,
                FONT_FAMILY,
                14.0,
                t.foreground.to_jian(),
                Point2D::new(0.0, 0.0),
            );
            p.draw_text(
                &text,
                Point2D::new(bx + (RGB_BOX_W - text_w) / 2.0, rgb_y + 21.0),
            );
            let lab_w = p.measure_text(label, 12.0);
            let lab = TextLayout::single_run(
                label,
                FONT_FAMILY,
                12.0,
                t.muted_foreground.to_jian(),
                Point2D::new(0.0, 0.0),
            );
            p.draw_text(
                &lab,
                Point2D::new(bx + (RGB_BOX_W - lab_w) / 2.0, rgb_y + RGB_BOX_H + 14.0),
            );
        }

        // Hex bar.
        let hex = Rect::xywh(
            panel.origin.x + PAD,
            panel.origin.y + PICKER_HEIGHT - PAD - HEX_HEIGHT,
            PICKER_WIDTH - PAD * 2.0,
            HEX_HEIGHT,
        );
        p.fill_round_rect(hex, 6.0, t.muted);
        p.stroke_round_rect(hex, 6.0, t.primary, 1.0);
        let mini = Rect::xywh(hex.origin.x + 6.0, hex.origin.y + 6.0, 16.0, 16.0);
        p.fill_round_rect(mini, 3.0, cur);
        p.stroke_round_rect(mini, 3.0, t.border, 1.0);
        let hex_str = format!("#{:02x}{:02x}{:02x}", r, g, b);
        let hex_layout = TextLayout::single_run(
            &hex_str,
            FONT_FAMILY,
            13.0,
            t.foreground.to_jian(),
            Point2D::new(0.0, 0.0),
        );
        p.draw_text(
            &hex_layout,
            Point2D::new(hex.origin.x + 32.0, hex.origin.y + 19.0),
        );

        // Close (×) chip — a dim disc so it reads against the SV gradient.
        let close = close_rect(panel);
        let chip_bg = Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.45,
        };
        p.fill_round_rect(close, 11.0, chip_bg);
        p.stroke_round_rect(close, 11.0, t.border, 1.0);
        let close_tl = Point2D::new(close.origin.x + 4.0, close.origin.y + 4.0);
        for d in self.close_icon {
            p.stroke_svg_path(d, close_tl, 14.0, Color::WHITE, 1.8);
        }

        // Handles on top.
        paint_sv_handle(p, sv_rect(panel), self.sat, self.val, t);
        paint_hue_handle(p, hue_rect(panel), self.hue, t);
    }

    /// Map a press inside the panel to a hit, or `None` if outside it.
    pub fn hit(panel: Rect, point: Point2D) -> Option<ColorPickerHit> {
        if !panel.contains(point) {
            return None;
        }
        // Close sits over the SV box's corner — check it first.
        if close_rect(panel).contains(point) {
            return Some(ColorPickerHit::Close);
        }
        if sv_rect(panel).contains(point) {
            return Some(ColorPickerHit::SvBox);
        }
        if hue_rect(panel).contains(point) {
            return Some(ColorPickerHit::HueSlider);
        }
        if eyedropper_rect(panel).contains(point) {
            return Some(ColorPickerHit::Eyedropper);
        }
        Some(ColorPickerHit::Inside)
    }

    /// `(sat, val)` for a point dragged inside the SV box.
    pub fn sv_at(panel: Rect, point: Point2D) -> (f32, f32) {
        let sv = sv_rect(panel);
        let x = ((point.x - sv.origin.x) / sv.size.x).clamp(0.0, 1.0);
        let y = ((point.y - sv.origin.y) / sv.size.y).clamp(0.0, 1.0);
        (x, 1.0 - y)
    }

    /// Hue (0..360) for a point dragged across the hue strip.
    pub fn hue_at(panel: Rect, point: Point2D) -> f32 {
        let h = hue_rect(panel);
        (((point.x - h.origin.x) / h.size.x).clamp(0.0, 1.0)) * 360.0
    }
}

fn header_top(panel: Rect) -> f32 {
    panel.origin.y + PAD
}

fn sv_rect(panel: Rect) -> Rect {
    Rect::xywh(
        panel.origin.x + PAD,
        header_top(panel) + HEADER_HEIGHT,
        PICKER_WIDTH - PAD * 2.0,
        SV_HEIGHT,
    )
}

fn hue_rect(panel: Rect) -> Rect {
    let y = sv_rect(panel).origin.y + SV_HEIGHT + ROW_GAP + 8.0;
    let x = panel.origin.x + PAD + SWATCH_SIZE + 24.0 + 8.0;
    let w = PICKER_WIDTH - PAD * 2.0 - SWATCH_SIZE - 24.0 - 8.0;
    Rect::xywh(x, y, w, HUE_HEIGHT)
}

fn eyedropper_rect(panel: Rect) -> Rect {
    let y = sv_rect(panel).origin.y + SV_HEIGHT + ROW_GAP + 6.0;
    Rect::xywh(panel.origin.x + PAD, y, 20.0, 20.0)
}

fn swatch_rect(panel: Rect) -> Rect {
    let y = sv_rect(panel).origin.y + SV_HEIGHT + ROW_GAP;
    Rect::xywh(panel.origin.x + PAD + 24.0, y, SWATCH_SIZE, SWATCH_SIZE)
}

fn close_rect(panel: Rect) -> Rect {
    Rect::xywh(
        panel.origin.x + PICKER_WIDTH - PAD - 22.0,
        header_top(panel) + (HEADER_HEIGHT - 22.0) / 2.0,
        22.0,
        22.0,
    )
}

fn paint_sv_box(p: &mut dyn Painter, sv: Rect, hue: f32) {
    const STEPS_X: u32 = 18;
    const STEPS_Y: u32 = 14;
    let pure = hsv_to_rgb(hue, 1.0, 1.0);
    let cell_w = sv.size.x / STEPS_X as f32;
    let cell_h = sv.size.y / STEPS_Y as f32;
    for ix in 0..STEPS_X {
        for iy in 0..STEPS_Y {
            let s = (ix as f32 + 0.5) / STEPS_X as f32;
            let v = 1.0 - (iy as f32 + 0.5) / STEPS_Y as f32;
            let color = Color {
                r: ((1.0 - s) + s * pure.r) * v,
                g: ((1.0 - s) + s * pure.g) * v,
                b: ((1.0 - s) + s * pure.b) * v,
                a: 1.0,
            };
            let cell = Rect::xywh(
                sv.origin.x + ix as f32 * cell_w,
                sv.origin.y + iy as f32 * cell_h,
                cell_w + 0.5,
                cell_h + 0.5,
            );
            p.fill_rect(cell, color);
        }
    }
    p.stroke_round_rect(
        sv,
        4.0,
        Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.25,
        },
        1.0,
    );
}

fn paint_hue_strip(p: &mut dyn Painter, hue: Rect) {
    const STEPS: u32 = 32;
    let cell_w = hue.size.x / STEPS as f32;
    for i in 0..STEPS {
        let h = (i as f32 + 0.5) / STEPS as f32 * 360.0;
        let cell = Rect::xywh(
            hue.origin.x + i as f32 * cell_w,
            hue.origin.y,
            cell_w + 0.5,
            hue.size.y,
        );
        p.fill_round_rect(cell, hue.size.y / 2.0, hsv_to_rgb(h, 1.0, 1.0));
    }
}

fn paint_sv_handle(p: &mut dyn Painter, sv: Rect, sat: f32, val: f32, t: &Tokens) {
    let x = sv.origin.x + sat * sv.size.x;
    let y = sv.origin.y + (1.0 - val) * sv.size.y;
    let ring = Rect::xywh(
        x - SV_HANDLE_R,
        y - SV_HANDLE_R,
        SV_HANDLE_R * 2.0,
        SV_HANDLE_R * 2.0,
    );
    p.stroke_oval(ring, t.background, 3.0);
    p.stroke_oval(ring, Color::WHITE, 1.5);
}

fn paint_hue_handle(p: &mut dyn Painter, hue: Rect, current_hue: f32, t: &Tokens) {
    let cx0 = hue.origin.x + (current_hue / 360.0).clamp(0.0, 1.0) * hue.size.x;
    let cy0 = hue.origin.y + hue.size.y / 2.0;
    let ring = Rect::xywh(
        cx0 - HUE_HANDLE_R,
        cy0 - HUE_HANDLE_R,
        HUE_HANDLE_R * 2.0,
        HUE_HANDLE_R * 2.0,
    );
    let center = Rect::xywh(cx0 - 5.0, cy0 - 5.0, 10.0, 10.0);
    p.fill_oval(center, hsv_to_rgb(current_hue, 1.0, 1.0));
    p.stroke_oval(ring, t.background, 1.0);
    p.stroke_oval(ring, Color::WHITE, 2.0);
}

/// HSV → RGB, h 0..360, s/v 0..1, alpha 1.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Color {
    let h = h.rem_euclid(360.0);
    let c = v * s;
    let hh = h / 60.0;
    let x = c * (1.0 - (hh.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match hh as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    Color {
        r: r1 + m,
        g: g1 + m,
        b: b1 + m,
        a: 1.0,
    }
}

/// RGB (0..1) → HSV (h 0..360, s 0..1, v 0..1).
pub fn rgb_to_hsv(c: Color) -> (f32, f32, f32) {
    let (r, g, b) = (c.r, c.g, c.b);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let v = max;
    let delta = max - min;
    let s = if max <= 0.0 { 0.0 } else { delta / max };
    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };
    (if h < 0.0 { h + 360.0 } else { h }, s, v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CapturePainter, PaintOp};

    fn picker(hue: f32, sat: f32, val: f32) -> ColorPicker<'static> {
        ColorPicker {
            hue,
            sat,
            val,
            title: "Fill",
            eyedropper_icon: &["M2 2l4 4"],
            close_icon: &["M18 6 6 18", "m6 6 12 12"],
        }
    }

    #[test]
    fn hsv_rgb_roundtrips() {
        for &(h, s, v) in &[
            (0.0, 1.0, 1.0),
            (120.0, 0.5, 0.8),
            (240.0, 1.0, 0.5),
            (33.0, 0.7, 0.9),
        ] {
            let c = hsv_to_rgb(h, s, v);
            let (h2, s2, v2) = rgb_to_hsv(c);
            assert!((s - s2).abs() < 0.01 && (v - v2).abs() < 0.01);
            let dh = (h - h2).abs() % 360.0;
            assert!(dh < 0.5 || (360.0 - dh) < 0.5, "hue {h} != {h2}");
        }
    }

    #[test]
    fn hit_order_close_then_sv_then_hue() {
        let panel = Rect::xywh(0.0, 0.0, PICKER_WIDTH, PICKER_HEIGHT);
        // A point in the SV box maps to SvBox.
        let sv = super::sv_rect(panel);
        assert_eq!(
            ColorPicker::hit(panel, Point2D::new(sv.origin.x + 10.0, sv.origin.y + 10.0)),
            Some(ColorPickerHit::SvBox)
        );
        // The close chip wins over the SV box at the top-right corner.
        let close = super::close_rect(panel);
        assert_eq!(
            ColorPicker::hit(
                panel,
                Point2D::new(close.origin.x + 5.0, close.origin.y + 5.0)
            ),
            Some(ColorPickerHit::Close)
        );
        // Outside the panel → None.
        assert_eq!(ColorPicker::hit(panel, Point2D::new(-5.0, -5.0)), None);
    }

    #[test]
    fn sv_at_maps_corners() {
        let panel = Rect::xywh(0.0, 0.0, PICKER_WIDTH, PICKER_HEIGHT);
        let sv = super::sv_rect(panel);
        // Top-left of SV box = sat 0, val 1.
        let (s, v) = ColorPicker::sv_at(panel, Point2D::new(sv.origin.x, sv.origin.y));
        assert!(s.abs() < 0.001 && (v - 1.0).abs() < 0.001);
        // Bottom-right = sat 1, val 0.
        let (s, v) = ColorPicker::sv_at(
            panel,
            Point2D::new(sv.origin.x + sv.size.x, sv.origin.y + sv.size.y),
        );
        assert!((s - 1.0).abs() < 0.001 && v.abs() < 0.001);
    }

    #[test]
    fn paint_fills_card_and_sv_cells() {
        let t = Tokens::dark();
        let mut p = CapturePainter::default();
        picker(180.0, 0.5, 0.5).paint(
            &mut p,
            Rect::xywh(0.0, 0.0, PICKER_WIDTH, PICKER_HEIGHT),
            &t,
        );
        // Card background.
        assert!(p
            .ops
            .iter()
            .any(|op| matches!(op, PaintOp::FillRoundRect(_, _, c) if *c == t.card)));
        // The SV grid paints many plain fill_rect cells (18*14 = 252).
        let cells = p
            .ops
            .iter()
            .filter(|op| matches!(op, PaintOp::FillRect(..)))
            .count();
        assert!(
            cells >= 18 * 14,
            "expected the SV gradient grid, got {cells} fill_rects"
        );
    }
}
