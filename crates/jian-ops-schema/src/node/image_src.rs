//! Reference-counted image source string.
//!
//! An [`ImageNode`](super::image::ImageNode)'s `src` is frequently a
//! multi-megabyte `data:` URL (a base64-encoded raster). The editor
//! clones the whole [`PenDocument`](crate::PenDocument) on every
//! scene-cache refresh and compares the previous build's document
//! against the live one (`last.doc == state.doc`) to decide whether to
//! rebuild. With a plain `String` `src` both the clone and the compare
//! walk the entire base64 payload every frame a redraw is marked
//! (hover, scroll, caret blink, streamed chat) — that was the
//! image-drag lag.
//!
//! Wrapping the string in an `Arc<str>` makes:
//! - **Clone** O(1): a single atomic refcount bump, no copy.
//! - **Equality** O(1) on the unchanged path: a cloned document shares
//!   the same `Arc` allocation, so a pointer-first `PartialEq`
//!   (`Arc::ptr_eq`) settles instantly when the source didn't change.
//!   Only genuinely distinct allocations fall back to a byte compare.
//!
//! On the wire `ImageSrc` serializes and deserializes as a plain JSON
//! string (its `Serialize`/`Deserialize` delegate to `&str` / `String`,
//! requiring no serde `rc` feature), so existing `.op` files round-trip
//! byte-for-byte. The schema (`schemars`) and TS (`ts-rs`) exports keep
//! reporting `string` via field-level attributes on `ImageNode.src`.

use std::sync::Arc;

/// Stable, bounded identifier used by every paint and raster-cache path.
///
/// Multi-megabyte data URLs must not be re-hashed in full on every scene
/// rebuild. FNV-1a receives the source length as an explicit little-endian
/// `u64`, followed by either the complete source (up to 1536 bytes) or
/// 512-byte head, middle, and tail windows. The exact byte stream is part of
/// the persisted `imageThumbs` format; do not replace it with `Hash` or a
/// toolchain-defined hasher.
pub fn paint_image_id(src: &str) -> u64 {
    const WINDOW: usize = 512;
    let bytes = src.as_bytes();
    let mut hash = FNV1A_OFFSET;
    fnv1a_extend(&mut hash, &(bytes.len() as u64).to_le_bytes());
    if bytes.len() <= 3 * WINDOW {
        fnv1a_extend(&mut hash, bytes);
    } else {
        let middle = bytes.len() / 2;
        fnv1a_extend(&mut hash, &bytes[..WINDOW]);
        fnv1a_extend(&mut hash, &bytes[middle..middle + WINDOW]);
        fnv1a_extend(&mut hash, &bytes[bytes.len() - WINDOW..]);
    }
    hash
}

const FNV1A_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a_extend(hash: &mut u64, bytes: &[u8]) {
    for &byte in bytes {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(FNV1A_PRIME);
    }
}

/// An `Arc`-shared image source string. See the module docs for why.
#[derive(Debug, Clone)]
pub struct ImageSrc(Arc<str>);

impl ImageSrc {
    /// Borrow the source as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Cheaply clone the underlying `Arc<str>` (a refcount bump) so the
    /// scene / payload layers can carry the same allocation without
    /// copying the base64 payload.
    pub fn as_arc(&self) -> Arc<str> {
        Arc::clone(&self.0)
    }
}

impl PartialEq for ImageSrc {
    fn eq(&self, other: &Self) -> bool {
        // Pointer-equal `Arc`s — the common case after a document clone
        // where the source was not edited — settle in O(1). Distinct
        // allocations fall back to a byte compare (still correct for two
        // independently-built identical sources).
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl Eq for ImageSrc {}

// String-comparison ergonomics so existing call sites that compare the
// source against a `&str` / `String` literal keep working unchanged
// (`image.src == "data:…"`, `assert_eq!(image.src, "…")`), exactly like
// `String`'s own cross-type `PartialEq` impls.
impl PartialEq<str> for ImageSrc {
    fn eq(&self, other: &str) -> bool {
        &*self.0 == other
    }
}

impl PartialEq<&str> for ImageSrc {
    fn eq(&self, other: &&str) -> bool {
        &*self.0 == *other
    }
}

impl PartialEq<String> for ImageSrc {
    fn eq(&self, other: &String) -> bool {
        &*self.0 == other.as_str()
    }
}

impl std::ops::Deref for ImageSrc {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ImageSrc {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ImageSrc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ImageSrc {
    fn from(s: String) -> Self {
        ImageSrc(Arc::from(s))
    }
}

impl From<&str> for ImageSrc {
    fn from(s: &str) -> Self {
        ImageSrc(Arc::from(s))
    }
}

impl serde::Serialize for ImageSrc {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Transparent: emit the bare string, identical to a `String` field.
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for ImageSrc {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Read a plain JSON string (no serde `rc` feature needed) then
        // wrap it in an `Arc`. Old `.op` files store `src` as a string,
        // so they deserialize unchanged. Inside a document-load scope
        // (see [`intern`]) `op-image:` refs resolve to the load's
        // shared table payloads and duplicate data URLs collapse to
        // one allocation — 2000 fills sharing 700 images must not
        // inflate into 2000 independent copies.
        let s = String::deserialize(deserializer)?;
        Ok(intern::resolve_or_intern(s))
    }
}

/// Scoped, thread-local sharing context for document loads.
///
/// A `.op` file saved with the deduplicated image table stores each
/// payload once and references it as `op-image:<id>`. The loader
/// installs the table here for the duration of the typed parse:
/// every `ImageSrc` that deserializes to a known ref receives an
/// `Arc` CLONE of the table payload (no per-reference allocation),
/// and — as a bonus for legacy inline files — duplicate large
/// `data:` strings intern to a single shared allocation. Outside a
/// scope, deserialization is byte-for-byte the historical behaviour.
pub mod intern {
    use super::ImageSrc;
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    /// Ref prefix — must match `image_table::REF_PREFIX`.
    const REF_PREFIX: &str = "op-image:";
    /// Only intern `data:` payloads at least this large (matches the
    /// externalize threshold; hashing tiny strings buys nothing).
    const INTERN_MIN_LEN: usize = 4096;

    struct LoadScope {
        /// `op-image:<id>` → shared payload, from the file's table.
        table: HashMap<String, Arc<str>>,
        /// Content-interned large `data:` strings (legacy inline files).
        seen: HashSet<Arc<str>>,
    }

    thread_local! {
        static SCOPE: RefCell<Option<LoadScope>> = const { RefCell::new(None) };
    }

    /// Run `f` with a document-load sharing scope installed. `table`
    /// maps table ids (WITHOUT the `op-image:` prefix) to their
    /// payloads. Scopes don't nest — the guard restores the previous
    /// scope on drop (including on unwind).
    pub fn with_load_scope<R>(table: HashMap<String, Arc<str>>, f: impl FnOnce() -> R) -> R {
        struct Guard(Option<LoadScope>);
        impl Drop for Guard {
            fn drop(&mut self) {
                SCOPE.with(|s| *s.borrow_mut() = self.0.take());
            }
        }
        let prev = SCOPE.with(|s| {
            s.borrow_mut().replace(LoadScope {
                table,
                seen: HashSet::new(),
            })
        });
        let _guard = Guard(prev);
        f()
    }

    /// Resolve `s` against the active load scope (ref lookup, then
    /// content interning); plain `Arc::from` outside a scope.
    pub(super) fn resolve_or_intern(s: String) -> ImageSrc {
        SCOPE.with(|scope| {
            let mut scope = scope.borrow_mut();
            let Some(scope) = scope.as_mut() else {
                return ImageSrc(Arc::from(s));
            };
            if let Some(id) = s.strip_prefix(REF_PREFIX) {
                if let Some(payload) = scope.table.get(id) {
                    return ImageSrc(Arc::clone(payload));
                }
            }
            if s.len() >= INTERN_MIN_LEN && s.starts_with("data:") {
                if let Some(existing) = scope.seen.get(s.as_str()) {
                    return ImageSrc(Arc::clone(existing));
                }
                let arc: Arc<str> = Arc::from(s);
                scope.seen.insert(Arc::clone(&arc));
                return ImageSrc(arc);
            }
            ImageSrc(Arc::from(s))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_image_id_has_a_fixed_cross_toolchain_value() {
        assert_eq!(
            paint_image_id("data:image/png;base64,AA=="),
            0x641a_8b95_c7ff_c372
        );
    }

    #[test]
    fn paint_image_id_hashes_bounded_windows_and_the_full_length() {
        let mut source = vec![b'a'; 2_048];
        let baseline = paint_image_id(std::str::from_utf8(&source).expect("ascii"));

        source[768] = b'b';
        assert_eq!(
            paint_image_id(std::str::from_utf8(&source).expect("ascii")),
            baseline,
            "bytes outside the bounded head/middle/tail windows are skipped"
        );

        source.push(b'a');
        assert_ne!(
            paint_image_id(std::str::from_utf8(&source).expect("ascii")),
            baseline,
            "the total length participates as a fixed-width u64"
        );
    }

    #[test]
    fn serializes_and_deserializes_as_a_plain_string() {
        let src = ImageSrc::from("data:image/png;base64,AA==");
        let json = serde_json::to_string(&src).expect("serialize");
        assert_eq!(json, "\"data:image/png;base64,AA==\"");
        let back: ImageSrc = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, src);
    }

    #[test]
    fn an_old_string_value_deserializes_unchanged() {
        // The exact shape an existing .op file carries.
        let back: ImageSrc = serde_json::from_str("\"assets/logo.png\"").expect("deserialize");
        assert_eq!(back.as_str(), "assets/logo.png");
    }

    #[test]
    fn clone_shares_the_arc_and_ptr_eq_short_circuits() {
        let a = ImageSrc::from("x".repeat(10_000));
        let b = a.clone();
        // Cloned from the same source → same allocation → ptr-equal.
        assert!(Arc::ptr_eq(&a.0, &b.0));
        assert_eq!(a, b);
        // Independently built identical sources compare equal by content.
        let c = ImageSrc::from("x".repeat(10_000));
        assert!(!Arc::ptr_eq(&a.0, &c.0));
        assert_eq!(a, c);
        // A different source compares unequal.
        let d = ImageSrc::from("y");
        assert_ne!(a, d);
    }
}
