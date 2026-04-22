//! Text measurement — a placeholder that estimates width using a fixed-width
//! heuristic. The real RenderBackend is expected to provide accurate shaping
//! in future plans; this keeps layout testable without a font engine.

pub fn estimate_text_size(content: &str, font_size: f32) -> (f32, f32) {
    estimate_text_size_weighted(content, font_size, 400)
}

/// Character-count heuristic that varies the per-glyph ratio with CSS
/// `font_weight`: bolder faces widen glyphs so a single ratio misfires
/// when a single layout has a mix of regular body text (400) and big
/// bold titles (700). Values tuned empirically against Inter / Space
/// Grotesk measurements — good to within ~10% for common Latin text.
pub fn estimate_text_size_weighted(content: &str, font_size: f32, weight: u16) -> (f32, f32) {
    let lines: Vec<&str> = content.split('\n').collect();
    let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    // Regular weight ~ 0.55; semibold 600 ~ 0.60; bold 700+ ~ 0.64.
    let ratio = if weight >= 700 {
        0.64
    } else if weight >= 600 {
        0.60
    } else {
        0.58
    };
    let width = widest as f32 * font_size * ratio;
    let height = lines.len() as f32 * font_size * 1.3;
    (width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line() {
        let (w, h) = estimate_text_size("Hello", 16.0);
        assert!(w > 0.0 && h > 0.0);
    }

    #[test]
    fn multi_line() {
        let (w_one, h_one) = estimate_text_size("Hi", 16.0);
        let (w_two, h_two) = estimate_text_size("Hi\nThere", 16.0);
        assert!(w_two >= w_one);
        assert!(h_two > h_one);
    }
}
