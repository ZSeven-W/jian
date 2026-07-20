//! Paragraph-backed first-line baseline measurement for widget text.
//!
//! Canvas text is painted with a baseline origin, while document layout stores
//! the top of the CSS/Pencil line box. A raw `Font::metrics()` ascent cannot
//! bridge those coordinate systems when authored line-height or fallback faces
//! are involved. This helper shapes the actual first line through the same
//! Skia Paragraph style used by `SkiaMeasure` and caches the resulting
//! alphabetic baseline for frame-to-frame paint.

use std::{collections::HashMap, rc::Rc};

use skia_safe::{
    font_style::{Slant, Weight, Width},
    textlayout::{FontCollection, ParagraphBuilder, ParagraphStyle, TextStyle},
    FontStyle,
};

use crate::{measure::build_collection, FontResolver};

const NATURAL_LAYOUT_BUDGET: f32 = 1.0e6;
const BASELINE_CACHE_CAP: usize = 4096;

#[derive(Clone, Hash, PartialEq, Eq)]
struct BaselineKey {
    text: String,
    family: String,
    font_size_bits: u32,
    weight: u16,
    italic: bool,
    line_height_bits: u32,
}

/// Cached Paragraph state used by a paint backend that already owns a
/// [`FontResolver`]. Keeping the collection separate avoids rebuilding font
/// providers for every text node while still sharing the resolver's source
/// ordering and runtime-import generation.
pub struct ParagraphBaseline {
    font_collection: Rc<FontCollection>,
    built_generation: u64,
    cache: HashMap<BaselineKey, f32>,
}

impl ParagraphBaseline {
    pub fn new(font_resolver: &FontResolver) -> Self {
        crate::font_lock::with_font_lock(|| {
            // Read before building: a concurrent registration can at worst
            // leave this value stale-low, forcing one harmless rebuild.
            let built_generation = crate::bundled_fonts::generation();
            Self {
                font_collection: Rc::new(build_collection(font_resolver)),
                built_generation,
                cache: HashMap::new(),
            }
        })
    }

    /// Return the distance from the line-box top to its alphabetic baseline.
    /// `line_height` is a multiplier; values `<= 0` retain Paragraph defaults.
    #[allow(clippy::too_many_arguments)]
    pub fn first_line_baseline(
        &mut self,
        font_resolver: &FontResolver,
        text: &str,
        family: &str,
        font_size: f32,
        weight: u16,
        italic: bool,
        line_height: f32,
    ) -> Option<f32> {
        if !font_size.is_finite() || font_size <= 0.0 || !line_height.is_finite() {
            return None;
        }
        crate::font_lock::with_font_lock(|| {
            self.first_line_baseline_locked(
                font_resolver,
                text,
                family,
                font_size,
                weight,
                italic,
                line_height,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn first_line_baseline_locked(
        &mut self,
        font_resolver: &FontResolver,
        text: &str,
        family: &str,
        font_size: f32,
        weight: u16,
        italic: bool,
        line_height: f32,
    ) -> Option<f32> {
        self.refresh_if_stale(font_resolver);
        let line = first_logical_line(text);
        let key = BaselineKey {
            text: line.to_string(),
            family: family.to_string(),
            font_size_bits: font_size.to_bits(),
            weight,
            italic,
            line_height_bits: line_height.to_bits(),
        };
        if let Some(value) = self.cache.get(&key) {
            return Some(*value);
        }

        let style = ParagraphStyle::new();
        let mut builder = ParagraphBuilder::new(&style, (*self.font_collection).clone());
        let mut text_style = TextStyle::new();
        text_style.set_font_size(font_size);
        let families = font_resolver.font_families_for_shaping(Some(family));
        let family_refs = families.iter().map(String::as_str).collect::<Vec<_>>();
        if !family_refs.is_empty() {
            text_style.set_font_families(&family_refs);
        }
        text_style.set_font_style(FontStyle::new(
            Weight::from(weight as i32),
            Width::NORMAL,
            if italic {
                Slant::Italic
            } else {
                Slant::Upright
            },
        ));
        if line_height > 0.0 {
            text_style.set_height(line_height);
            text_style.set_height_override(true);
            text_style.set_half_leading(true);
        }
        builder.push_style(&text_style);
        // An empty authored line still has a font strut and a baseline. A
        // zero-width space obtains those metrics without painting visible ink.
        builder.add_text(if line.is_empty() { "\u{200b}" } else { line });
        builder.pop();

        let mut paragraph = builder.build();
        paragraph.layout(NATURAL_LAYOUT_BUDGET);
        let baseline = paragraph.alphabetic_baseline();
        if !baseline.is_finite() || baseline <= 0.0 {
            return None;
        }
        if self.cache.len() >= BASELINE_CACHE_CAP {
            self.cache.clear();
        }
        self.cache.insert(key, baseline);
        Some(baseline)
    }

    fn refresh_if_stale(&mut self, font_resolver: &FontResolver) {
        let generation = crate::bundled_fonts::generation();
        if generation == self.built_generation {
            return;
        }
        self.font_collection = Rc::new(build_collection(font_resolver));
        self.built_generation = generation;
        self.cache.clear();
    }

    #[cfg(test)]
    fn cache_len(&self) -> usize {
        self.cache.len()
    }
}

fn first_logical_line(text: &str) -> &str {
    let end = text.find(['\n', '\r']).unwrap_or(text.len());
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use skia_safe::FontMgr;

    #[test]
    fn selects_the_actual_first_line_including_emoji() {
        assert_eq!(first_logical_line("🧥 new\nM later"), "🧥 new");
        assert_eq!(first_logical_line("\r\nsecond"), "");
    }

    #[test]
    fn authored_line_height_moves_the_paragraph_baseline() {
        let resolver = FontResolver::new(FontMgr::default());
        let mut metrics = ParagraphBaseline::new(&resolver);
        let compact = metrics
            .first_line_baseline(&resolver, "Navigation", "sans-serif", 16.0, 400, false, 1.0)
            .unwrap();
        let loose = metrics
            .first_line_baseline(&resolver, "Navigation", "sans-serif", 16.0, 400, false, 1.5)
            .unwrap();
        // With Paragraph half-leading, the extra 0.5em is split around the
        // font metrics, so the first baseline advances by one quarter em.
        assert!((loose - compact - 4.0).abs() < 0.05, "{compact} -> {loose}");
    }

    #[test]
    fn cache_key_keeps_actual_fallback_text_distinct() {
        let resolver = FontResolver::new(FontMgr::default());
        let mut metrics = ParagraphBaseline::new(&resolver);
        for text in ["M", "🧥"] {
            assert!(metrics
                .first_line_baseline(&resolver, text, "sans-serif", 24.0, 400, false, 1.5)
                .is_some());
        }
        assert_eq!(metrics.cache_len(), 2);
    }
}
