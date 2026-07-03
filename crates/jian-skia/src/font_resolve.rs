//! Shared Skia typeface resolution for measuring and painting text.
//!
//! Hosts that draw text directly with `Canvas::draw_str` and layout
//! code that measures through `SkiaMeasure` must choose the same
//! typeface for the same family / weight / character tuple. This
//! resolver owns that policy and caches the per-character result.

use std::{cell::RefCell, collections::HashMap};

use skia_safe::{
    font_style::{Slant, Weight, Width},
    utils::OrderedFontMgr,
    Font, FontMgr, FontStyle, Typeface,
};

/// Native paint uses a stroke fill to synthesize bold when the
/// resolved face is not actually bold. The measurement path applies
/// the same small advance compensation so geometry sees the painted
/// visual width, not just the fill glyph advance.
pub const SYNTHETIC_BOLD_WIDTH_FACTOR: f32 = 1.03;

/// Synthetic oblique skew applied when an italic request resolves to
/// an upright face. Skewing keeps the same advance width.
pub const SYNTHETIC_ITALIC_SKEW: f32 = -0.25;

const BOLD_WEIGHT_MIN: u16 = 600;

/// One text segment whose chars all use the same typeface and
/// synthetic-style branch.
#[derive(Clone)]
pub struct FontSegment {
    pub typeface: Typeface,
    pub text: String,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
}

/// A resolved typeface plus the style branch both measure and paint
/// must agree on.
#[derive(Clone)]
pub struct ResolvedTypeface {
    pub typeface: Typeface,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
}

#[derive(Default)]
struct FontResolverCache {
    chars: HashMap<FontKey, Option<ResolvedTypeface>>,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct FontKey {
    family: Option<String>,
    codepoint: i32,
    weight: u16,
    italic: bool,
}

/// Shared typeface resolver for Skia-backed text paths.
pub struct FontResolver {
    system_mgr: FontMgr,
    bundled_mgr: Option<FontMgr>,
    default_typeface: Option<Typeface>,
    cache: RefCell<FontResolverCache>,
}

impl FontResolver {
    pub fn new(font_mgr: FontMgr) -> Self {
        let default_typeface = font_mgr.legacy_make_typeface(None, FontStyle::default());
        Self::with_default_typeface(font_mgr, default_typeface)
    }

    pub fn with_default_typeface(font_mgr: FontMgr, default_typeface: Option<Typeface>) -> Self {
        let bundled_mgr = crate::bundled_fonts::bundled_provider().map(Into::into);
        Self {
            system_mgr: font_mgr,
            bundled_mgr,
            default_typeface,
            cache: RefCell::new(FontResolverCache::default()),
        }
    }

    /// Build an ordered manager for Skia Paragraph shaping. Direct
    /// resolver calls still make explicit source decisions so they
    /// can report the synthetic-bold branch.
    pub fn ordered_font_manager(&self) -> FontMgr {
        let mut ordered = OrderedFontMgr::new();
        ordered.append(self.system_mgr.clone());
        if let Some(bundled) = &self.bundled_mgr {
            ordered.append(bundled.clone());
        }
        let mut default_provider = skia_safe::textlayout::TypefaceFontProvider::new();
        if let Some(default_typeface) = &self.default_typeface {
            default_provider.register_typeface(default_typeface.clone(), Some("Roboto"));
            ordered.append(default_provider);
        }
        ordered.into()
    }

    pub fn typeface_for_char(
        &self,
        family: Option<&str>,
        c: char,
        weight: u16,
        italic: bool,
    ) -> Option<ResolvedTypeface> {
        let family = primary_font_family(family);
        let key = FontKey {
            family: family.clone(),
            codepoint: c as i32,
            weight,
            italic,
        };
        if let Some(cached) = self.cache.borrow().chars.get(&key) {
            return cached.clone();
        }
        let resolved = self.resolve_uncached(family.as_deref(), c, weight, italic);
        self.cache.borrow_mut().chars.insert(key, resolved.clone());
        resolved
    }

    pub fn segment_text(
        &self,
        text: &str,
        family: Option<&str>,
        weight: u16,
        italic: bool,
    ) -> Vec<FontSegment> {
        let mut segments: Vec<FontSegment> = Vec::new();
        for c in text.chars() {
            let Some(resolved) = self.typeface_for_char(family, c, weight, italic) else {
                if let Some(last) = segments.last_mut() {
                    last.text.push(c);
                }
                continue;
            };
            match segments.last_mut() {
                Some(last)
                    if last.typeface.unique_id() == resolved.typeface.unique_id()
                        && last.synthetic_bold == resolved.synthetic_bold
                        && last.synthetic_italic == resolved.synthetic_italic =>
                {
                    last.text.push(c);
                }
                _ => segments.push(FontSegment {
                    typeface: resolved.typeface,
                    text: c.to_string(),
                    synthetic_bold: resolved.synthetic_bold,
                    synthetic_italic: resolved.synthetic_italic,
                }),
            }
        }
        segments
    }

    pub fn measure_text(
        &self,
        text: &str,
        font_size: f32,
        family: Option<&str>,
        weight: u16,
        italic: bool,
    ) -> f32 {
        let mut width = 0.0_f32;
        for segment in self.segment_text(text, family, weight, italic) {
            let mut font = Font::new(&segment.typeface, font_size);
            if segment.synthetic_italic {
                font.set_skew_x(SYNTHETIC_ITALIC_SKEW);
            }
            let (mut advance, _) = font.measure_str(&segment.text, None);
            if segment.synthetic_bold {
                advance *= SYNTHETIC_BOLD_WIDTH_FACTOR;
            }
            width += advance;
        }
        width
    }

    pub fn cache_len(&self) -> usize {
        self.cache.borrow().chars.len()
    }

    fn resolve_uncached(
        &self,
        family: Option<&str>,
        c: char,
        weight: u16,
        italic: bool,
    ) -> Option<ResolvedTypeface> {
        let style = font_style_for(weight, italic);
        if let Some(family) = family {
            if let Some(typeface) = family_typeface_covering(&self.system_mgr, family, style, c) {
                return Some(resolved(typeface, weight, italic));
            }
            if let Some(bundled) = &self.bundled_mgr {
                if let Some(typeface) = family_typeface_covering(bundled, family, style, c) {
                    return Some(resolved(typeface, weight, italic));
                }
            }
        }
        if let Some(typeface) = self
            .default_typeface
            .as_ref()
            .filter(|typeface| covers(typeface, c))
            .cloned()
        {
            return Some(resolved(typeface, weight, italic));
        }
        self.system_mgr
            .match_family_style_character("", style, &[], c as i32)
            .map(|typeface| resolved(typeface, weight, italic))
    }
}

pub fn font_style_for(weight: u16, italic: bool) -> FontStyle {
    FontStyle::new(
        Weight::from(weight as i32),
        Width::NORMAL,
        if italic {
            Slant::Italic
        } else {
            Slant::Upright
        },
    )
}

fn family_typeface_covering(
    mgr: &FontMgr,
    family: &str,
    style: FontStyle,
    c: char,
) -> Option<Typeface> {
    if mgr.match_family(family).count() == 0 {
        return None;
    }
    mgr.match_family_style(family, style)
        .filter(|typeface| covers(typeface, c))
}

fn resolved(typeface: Typeface, weight: u16, italic: bool) -> ResolvedTypeface {
    let synthetic_bold = weight >= BOLD_WEIGHT_MIN && !typeface.is_bold();
    let synthetic_italic = italic && !typeface.is_italic();
    ResolvedTypeface {
        typeface,
        synthetic_bold,
        synthetic_italic,
    }
}

fn covers(typeface: &Typeface, c: char) -> bool {
    typeface.unichar_to_glyph(c as i32) != 0
}

fn primary_font_family(stack: Option<&str>) -> Option<String> {
    let first = stack?.split(',').next()?.trim().trim_matches(['"', '\'']);
    if first.is_empty()
        || matches!(
            first,
            "system-ui" | "sans-serif" | "serif" | "monospace" | "-apple-system"
        )
    {
        None
    } else {
        Some(first.to_string())
    }
}
