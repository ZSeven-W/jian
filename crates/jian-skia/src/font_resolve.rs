//! Shared Skia typeface resolution for measuring and painting text.
//!
//! Hosts that draw text directly with `Canvas::draw_str` and layout
//! code that measures through `SkiaMeasure` must choose the same
//! typeface for the same family / weight / character tuple. This
//! resolver owns that policy and caches the per-character result.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

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
///
/// The bundled + imported managers are rebuilt lazily whenever the
/// process-global font registry advances its generation, so a runtime
/// font import takes effect on the next resolve without reconstructing
/// the resolver.
pub struct FontResolver {
    system_mgr: FontMgr,
    bundled_mgr: RefCell<Option<FontMgr>>,
    imported_mgr: RefCell<Option<FontMgr>>,
    default_typeface: Option<Typeface>,
    cache: RefCell<FontResolverCache>,
    built_generation: Cell<u64>,
}

impl FontResolver {
    pub fn new(font_mgr: FontMgr) -> Self {
        let default_typeface = font_mgr.legacy_make_typeface(None, FontStyle::default());
        Self::with_default_typeface(font_mgr, default_typeface)
    }

    pub fn with_default_typeface(font_mgr: FontMgr, default_typeface: Option<Typeface>) -> Self {
        // Read the generation BEFORE building the providers. If an import
        // lands between here and the build, the providers are already
        // fresh and `built_generation` is merely stale-low, so the next
        // `refresh_if_stale` rebuilds (harmlessly redundant). Reading it
        // AFTER the build would record a new generation against old
        // providers — a permanent miss until the next mutation.
        let built_generation = crate::bundled_fonts::generation();
        let bundled_mgr = crate::bundled_fonts::bundled_provider().map(Into::into);
        let imported_mgr = crate::bundled_fonts::imported_provider().map(Into::into);
        Self {
            system_mgr: font_mgr,
            bundled_mgr: RefCell::new(bundled_mgr),
            imported_mgr: RefCell::new(imported_mgr),
            default_typeface,
            cache: RefCell::new(FontResolverCache::default()),
            built_generation: Cell::new(built_generation),
        }
    }

    /// Rebuild the bundled + imported managers and drop the per-char
    /// cache when the registry generation has advanced since we last
    /// built. Cheap (one atomic load) on the common no-change path.
    fn refresh_if_stale(&self) {
        let current = crate::bundled_fonts::generation();
        if current == self.built_generation.get() {
            return;
        }
        *self.bundled_mgr.borrow_mut() = crate::bundled_fonts::bundled_provider().map(Into::into);
        *self.imported_mgr.borrow_mut() = crate::bundled_fonts::imported_provider().map(Into::into);
        self.cache.borrow_mut().chars.clear();
        self.built_generation.set(current);
    }

    /// Build an ordered manager for Skia Paragraph shaping. Imported
    /// fonts come first (a deliberate override), then system, then the
    /// app's bundled fonts, then the Roboto default. Direct resolver
    /// calls still make explicit source decisions so they can report the
    /// synthetic-bold branch.
    pub fn ordered_font_manager(&self) -> FontMgr {
        self.refresh_if_stale();
        let mut ordered = OrderedFontMgr::new();
        if let Some(imported) = &*self.imported_mgr.borrow() {
            ordered.append(imported.clone());
        }
        ordered.append(self.system_mgr.clone());
        if let Some(bundled) = &*self.bundled_mgr.borrow() {
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
        self.refresh_if_stale();
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
            // Imported fonts win over a same-named system font — the user
            // imported this deliberately.
            if let Some(imported) = &*self.imported_mgr.borrow() {
                if let Some(typeface) = family_typeface_covering(imported, family, style, c) {
                    return Some(resolved(typeface, weight, italic));
                }
            }
            if let Some(typeface) = family_typeface_covering(&self.system_mgr, family, style, c) {
                return Some(resolved(typeface, weight, italic));
            }
            if let Some(bundled) = &*self.bundled_mgr.borrow() {
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
