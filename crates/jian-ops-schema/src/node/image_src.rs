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
        // so they deserialize unchanged.
        let s = String::deserialize(deserializer)?;
        Ok(ImageSrc(Arc::from(s)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
