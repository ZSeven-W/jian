//! Process-global registry of font blobs — app-shipped *bundled*
//! fonts plus user-*imported* fonts.
//!
//! A host (e.g. OpenPencil) calls [`register_bundled_fonts`] once at
//! startup with the `.ttf` / `.otf` bytes of the design fonts it ships,
//! so the paragraph draw + measure paths resolve those families even
//! when they are not installed system-wide. Users may additionally
//! [`register_imported_font`] their own faces at runtime; those take
//! precedence over system fonts of the same family (an import is a
//! deliberate override). Without this, an unknown family falls back to
//! the platform default and both *renders* and *measures* with the
//! wrong glyphs + metrics — the latter silently shifts every downstream
//! `fit_content` height.
//!
//! The blobs are kept as raw bytes (which are `Send + Sync`); the skia
//! `Typeface` / `TypefaceFontProvider` objects are rebuilt per call site
//! since they are not thread-safe to share. Every mutation bumps a
//! process-global [`generation`] counter; long-lived caches (the
//! resolver's per-char map, the measure backend's `FontCollection`, the
//! native backend's resolver, the export backend) compare against it and
//! rebuild lazily, so a runtime import reflows an already-open document.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use jian_core::layout::measure::FontStyleKind;
use skia_safe::textlayout::TypefaceFontProvider;
use skia_safe::FontMgr;

/// Where a registered blob came from. Imported faces override system
/// fonts of the same family; bundled faces are a fallback below system.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontSource {
    /// App-shipped design fonts registered once at startup.
    Bundled,
    /// User-imported faces registered at runtime.
    Imported,
}

/// One registered font face plus the metadata the registry keys on.
#[derive(Clone)]
pub struct FontBlob {
    /// The face's own family name (extracted from the file).
    pub family: String,
    /// Normal vs Italic (from the face's slant).
    pub style: FontStyleKind,
    /// Numeric weight 100..900 (from the face's `OS/2` weight).
    pub weight: u16,
    /// Content hash — persistence filename + bytes-equal dedup tiebreak.
    pub hash: u64,
    /// Provenance (bundled vs imported).
    pub source: FontSource,
    /// Raw `.ttf` / `.otf` bytes.
    pub bytes: Arc<Vec<u8>>,
}

/// Per-family summary for the import picker: one row per family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyMeta {
    pub family: String,
    /// How many imported faces (weights/styles) this family groups.
    pub face_count: usize,
}

#[derive(Default)]
struct Registry {
    fonts: Vec<FontBlob>,
}

static REGISTRY: OnceLock<RwLock<Registry>> = OnceLock::new();
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn registry() -> &'static RwLock<Registry> {
    REGISTRY.get_or_init(|| RwLock::new(Registry::default()))
}

/// Monotonic counter bumped on every add/remove. Long-lived caches
/// compare their built-at value against this to know when to rebuild.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

fn bump_generation() {
    GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// Register app-shipped fallback font blobs. First call wins; later
/// calls are ignored, since the bundled set is process-global and must
/// be stable before any shaping runs. Pass each font file's raw bytes.
pub fn register_bundled_fonts(blobs: Vec<Vec<u8>>) {
    // An empty call registers nothing, leaves no `Bundled` marker, and must
    // not bump the generation — otherwise it would both spend a needless
    // rebuild and fail to reserve "first call wins", letting a later real
    // call be (correctly) honored while having already perturbed callers.
    if blobs.is_empty() {
        return;
    }
    let reg = registry();
    {
        let guard = reg.read().expect("font registry poisoned");
        if guard.fonts.iter().any(|f| f.source == FontSource::Bundled) {
            return; // first call wins
        }
    }
    let mut guard = reg.write().expect("font registry poisoned");
    // Re-check under the write lock in case of a race.
    if guard.fonts.iter().any(|f| f.source == FontSource::Bundled) {
        return;
    }
    for bytes in blobs {
        let (family, style, weight) =
            parse_face_meta(&bytes).unwrap_or_else(|| (String::new(), FontStyleKind::Normal, 400));
        guard.fonts.push(FontBlob {
            family,
            style,
            weight,
            hash: content_hash(&bytes),
            source: FontSource::Bundled,
            bytes: Arc::new(bytes),
        });
    }
    drop(guard);
    bump_generation();
}

/// Font metadata extracted from raw bytes *without* mutating the
/// registry. Lets a caller (e.g. the desktop `FontStore`) validate a
/// file and learn its `(family, style, weight, hash)` — enough to name
/// the persisted file + index entry — so it can commit the file to disk
/// BEFORE calling [`register_imported_font`]. Registering first would
/// leave a live-but-unpersisted font in the process registry if the
/// subsequent disk write failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedFontMeta {
    pub family: String,
    pub style: FontStyleKind,
    pub weight: u16,
    pub hash: u64,
}

/// Validate + parse imported font bytes without touching the registry.
/// Returns `None` when the bytes are not a font skia can parse or the
/// face has no family name (same rejection as [`register_imported_font`]).
/// The `hash` matches what `register_imported_font` computes for the same
/// bytes, so a file named by this hash lines up with the later blob.
pub fn parse_imported_font_meta(bytes: &[u8]) -> Option<ImportedFontMeta> {
    let (family, style, weight) = parse_face_meta(bytes)?;
    if family.is_empty() {
        return None;
    }
    Some(ImportedFontMeta {
        family,
        style,
        weight,
        hash: content_hash(bytes),
    })
}

/// Register a user-imported font face. Parses family/style/weight from
/// the bytes; dedups on `(family, style, weight)` with last-import-wins.
///
/// A re-import of a byte-identical file for an existing face is a no-op
/// (returns the existing blob, no generation bump). A same-key import of
/// *different* bytes replaces the face. `hash` is only a content key +
/// bytes-equal tiebreak, so a `u64` hash collision can never silently
/// drop a distinct font.
pub fn register_imported_font(bytes: Vec<u8>) -> Result<FontBlob, String> {
    let (family, style, weight) =
        parse_face_meta(&bytes).ok_or_else(|| "not a valid ttf/otf font file".to_string())?;
    if family.is_empty() {
        return Err("font file has no family name".to_string());
    }
    let hash = content_hash(&bytes);
    let mut guard = registry().write().expect("font registry poisoned");

    if let Some(existing) = guard.fonts.iter().find(|f| {
        f.source == FontSource::Imported
            && f.family == family
            && f.style == style
            && f.weight == weight
    }) {
        // Same face key + byte-identical file → nothing changed.
        if existing.hash == hash && existing.bytes.as_slice() == bytes.as_slice() {
            return Ok(existing.clone());
        }
    }

    let blob = FontBlob {
        family: family.clone(),
        style,
        weight,
        hash,
        source: FontSource::Imported,
        bytes: Arc::new(bytes),
    };
    // Replace any existing face with the same key (last-import-wins),
    // else append.
    guard.fonts.retain(|f| {
        !(f.source == FontSource::Imported
            && f.family == family
            && f.style == style
            && f.weight == weight)
    });
    guard.fonts.push(blob.clone());
    drop(guard);
    bump_generation();
    Ok(blob)
}

/// Remove every imported face of `family` (remove targets the whole
/// family — there is no per-face remove). Returns `true` if anything was
/// removed. Bundled faces are never touched.
pub fn remove_imported_font(family: &str) -> bool {
    let mut guard = registry().write().expect("font registry poisoned");
    let before = guard.fonts.len();
    guard
        .fonts
        .retain(|f| !(f.source == FontSource::Imported && f.family == family));
    let removed = guard.fonts.len() != before;
    drop(guard);
    if removed {
        bump_generation();
    }
    removed
}

/// One row per imported family (weights/styles collapsed), sorted by
/// family name for a stable picker order.
pub fn list_families() -> Vec<FamilyMeta> {
    let guard = registry().read().expect("font registry poisoned");
    let mut metas: Vec<FamilyMeta> = Vec::new();
    for blob in guard
        .fonts
        .iter()
        .filter(|f| f.source == FontSource::Imported)
    {
        if let Some(meta) = metas.iter_mut().find(|m| m.family == blob.family) {
            meta.face_count += 1;
        } else {
            metas.push(FamilyMeta {
                family: blob.family.clone(),
                face_count: 1,
            });
        }
    }
    metas.sort_by(|a, b| a.family.cmp(&b.family));
    metas
}

/// Build a `TypefaceFontProvider` carrying every *bundled* blob, keyed
/// by each font's own family name (alias `None`). Returns `None` when
/// nothing is registered, so callers keep their default-only collection
/// untouched.
pub fn bundled_provider() -> Option<TypefaceFontProvider> {
    provider_for(FontSource::Bundled)
}

/// Build a `TypefaceFontProvider` carrying every *imported* blob, keyed
/// by each font's own family name (alias `None`). Returns `None` when no
/// import is registered.
pub fn imported_provider() -> Option<TypefaceFontProvider> {
    provider_for(FontSource::Imported)
}

/// Build one `TypefaceFontProvider` carrying imported faces first, then
/// bundled — for use as a Paragraph asset font manager (a fallback below
/// the ordered default manager). Returns `None` when nothing at all is
/// registered.
pub fn asset_provider() -> Option<TypefaceFontProvider> {
    let guard = registry().read().expect("font registry poisoned");
    if guard.fonts.is_empty() {
        return None;
    }
    let mgr = FontMgr::new();
    let mut provider = TypefaceFontProvider::new();
    let ordered = guard
        .fonts
        .iter()
        .filter(|f| f.source == FontSource::Imported)
        .chain(
            guard
                .fonts
                .iter()
                .filter(|f| f.source == FontSource::Bundled),
        );
    let mut any = false;
    for blob in ordered {
        if let Some(tf) = mgr.new_from_data(&blob.bytes, None) {
            provider.register_typeface(tf, None);
            any = true;
        }
    }
    any.then_some(provider)
}

fn provider_for(source: FontSource) -> Option<TypefaceFontProvider> {
    let guard = registry().read().expect("font registry poisoned");
    let mgr = FontMgr::new();
    let mut provider = TypefaceFontProvider::new();
    let mut any = false;
    for blob in guard.fonts.iter().filter(|f| f.source == source) {
        if let Some(tf) = mgr.new_from_data(&blob.bytes, None) {
            provider.register_typeface(tf, None);
            any = true;
        }
    }
    any.then_some(provider)
}

/// Extract `(family, style, weight)` from raw font bytes via skia. Returns
/// `None` when the bytes are not a font skia can parse.
fn parse_face_meta(bytes: &[u8]) -> Option<(String, FontStyleKind, u16)> {
    let mgr = FontMgr::new();
    let typeface = mgr.new_from_data(bytes, None)?;
    let family = typeface.family_name();
    let fs = typeface.font_style();
    let weight = (*fs.weight()).clamp(1, 1000) as u16;
    let style = match fs.slant() {
        skia_safe::font_style::Slant::Upright => FontStyleKind::Normal,
        _ => FontStyleKind::Italic,
    };
    Some((family, style, weight))
}

fn content_hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}
