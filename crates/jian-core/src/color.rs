//! Canonical HSV / RGB / hex colour conversions, tuple-based so the
//! low-level layer carries no `Color` type dependency. jian-widgets wraps
//! `hsv_to_rgb` / `rgb_to_hsv` to its `Color` struct for painting, and
//! document layers (e.g. OpenPencil) reuse the tuple + hex helpers for
//! writing colours into their model — one copy of the maths for everyone.

/// HSV → RGB. `h` is 0..360, `s` / `v` are 0..1; each output channel 0..1.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
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
    (r1 + m, g1 + m, b1 + m)
}

/// RGB (each 0..1) → HSV (`h` 0..360, `s` / `v` 0..1).
pub fn rgb_to_hsv(rgb: (f32, f32, f32)) -> (f32, f32, f32) {
    let (r, g, b) = rgb;
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
    let h = if h < 0.0 { h + 360.0 } else { h };
    (h, s, v)
}

/// Parse `#rgb` / `#rrggbb` / `#rrggbbaa` into RGB floats (0..1).
/// Lenient on case; requires the leading `#`.
pub fn parse_hex_rgb(s: &str) -> Option<(f32, f32, f32)> {
    let s = s.trim().strip_prefix('#')?;
    let (r, g, b) = match s.len() {
        3 => (
            u8::from_str_radix(&s[0..1].repeat(2), 16).ok()?,
            u8::from_str_radix(&s[1..2].repeat(2), 16).ok()?,
            u8::from_str_radix(&s[2..3].repeat(2), 16).ok()?,
        ),
        6 | 8 => (
            u8::from_str_radix(&s[0..2], 16).ok()?,
            u8::from_str_radix(&s[2..4], 16).ok()?,
            u8::from_str_radix(&s[4..6], 16).ok()?,
        ),
        _ => return None,
    };
    Some((r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0))
}

/// Parse the alpha channel out of `#rrggbbaa` — defaults to `1.0` when the
/// hex is 6-char (no alpha authored) or unparseable.
pub fn parse_hex_alpha(s: &str) -> f32 {
    let Some(stripped) = s.trim().strip_prefix('#') else {
        return 1.0;
    };
    if stripped.len() != 8 {
        return 1.0;
    }
    u8::from_str_radix(&stripped[6..8], 16)
        .map(|a| a as f32 / 255.0)
        .unwrap_or(1.0)
}

/// Format RGB floats (0..1) as a `#rrggbb` hex string.
pub fn rgb_to_hex(r: f32, g: f32, b: f32) -> String {
    fn ch(v: f32) -> u8 {
        (v.clamp(0.0, 1.0) * 255.0).round() as u8
    }
    format!("#{:02x}{:02x}{:02x}", ch(r), ch(g), ch(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_rgb_roundtrip_is_stable() {
        for &(h, s, v) in &[(0.0, 1.0, 1.0), (210.0, 0.6, 0.4), (45.0, 0.2, 0.9)] {
            let (h2, s2, v2) = rgb_to_hsv(hsv_to_rgb(h, s, v));
            assert!((h - h2).abs() < 0.5, "h {h} -> {h2}");
            assert!((s - s2).abs() < 0.01, "s {s} -> {s2}");
            assert!((v - v2).abs() < 0.01, "v {v} -> {v2}");
        }
    }

    #[test]
    fn hex_parse_and_format() {
        assert_eq!(parse_hex_rgb("#fff"), parse_hex_rgb("#ffffff"));
        let (r, g, b) = parse_hex_rgb("#2d5e38").unwrap();
        assert_eq!(rgb_to_hex(r, g, b), "#2d5e38");
        assert_eq!(parse_hex_alpha("#11223380"), 128.0 / 255.0);
        assert_eq!(parse_hex_alpha("#112233"), 1.0);
        assert!(parse_hex_rgb("nope").is_none());
    }
}
