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
    families: Vec<String>,
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
    system_family_names: HashMap<String, String>,
    bundled_mgr: RefCell<Option<FontMgr>>,
    bundled_family_names: RefCell<HashMap<String, String>>,
    imported_mgr: RefCell<Option<FontMgr>>,
    imported_family_names: RefCell<HashMap<String, String>>,
    default_typeface: Option<Typeface>,
    cache: RefCell<FontResolverCache>,
    built_generation: Cell<u64>,
}

impl FontResolver {
    pub fn new(font_mgr: FontMgr) -> Self {
        // `legacy_make_typeface` hits DirectWrite on Windows; serialize it
        // (reentrant — `with_default_typeface` re-locks harmlessly).
        crate::font_lock::with_font_lock(|| {
            let default_typeface = font_mgr.legacy_make_typeface(None, FontStyle::default());
            Self::with_default_typeface(font_mgr, default_typeface)
        })
    }

    pub fn with_default_typeface(font_mgr: FontMgr, default_typeface: Option<Typeface>) -> Self {
        // Serialize provider construction with all other DirectWrite work
        // (reentrant under a locked measure/build path).
        crate::font_lock::with_font_lock(|| {
            // Read the generation BEFORE building the providers. If an import
            // lands between here and the build, the providers are already
            // fresh and `built_generation` is merely stale-low, so the next
            // `refresh_if_stale` rebuilds (harmlessly redundant). Reading it
            // AFTER the build would record a new generation against old
            // providers — a permanent miss until the next mutation.
            let built_generation = crate::bundled_fonts::generation();
            let bundled_mgr = crate::bundled_fonts::bundled_provider().map(Into::into);
            let imported_mgr = crate::bundled_fonts::imported_provider().map(Into::into);
            let system_family_names = font_family_name_index(&font_mgr);
            let bundled_family_names = bundled_mgr
                .as_ref()
                .map(font_family_name_index)
                .unwrap_or_default();
            let imported_family_names = imported_mgr
                .as_ref()
                .map(font_family_name_index)
                .unwrap_or_default();
            Self {
                system_mgr: font_mgr,
                system_family_names,
                bundled_mgr: RefCell::new(bundled_mgr),
                bundled_family_names: RefCell::new(bundled_family_names),
                imported_mgr: RefCell::new(imported_mgr),
                imported_family_names: RefCell::new(imported_family_names),
                default_typeface,
                cache: RefCell::new(FontResolverCache::default()),
                built_generation: Cell::new(built_generation),
            }
        })
    }

    /// Rebuild the bundled + imported managers and drop the per-char
    /// cache when the registry generation has advanced since we last
    /// built. Cheap (one atomic load) on the common no-change path.
    fn refresh_if_stale(&self) {
        let current = crate::bundled_fonts::generation();
        if current == self.built_generation.get() {
            return;
        }
        let (bundled_mgr, bundled_family_names, imported_mgr, imported_family_names) =
            crate::font_lock::with_font_lock(|| {
                let bundled_mgr = crate::bundled_fonts::bundled_provider().map(Into::into);
                let bundled_family_names = bundled_mgr
                    .as_ref()
                    .map(font_family_name_index)
                    .unwrap_or_default();
                let imported_mgr = crate::bundled_fonts::imported_provider().map(Into::into);
                let imported_family_names = imported_mgr
                    .as_ref()
                    .map(font_family_name_index)
                    .unwrap_or_default();
                (
                    bundled_mgr,
                    bundled_family_names,
                    imported_mgr,
                    imported_family_names,
                )
            });
        *self.bundled_mgr.borrow_mut() = bundled_mgr;
        *self.bundled_family_names.borrow_mut() = bundled_family_names;
        *self.imported_mgr.borrow_mut() = imported_mgr;
        *self.imported_family_names.borrow_mut() = imported_family_names;
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

    /// Resolve a CSS family stack for Skia Paragraph shaping. Authored family
    /// order is preserved; within each candidate an explicitly imported face
    /// wins over a same-named system or app-bundled face.
    pub fn font_families_for_shaping(&self, stack: Option<&str>) -> Vec<String> {
        self.refresh_if_stale();
        let candidates = font_family_candidates(stack);
        crate::font_lock::with_font_lock(|| {
            let imported_mgr = self.imported_mgr.borrow();
            let imported_names = self.imported_family_names.borrow();
            let bundled_mgr = self.bundled_mgr.borrow();
            let bundled_names = self.bundled_family_names.borrow();
            canonicalize_font_family_candidates(candidates, |family| {
                imported_mgr
                    .as_ref()
                    .and_then(|manager| canonical_font_family(manager, &imported_names, family))
                    .or_else(|| {
                        canonical_font_family(&self.system_mgr, &self.system_family_names, family)
                    })
                    .or_else(|| {
                        bundled_mgr.as_ref().and_then(|manager| {
                            canonical_font_family(manager, &bundled_names, family)
                        })
                    })
            })
        })
    }

    pub fn typeface_for_char(
        &self,
        family: Option<&str>,
        c: char,
        weight: u16,
        italic: bool,
    ) -> Option<ResolvedTypeface> {
        self.refresh_if_stale();
        let families = font_family_candidates(family);
        self.typeface_for_char_in_families(&families, c, weight, italic)
    }

    fn typeface_for_char_in_families(
        &self,
        families: &[String],
        c: char,
        weight: u16,
        italic: bool,
    ) -> Option<ResolvedTypeface> {
        let key = FontKey {
            families: families
                .iter()
                .map(|family| family.to_ascii_lowercase())
                .collect(),
            codepoint: c as i32,
            weight,
            italic,
        };
        if let Some(cached) = self.cache.borrow().chars.get(&key) {
            return cached.clone();
        }
        // Cache miss → `match_family_style_character` (DirectWrite system font
        // scan on Windows). Serialize just this call so the per-char cached
        // fast path above stays lock-free.
        let resolved =
            crate::font_lock::with_font_lock(|| self.resolve_uncached(families, c, weight, italic));
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
        self.refresh_if_stale();
        let families = font_family_candidates(family);
        let mut segments: Vec<FontSegment> = Vec::new();
        for c in text.chars() {
            let Some(resolved) = self.typeface_for_char_in_families(&families, c, weight, italic)
            else {
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
        // Serialize segmentation (resolves system typefaces via DirectWrite)
        // + glyph-advance measurement with all other font work. Reentrant, so
        // this is a no-op re-lock when called from within `SkiaMeasure`.
        crate::font_lock::with_font_lock(|| {
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
        })
    }

    pub fn cache_len(&self) -> usize {
        self.cache.borrow().chars.len()
    }

    fn resolve_uncached(
        &self,
        families: &[String],
        c: char,
        weight: u16,
        italic: bool,
    ) -> Option<ResolvedTypeface> {
        let style = font_style_for(weight, italic);
        let imported_mgr = self.imported_mgr.borrow();
        let imported_names = self.imported_family_names.borrow();
        let bundled_mgr = self.bundled_mgr.borrow();
        let bundled_names = self.bundled_family_names.borrow();
        for family in families {
            // Preserve CSS family order. Within one authored candidate, an
            // imported face wins over the same-named system or bundled face.
            if let Some(imported) = &*imported_mgr {
                if let Some(typeface) =
                    family_typeface_covering(imported, &imported_names, family, style, c)
                {
                    return Some(resolved(typeface, weight, italic));
                }
            }
            if let Some(typeface) = family_typeface_covering(
                &self.system_mgr,
                &self.system_family_names,
                family,
                style,
                c,
            ) {
                return Some(resolved(typeface, weight, italic));
            }
            if let Some(bundled) = &*bundled_mgr {
                if let Some(typeface) =
                    family_typeface_covering(bundled, &bundled_names, family, style, c)
                {
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
    family_names: &HashMap<String, String>,
    family: &str,
    style: FontStyle,
    c: char,
) -> Option<Typeface> {
    let family = canonical_font_family(mgr, family_names, family)?;
    mgr.match_family_style(&family, style)
        .filter(|typeface| covers(typeface, c))
}

/// Index the canonical spelling exposed by a `FontMgr` under an ASCII-folded
/// key. CSS family names are ASCII case-insensitive, while Skia's custom
/// `TypefaceFontProvider` lookup is case-sensitive on some platforms.
fn font_family_name_index(mgr: &FontMgr) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for index in 0..mgr.count_families() {
        let family = mgr.family_name(index);
        if !family.trim().is_empty() {
            names.entry(family.to_ascii_lowercase()).or_insert(family);
        }
    }
    names
}

fn canonical_font_family(
    mgr: &FontMgr,
    family_names: &HashMap<String, String>,
    family: &str,
) -> Option<String> {
    if let Some(canonical) = family_names.get(&family.to_ascii_lowercase()) {
        return Some(canonical.clone());
    }
    let mut styles = mgr.match_family(family);
    if styles.count() == 0 {
        return None;
    }
    mgr.match_family_style(family, FontStyle::default())
        .map(|typeface| typeface.family_name())
        .or_else(|| Some(family.to_string()))
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

/// Parse a CSS font-family stack into the concrete family names Skia should
/// try, preserving authored order. Quoted commas remain part of a family name,
/// and CSS generic families expand to a native family Skia can resolve.
pub fn font_family_candidates(stack: Option<&str>) -> Vec<String> {
    let Some(stack) = stack else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for family in split_font_family_stack(stack) {
        let Some(family) = concrete_font_family(&family) else {
            continue;
        };
        if !candidates
            .iter()
            .any(|candidate: &String| candidate.eq_ignore_ascii_case(family))
        {
            candidates.push(family.to_string());
        }
    }
    candidates
}

fn canonicalize_font_family_candidates(
    candidates: Vec<String>,
    mut canonicalize: impl FnMut(&str) -> Option<String>,
) -> Vec<String> {
    let mut resolved = Vec::with_capacity(candidates.len());
    for family in candidates {
        let canonical = canonicalize(&family).unwrap_or(family);
        push_unique_font_family(&mut resolved, canonical);
    }
    resolved
}

fn push_unique_font_family(families: &mut Vec<String>, family: String) {
    if !families
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(&family))
    {
        families.push(family);
    }
}

fn split_font_family_stack(stack: &str) -> Vec<String> {
    let mut families = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for ch in stack.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            ',' => push_font_family(&mut families, &mut current),
            _ => current.push(ch),
        }
    }
    if escaped {
        current.push('\\');
    }
    push_font_family(&mut families, &mut current);
    families
}

fn push_font_family(families: &mut Vec<String>, current: &mut String) {
    let family = current.trim();
    if !family.is_empty() {
        families.push(family.to_string());
    }
    current.clear();
}

fn concrete_font_family(family: &str) -> Option<&str> {
    let generic = family.to_ascii_lowercase();
    match generic.as_str() {
        "system-ui" | "ui-sans-serif" => Some(platform_system_ui_family()),
        "-apple-system" | "blinkmacsystemfont" => platform_apple_system_family(),
        "sans-serif" => Some(platform_sans_serif_family()),
        "serif" | "ui-serif" => Some(platform_serif_family()),
        "monospace" | "ui-monospace" => Some(platform_monospace_family()),
        "ui-rounded" => Some(platform_system_ui_family()),
        _ => Some(family),
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn platform_system_ui_family() -> &'static str {
    ".AppleSystemUIFont"
}

#[cfg(target_os = "windows")]
fn platform_system_ui_family() -> &'static str {
    "Segoe UI"
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
fn platform_system_ui_family() -> &'static str {
    "sans-serif"
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn platform_apple_system_family() -> Option<&'static str> {
    Some(".AppleSystemUIFont")
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn platform_apple_system_family() -> Option<&'static str> {
    None
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "windows"))]
fn platform_sans_serif_family() -> &'static str {
    "Arial"
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
fn platform_sans_serif_family() -> &'static str {
    "sans-serif"
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn platform_serif_family() -> &'static str {
    "Times"
}

#[cfg(target_os = "windows")]
fn platform_serif_family() -> &'static str {
    "Times New Roman"
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
fn platform_serif_family() -> &'static str {
    "serif"
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn platform_monospace_family() -> &'static str {
    "Menlo"
}

#[cfg(target_os = "windows")]
fn platform_monospace_family() -> &'static str {
    "Consolas"
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
fn platform_monospace_family() -> &'static str {
    "monospace"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_names_and_commas_without_losing_order() {
        assert_eq!(
            split_font_family_stack(r#""ACME, Display", 'DM Sans', Inter"#),
            vec!["ACME, Display", "DM Sans", "Inter"]
        );
    }

    #[test]
    fn expands_generics_and_deduplicates_aliases() {
        assert_eq!(
            font_family_candidates(Some(
                "Missing, ui-sans-serif, system-ui, sans-serif, serif, monospace"
            )),
            vec![
                "Missing",
                platform_system_ui_family(),
                platform_sans_serif_family(),
                platform_serif_family(),
                platform_monospace_family(),
            ]
        );
    }

    #[test]
    fn shaping_preserves_authored_order_and_canonicalizes_each_family() {
        let candidates = vec!["inter".to_string(), ".applesystemuifont".to_string()];
        assert_eq!(
            canonicalize_font_family_candidates(candidates, |family| match family {
                "inter" => Some("Inter".to_string()),
                ".applesystemuifont" => Some(".AppleSystemUIFont".to_string()),
                _ => None,
            }),
            vec!["Inter", ".AppleSystemUIFont"]
        );
        assert_eq!(
            canonicalize_font_family_candidates(
                vec!["Missing".to_string(), "inter".to_string()],
                |family| (family == "inter").then(|| "Inter".to_string()),
            ),
            vec!["Missing", "Inter"]
        );
    }

    #[test]
    fn custom_provider_family_lookup_ignores_ascii_case() {
        crate::font_lock::with_font_lock(|| {
            const FAMILY: &str = "Case Fold Test";
            let default_mgr = FontMgr::default();
            let typeface = default_mgr
                .legacy_make_typeface(None, FontStyle::default())
                .expect("platform should expose a default typeface");
            let mut provider = skia_safe::textlayout::TypefaceFontProvider::new();
            provider.register_typeface(typeface, Some(FAMILY));
            let manager: FontMgr = provider.into();
            let names = font_family_name_index(&manager);

            assert_eq!(
                canonical_font_family(&manager, &names, "case fold test").as_deref(),
                Some(FAMILY)
            );
            assert_eq!(
                canonicalize_font_family_candidates(
                    font_family_candidates(Some("case fold test")),
                    |family| canonical_font_family(&manager, &names, family),
                ),
                vec![FAMILY]
            );
            assert!(family_typeface_covering(
                &manager,
                &names,
                "case fold test",
                FontStyle::default(),
                'A',
            )
            .is_some());
        });
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_ui_resolves_to_sf_system_face() {
        let resolver = FontResolver::new(FontMgr::default());
        assert_eq!(
            resolver.font_families_for_shaping(Some("Inter, system-ui")),
            vec!["Inter", ".AppleSystemUIFont"]
        );
        assert_eq!(
            resolver.font_families_for_shaping(Some("Inter")),
            vec!["Inter"]
        );
        let resolved = resolver
            .typeface_for_char(Some("Inter, system-ui"), 'A', 400, false)
            .expect("macOS system font should cover ASCII");
        assert_eq!(resolved.typeface.family_name(), ".AppleSystemUIFont");
        assert!(resolved.typeface.font_style().weight() >= Weight::NORMAL);
    }
}
