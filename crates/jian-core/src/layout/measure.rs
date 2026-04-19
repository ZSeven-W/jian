//! Text measurement — a placeholder that estimates width using a fixed-width
//! heuristic. The real RenderBackend is expected to provide accurate shaping
//! in future plans; this keeps layout testable without a font engine.

pub fn estimate_text_size(content: &str, font_size: f32) -> (f32, f32) {
    let lines: Vec<&str> = content.split('\n').collect();
    let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let width = widest as f32 * font_size * 0.55;
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
