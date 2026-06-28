//! Process-global registry of host-supplied fallback font blobs.
//!
//! A host (e.g. OpenPencil) calls [`register_bundled_fonts`] once at
//! startup with the `.ttf` / `.otf` bytes of the design fonts it ships,
//! so the paragraph draw + measure paths resolve those families even
//! when they are not installed system-wide. Without this, an unknown
//! family falls back to the platform default and both *renders* and
//! *measures* with the wrong glyphs + metrics — the latter silently
//! shifts every downstream `fit_content` height.
//!
//! The blobs are kept as raw bytes (which are `Send + Sync`); the skia
//! `Typeface` / `TypefaceFontProvider` objects are rebuilt per call site
//! since they are not thread-safe to share. Callers that build a long-
//! lived `FontCollection` (the measure backend) pay this once; the
//! per-paragraph draw path already constructs a fresh collection each
//! call, so it is no worse than the status quo.

use std::sync::OnceLock;

use skia_safe::textlayout::TypefaceFontProvider;
use skia_safe::FontMgr;

static BUNDLED: OnceLock<Vec<Vec<u8>>> = OnceLock::new();

/// Register fallback font blobs. First call wins; later calls are
/// ignored, since the set is process-global and must be stable before
/// any shaping runs. Pass each font file's raw bytes.
pub fn register_bundled_fonts(blobs: Vec<Vec<u8>>) {
    let _ = BUNDLED.set(blobs);
}

/// Build a `TypefaceFontProvider` carrying every registered blob, keyed
/// by each font's own family name (alias `None`). Returns `None` when
/// nothing was registered, so callers keep their default-only
/// collection untouched.
pub fn bundled_provider() -> Option<TypefaceFontProvider> {
    let blobs = BUNDLED.get()?;
    if blobs.is_empty() {
        return None;
    }
    let mgr = FontMgr::new();
    let mut provider = TypefaceFontProvider::new();
    for bytes in blobs {
        if let Some(tf) = mgr.new_from_data(bytes, None) {
            provider.register_typeface(tf, None);
        }
    }
    Some(provider)
}
