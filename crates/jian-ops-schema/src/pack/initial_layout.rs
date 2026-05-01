//! `aot/initial_layout.bin` — pre-computed first-frame rects for the
//! default viewport (Plan 19 Task 6 D1).
//!
//! ## Wire format (little-endian)
//!
//! ```text
//! [0..4]   magic        b"OPL1"           (4 bytes)
//! [4..6]   version      u16 = 1           (2 bytes)
//! [6..10]  viewport_w   f32 (LE)          (4 bytes)
//! [10..14] viewport_h   f32 (LE)          (4 bytes)
//! [14..18] rect_count   u32 (LE)          (4 bytes)
//! For each rect:
//!   [..2]  id_len       u16 (LE)          (2 bytes)
//!   [..N]  id           UTF-8             (id_len bytes)
//!   [..16] rect         [f32; 4] (LE)     (16 bytes — x, y, w, h)
//! ```
//!
//! Total header is 18 bytes. Each rect costs `2 + id_len + 16` bytes
//! (≈26 bytes for typical 8-char ids; ≈40-50 bytes for namespaced ids).
//!
//! `OPL1` is the format magic; bumping the trailing digit signals a
//! breaking wire change. The current writer + reader are wire-pinned by
//! a constants test below.
//!
//! ## Why hand-rolled, not bincode
//!
//! `jian-ops-schema` deliberately stays free of binary-serialization
//! deps so the schema crate keeps a tight footprint (CI containers,
//! WASM hosts, downstream tools). The layout snapshot's shape is
//! fixed-width-per-row, so a little-endian SoA serializer is ~50 LOC
//! and easier to audit than a derived bincode format whose stability
//! semantics depend on field ordering.
//!
//! ## Why this format and not JSON
//!
//! A 200-node doc produces ~5 KB of binary vs ~22 KB of JSON. The
//! cold-start budget cares about both byte count (zip-internal seek)
//! and parse cost (`f32::from_le_bytes` vs `serde_json::from_slice`).
//! Per Plan 19 Task 6's "skip first layout pass" target, the parse
//! cost for the snapshot must stay sub-millisecond on every realistic
//! doc; that rules out JSON parsing on slow targets.

use crate::pack::manifest::DefaultViewport;
use std::collections::BTreeMap;

/// Magic header bytes identifying this format. A reader that doesn't
/// see these four bytes at offset 0 must reject the input.
pub const INITIAL_LAYOUT_MAGIC: [u8; 4] = *b"OPL1";

/// Current format version. Bump on a wire-breaking change. Additive
/// changes (new optional fields) need a new magic instead so old
/// readers reject loudly rather than misparse.
pub const INITIAL_LAYOUT_VERSION: u16 = 1;

/// Errors the writer / reader can produce. Two-axis: structural
/// (wrong magic / truncated) vs content (id not UTF-8, non-finite
/// f32). Both are unrecoverable; the reader logs and falls back to
/// a fresh layout pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitialLayoutError {
    /// Input shorter than the fixed header (18 bytes).
    TooShort {
        /// Actual byte count seen.
        got: usize,
        /// Minimum bytes required to read the header.
        need: usize,
    },
    /// Magic bytes don't match [`INITIAL_LAYOUT_MAGIC`]. Probably not
    /// a snapshot at all — caller should fall back rather than retry.
    BadMagic { got: [u8; 4] },
    /// Version field doesn't match a supported version. Currently
    /// only [`INITIAL_LAYOUT_VERSION`] is recognised; old readers see
    /// a future format and bail rather than misinterpret.
    UnsupportedVersion { got: u16 },
    /// A rect's id length pointed past the end of the buffer.
    Truncated {
        /// Zero-based index of the rect that ran out.
        rect_index: usize,
    },
    /// A rect's id bytes weren't valid UTF-8.
    InvalidIdUtf8 {
        /// Zero-based index of the offending rect.
        rect_index: usize,
    },
    /// A rect's f32 field decoded as `NaN` or `inf`. Layout rects
    /// must be finite — a non-finite value indicates corruption.
    /// Writer: returned by [`InitialLayoutSnapshot::write_bytes`] to
    /// reject smuggling NaN/inf into the file; reader: returned by
    /// [`InitialLayoutSnapshot::read_bytes`] for the same reason.
    NonFiniteRect {
        /// Zero-based index of the offending rect.
        rect_index: usize,
        /// Which f32 (`x` / `y` / `w` / `h`) tripped the check.
        component: &'static str,
    },
    /// Viewport `width` / `height` is non-finite or non-positive.
    /// The runtime preload path has no meaningful interpretation for
    /// a `0 × 0` or `NaN` viewport, so the encoder/decoder both reject.
    InvalidViewport {
        /// `"width"` or `"height"`.
        component: &'static str,
    },
    /// An id whose length exceeds `u16::MAX` (65535) bytes can't be
    /// represented in the wire format. Writer-side only — the reader
    /// can't observe this case because a u16 length field bounds it.
    /// Returned by [`InitialLayoutSnapshot::write_bytes`].
    IdTooLong {
        /// Index into the `BTreeMap` iteration order of the offending id.
        rect_index: usize,
        /// Length in bytes of the offending id.
        len: usize,
    },
    /// Reader saw bytes after the declared `rect_count` was satisfied.
    /// A trailing-byte tolerance would mask zip-level corruption that
    /// happens to land at exactly the layout-bin offset.
    TrailingBytes {
        /// Number of bytes left over.
        leftover: usize,
    },
    /// Two rects shared the same id. The writer can't emit duplicates
    /// (a `BTreeMap` enforces unique keys at type level), so a
    /// duplicate on the wire signals corruption or a malicious blob.
    /// Codex round 2 MEDIUM.
    DuplicateId {
        /// Zero-based index of the second rect with the colliding id.
        rect_index: usize,
    },
}

impl std::fmt::Display for InitialLayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort { got, need } => {
                write!(f, "input too short: got {got} bytes, need at least {need}")
            }
            Self::BadMagic { got } => write!(
                f,
                "bad magic: got {:02x?}, expected {:02x?}",
                got, INITIAL_LAYOUT_MAGIC
            ),
            Self::UnsupportedVersion { got } => {
                write!(f, "unsupported initial-layout version {got}")
            }
            Self::Truncated { rect_index } => {
                write!(f, "truncated input at rect {rect_index}")
            }
            Self::InvalidIdUtf8 { rect_index } => {
                write!(f, "invalid utf-8 id at rect {rect_index}")
            }
            Self::NonFiniteRect {
                rect_index,
                component,
            } => write!(f, "non-finite {component} at rect {rect_index}"),
            Self::InvalidViewport { component } => {
                write!(f, "invalid viewport {component} (must be finite and > 0)")
            }
            Self::IdTooLong { rect_index, len } => write!(
                f,
                "id at rect {rect_index} is {len} bytes, exceeds u16::MAX"
            ),
            Self::TrailingBytes { leftover } => {
                write!(f, "trailing {leftover} byte(s) after declared rect count")
            }
            Self::DuplicateId { rect_index } => {
                write!(f, "duplicate id at rect {rect_index}")
            }
        }
    }
}

impl std::error::Error for InitialLayoutError {}

/// One node's first-frame rect. `[x, y, w, h]` in scene-coord f32, the
/// same shape `LayoutEngine::node_rect` returns at runtime. Stored as
/// a tuple struct rather than a named struct to keep the binary footprint
/// minimal (no field-name overhead via serde).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PackedRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl PackedRect {
    /// Construct from a 4-tuple `(x, y, w, h)`. Helper for callsites
    /// that already produce taffy-shaped tuples.
    pub fn from_xywh(xywh: (f32, f32, f32, f32)) -> Self {
        Self {
            x: xywh.0,
            y: xywh.1,
            w: xywh.2,
            h: xywh.3,
        }
    }

    /// Inverse of [`Self::from_xywh`].
    pub fn into_xywh(self) -> (f32, f32, f32, f32) {
        (self.x, self.y, self.w, self.h)
    }
}

/// Pre-computed first-frame layout for a single document at a single
/// viewport. The runtime preloads this when `BootstrapSource::Pack`
/// is fed an archive that includes [`crate::pack::ENTRY_AOT_INITIAL_LAYOUT`].
/// `BTreeMap` (vs `HashMap`) gives a stable iteration order so the
/// serialised bytes are deterministic — important for content-
/// addressed pack hashes and for diff-friendly CI fixtures.
#[derive(Debug, Clone, PartialEq)]
pub struct InitialLayoutSnapshot {
    pub viewport: DefaultViewport,
    pub rects: BTreeMap<String, PackedRect>,
}

impl InitialLayoutSnapshot {
    /// Serialise to the `aot/initial_layout.bin` wire format. See the
    /// module-level doc for the byte-by-byte layout.
    ///
    /// Returns an error when:
    /// - `viewport.width` / `viewport.height` is non-finite or `<= 0`
    ///   ([`InitialLayoutError::InvalidViewport`]) — codex round 1
    ///   MEDIUM: the reader rejected non-finite viewports but the
    ///   writer used to smuggle them.
    /// - any `PackedRect` component is non-finite
    ///   ([`InitialLayoutError::NonFiniteRect`]) — codex round 1 HIGH:
    ///   a `PackedRect` constructed directly (not via the checked
    ///   reader path) could have NaN/inf and the writer didn't
    ///   re-validate.
    /// - any id's UTF-8 byte length exceeds `u16::MAX`
    ///   ([`InitialLayoutError::IdTooLong`]) — codex round 1 HIGH:
    ///   `as u16` truncation silently corrupted the byte stream.
    pub fn write_bytes(&self) -> Result<Vec<u8>, InitialLayoutError> {
        if !self.viewport.width.is_finite() || self.viewport.width <= 0.0 {
            return Err(InitialLayoutError::InvalidViewport { component: "width" });
        }
        if !self.viewport.height.is_finite() || self.viewport.height <= 0.0 {
            return Err(InitialLayoutError::InvalidViewport {
                component: "height",
            });
        }
        // Pre-size the output: 18-byte header + per-rect (2 + id_len + 16).
        let body: usize = self.rects.keys().map(|k| 2 + k.len() + 16).sum();
        let mut out = Vec::with_capacity(18 + body);
        out.extend_from_slice(&INITIAL_LAYOUT_MAGIC);
        out.extend_from_slice(&INITIAL_LAYOUT_VERSION.to_le_bytes());
        out.extend_from_slice(&self.viewport.width.to_le_bytes());
        out.extend_from_slice(&self.viewport.height.to_le_bytes());
        // `rects.len()` fits u32 by `BTreeMap`'s capacity (architecture
        // word size); a 4 G-rect doc would have other problems first.
        out.extend_from_slice(&(self.rects.len() as u32).to_le_bytes());
        for (rect_index, (id, r)) in self.rects.iter().enumerate() {
            let id_len = u16::try_from(id.len()).map_err(|_| InitialLayoutError::IdTooLong {
                rect_index,
                len: id.len(),
            })?;
            for (component, v) in [("x", r.x), ("y", r.y), ("w", r.w), ("h", r.h)] {
                if !v.is_finite() {
                    return Err(InitialLayoutError::NonFiniteRect {
                        rect_index,
                        component,
                    });
                }
            }
            out.extend_from_slice(&id_len.to_le_bytes());
            out.extend_from_slice(id.as_bytes());
            out.extend_from_slice(&r.x.to_le_bytes());
            out.extend_from_slice(&r.y.to_le_bytes());
            out.extend_from_slice(&r.w.to_le_bytes());
            out.extend_from_slice(&r.h.to_le_bytes());
        }
        Ok(out)
    }

    /// Inverse of [`Self::write_bytes`]. See [`InitialLayoutError`] for
    /// the error taxonomy. On any error the caller should fall back to
    /// a fresh `build_layout` pass — the snapshot is an optimisation,
    /// not a correctness gate.
    pub fn read_bytes(bytes: &[u8]) -> Result<Self, InitialLayoutError> {
        const HEADER: usize = 18;
        if bytes.len() < HEADER {
            return Err(InitialLayoutError::TooShort {
                got: bytes.len(),
                need: HEADER,
            });
        }
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&bytes[0..4]);
        if magic != INITIAL_LAYOUT_MAGIC {
            return Err(InitialLayoutError::BadMagic { got: magic });
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != INITIAL_LAYOUT_VERSION {
            return Err(InitialLayoutError::UnsupportedVersion { got: version });
        }
        let width = f32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]);
        let height = f32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        // Codex round 1 MEDIUM: viewport components were decoded
        // unchecked. Reject NaN/inf/0 so the runtime preload path
        // can rely on a positive-finite default viewport.
        if !width.is_finite() || width <= 0.0 {
            return Err(InitialLayoutError::InvalidViewport { component: "width" });
        }
        if !height.is_finite() || height <= 0.0 {
            return Err(InitialLayoutError::InvalidViewport {
                component: "height",
            });
        }
        let count = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]) as usize;

        let mut rects = BTreeMap::new();
        let mut cursor = HEADER;
        for rect_index in 0..count {
            if cursor + 2 > bytes.len() {
                return Err(InitialLayoutError::Truncated { rect_index });
            }
            let id_len = u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]]) as usize;
            cursor += 2;
            if cursor + id_len + 16 > bytes.len() {
                return Err(InitialLayoutError::Truncated { rect_index });
            }
            let id_bytes = &bytes[cursor..cursor + id_len];
            let id = std::str::from_utf8(id_bytes)
                .map_err(|_| InitialLayoutError::InvalidIdUtf8 { rect_index })?
                .to_owned();
            cursor += id_len;
            let x = f32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
            let y = f32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap());
            let w = f32::from_le_bytes(bytes[cursor + 8..cursor + 12].try_into().unwrap());
            let h = f32::from_le_bytes(bytes[cursor + 12..cursor + 16].try_into().unwrap());
            cursor += 16;
            // Reject non-finite components — a NaN width would silently
            // poison every downstream rect calculation.
            for (component, v) in [("x", x), ("y", y), ("w", w), ("h", h)] {
                if !v.is_finite() {
                    return Err(InitialLayoutError::NonFiniteRect {
                        rect_index,
                        component,
                    });
                }
            }
            // Codex round 2 MEDIUM: a `BTreeMap::insert` on a
            // duplicate key silently overwrites. The writer can't
            // produce duplicates (BTreeMap dedupes at type level), so
            // a duplicate on the wire is corruption — surface it.
            if rects.insert(id, PackedRect { x, y, w, h }).is_some() {
                return Err(InitialLayoutError::DuplicateId { rect_index });
            }
        }
        // Codex round 1 MEDIUM: tolerate-and-pass on trailing bytes
        // would mask zip-level corruption that lands at the layout-bin
        // offset. Reject leftover bytes so the caller falls back to a
        // fresh layout pass instead of using a partially-trusted blob.
        if cursor != bytes.len() {
            return Err(InitialLayoutError::TrailingBytes {
                leftover: bytes.len() - cursor,
            });
        }
        Ok(Self {
            viewport: DefaultViewport { width, height },
            rects,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(rects: &[(&str, f32, f32, f32, f32)]) -> InitialLayoutSnapshot {
        let mut map = BTreeMap::new();
        for (id, x, y, w, h) in rects {
            map.insert(
                (*id).to_owned(),
                PackedRect {
                    x: *x,
                    y: *y,
                    w: *w,
                    h: *h,
                },
            );
        }
        InitialLayoutSnapshot {
            viewport: DefaultViewport {
                width: 800.0,
                height: 600.0,
            },
            rects: map,
        }
    }

    #[test]
    fn magic_and_version_are_pinned() {
        // Wire compatibility — bumping these requires a coordinated
        // reader update; the test guards against accidental edits.
        assert_eq!(&INITIAL_LAYOUT_MAGIC, b"OPL1");
        assert_eq!(INITIAL_LAYOUT_VERSION, 1);
    }

    #[test]
    fn round_trip_empty_snapshot() {
        let s = InitialLayoutSnapshot {
            viewport: DefaultViewport {
                width: 320.0,
                height: 240.0,
            },
            rects: BTreeMap::new(),
        };
        let bytes = s.write_bytes().expect("encode");
        // Header only — 18 bytes.
        assert_eq!(bytes.len(), 18);
        let back = InitialLayoutSnapshot::read_bytes(&bytes).expect("decode");
        assert_eq!(back, s);
    }

    #[test]
    fn round_trip_simple_three_rect_doc() {
        let s = snap(&[
            ("root", 0.0, 0.0, 800.0, 600.0),
            ("title", 16.0, 16.0, 200.0, 32.0),
            ("button", 16.0, 64.0, 120.0, 40.0),
        ]);
        let bytes = s.write_bytes().expect("encode");
        let back = InitialLayoutSnapshot::read_bytes(&bytes).expect("decode");
        assert_eq!(back, s);
    }

    #[test]
    fn round_trip_preserves_viewport() {
        let s = snap(&[("a", 1.0, 2.0, 3.0, 4.0)]);
        let bytes = s.write_bytes().expect("encode");
        let back = InitialLayoutSnapshot::read_bytes(&bytes).expect("decode");
        assert_eq!(back.viewport.width, 800.0);
        assert_eq!(back.viewport.height, 600.0);
    }

    #[test]
    fn round_trip_preserves_id_order_via_btreemap() {
        // A stable iteration order means the same input produces the
        // same bytes. Two snapshots with the same rects (different
        // insert order) must round-trip to byte-identical output.
        let s1 = snap(&[
            ("z", 0.0, 0.0, 0.0, 0.0),
            ("a", 0.0, 0.0, 0.0, 0.0),
            ("m", 0.0, 0.0, 0.0, 0.0),
        ]);
        let s2 = snap(&[
            ("a", 0.0, 0.0, 0.0, 0.0),
            ("m", 0.0, 0.0, 0.0, 0.0),
            ("z", 0.0, 0.0, 0.0, 0.0),
        ]);
        assert_eq!(
            s1.write_bytes().expect("encode"),
            s2.write_bytes().expect("encode")
        );
    }

    #[test]
    fn round_trip_preserves_unicode_ids() {
        // Author ids can be any UTF-8 string. Test a mix of ASCII,
        // CJK, and punctuation to pin the utf-8-clean wire shape.
        let s = snap(&[
            ("ascii", 0.0, 0.0, 0.0, 0.0),
            ("中文-id", 0.0, 0.0, 0.0, 0.0),
            ("emoji-🎨", 0.0, 0.0, 0.0, 0.0),
        ]);
        let bytes = s.write_bytes().expect("encode");
        let back = InitialLayoutSnapshot::read_bytes(&bytes).expect("decode");
        assert_eq!(back, s);
        assert!(back.rects.contains_key("中文-id"));
        assert!(back.rects.contains_key("emoji-🎨"));
    }

    #[test]
    fn empty_input_rejects_with_too_short() {
        let err = InitialLayoutSnapshot::read_bytes(&[]).unwrap_err();
        assert_eq!(err, InitialLayoutError::TooShort { got: 0, need: 18 });
    }

    #[test]
    fn truncated_header_rejects_with_too_short() {
        let err = InitialLayoutSnapshot::read_bytes(&[1, 2, 3, 4, 5]).unwrap_err();
        assert_eq!(err, InitialLayoutError::TooShort { got: 5, need: 18 });
    }

    #[test]
    fn wrong_magic_is_rejected() {
        let mut bytes = vec![b'O', b'P', b'L', b'2'];
        bytes.extend_from_slice(&[0u8; 14]); // pad to 18
        let err = InitialLayoutSnapshot::read_bytes(&bytes).unwrap_err();
        assert!(matches!(err, InitialLayoutError::BadMagic { got } if got == *b"OPL2"));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let mut bytes = INITIAL_LAYOUT_MAGIC.to_vec();
        bytes.extend_from_slice(&99u16.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 12]); // viewport + count
        let err = InitialLayoutSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(err, InitialLayoutError::UnsupportedVersion { got: 99 });
    }

    #[test]
    fn truncated_after_header_is_rejected() {
        // Header claims 1 rect but no rect bytes follow.
        let mut bytes = INITIAL_LAYOUT_MAGIC.to_vec();
        bytes.extend_from_slice(&INITIAL_LAYOUT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&100.0f32.to_le_bytes());
        bytes.extend_from_slice(&50.0f32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        let err = InitialLayoutSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(err, InitialLayoutError::Truncated { rect_index: 0 });
    }

    #[test]
    fn truncated_in_id_bytes_is_rejected() {
        // Header + id_len=10 but only 4 id bytes available.
        let mut bytes = INITIAL_LAYOUT_MAGIC.to_vec();
        bytes.extend_from_slice(&INITIAL_LAYOUT_VERSION.to_le_bytes());
        // Valid viewport (the InvalidViewport guard added in
        // round 2 rejects 0/NaN — the tests below want to exercise
        // *other* error paths so they get a sensible viewport).
        bytes.extend_from_slice(&800.0f32.to_le_bytes());
        bytes.extend_from_slice(&600.0f32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&10u16.to_le_bytes());
        bytes.extend_from_slice(b"abcd"); // 4 bytes, claim 10
        let err = InitialLayoutSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(err, InitialLayoutError::Truncated { rect_index: 0 });
    }

    #[test]
    fn invalid_utf8_id_is_rejected() {
        let mut bytes = INITIAL_LAYOUT_MAGIC.to_vec();
        bytes.extend_from_slice(&INITIAL_LAYOUT_VERSION.to_le_bytes());
        // Valid viewport (the InvalidViewport guard added in
        // round 2 rejects 0/NaN — the tests below want to exercise
        // *other* error paths so they get a sensible viewport).
        bytes.extend_from_slice(&800.0f32.to_le_bytes());
        bytes.extend_from_slice(&600.0f32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&[0xff, 0xfe]); // invalid utf-8
        bytes.extend_from_slice(&[0u8; 16]); // rect padding
        let err = InitialLayoutSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(err, InitialLayoutError::InvalidIdUtf8 { rect_index: 0 });
    }

    #[test]
    fn non_finite_rect_component_is_rejected() {
        let mut bytes = INITIAL_LAYOUT_MAGIC.to_vec();
        bytes.extend_from_slice(&INITIAL_LAYOUT_VERSION.to_le_bytes());
        // Valid viewport (the InvalidViewport guard added in
        // round 2 rejects 0/NaN — the tests below want to exercise
        // *other* error paths so they get a sensible viewport).
        bytes.extend_from_slice(&800.0f32.to_le_bytes());
        bytes.extend_from_slice(&600.0f32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(b"a");
        bytes.extend_from_slice(&f32::NAN.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 12]);
        let err = InitialLayoutSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(
            err,
            InitialLayoutError::NonFiniteRect {
                rect_index: 0,
                component: "x"
            }
        );
    }

    #[test]
    fn packed_rect_xywh_round_trip() {
        let r = PackedRect::from_xywh((1.0, 2.0, 3.0, 4.0));
        assert_eq!(r.x, 1.0);
        assert_eq!(r.y, 2.0);
        assert_eq!(r.w, 3.0);
        assert_eq!(r.h, 4.0);
        assert_eq!(r.into_xywh(), (1.0, 2.0, 3.0, 4.0));
    }

    // ──────────────────────────────────────────────────────────────
    // Codex round 1 (HIGH/MEDIUM) — writer-side validation +
    // reader-side trailing-bytes / viewport-finiteness rejection.
    // ──────────────────────────────────────────────────────────────

    #[test]
    fn write_bytes_rejects_non_finite_rect_components() {
        // A `PackedRect` constructed with NaN must not silently
        // serialise — codex round 1 HIGH (round-trip correctness).
        let mut rects = BTreeMap::new();
        rects.insert(
            "bad".to_owned(),
            PackedRect {
                x: f32::NAN,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
        );
        let s = InitialLayoutSnapshot {
            viewport: DefaultViewport {
                width: 800.0,
                height: 600.0,
            },
            rects,
        };
        let err = s.write_bytes().unwrap_err();
        assert_eq!(
            err,
            InitialLayoutError::NonFiniteRect {
                rect_index: 0,
                component: "x"
            }
        );
    }

    #[test]
    fn write_bytes_rejects_oversized_id() {
        // An id whose UTF-8 byte length exceeds u16::MAX can't be
        // represented — codex round 1 HIGH (silent truncation).
        let mut rects = BTreeMap::new();
        let huge_id = "a".repeat(u16::MAX as usize + 1);
        rects.insert(
            huge_id,
            PackedRect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
        );
        let s = InitialLayoutSnapshot {
            viewport: DefaultViewport {
                width: 800.0,
                height: 600.0,
            },
            rects,
        };
        let err = s.write_bytes().unwrap_err();
        assert!(matches!(
            err,
            InitialLayoutError::IdTooLong { rect_index: 0, .. }
        ));
    }

    #[test]
    fn write_bytes_rejects_invalid_viewport() {
        // Mirror of the reader-side guard — codex round 1 MEDIUM.
        let mut bad_w = InitialLayoutSnapshot {
            viewport: DefaultViewport {
                width: 0.0,
                height: 600.0,
            },
            rects: BTreeMap::new(),
        };
        assert!(matches!(
            bad_w.write_bytes(),
            Err(InitialLayoutError::InvalidViewport { component: "width" })
        ));
        bad_w.viewport.width = f32::NAN;
        assert!(matches!(
            bad_w.write_bytes(),
            Err(InitialLayoutError::InvalidViewport { component: "width" })
        ));
        bad_w.viewport.width = 800.0;
        bad_w.viewport.height = -1.0;
        assert!(matches!(
            bad_w.write_bytes(),
            Err(InitialLayoutError::InvalidViewport {
                component: "height"
            })
        ));
    }

    #[test]
    fn read_bytes_rejects_invalid_viewport() {
        // Crafted bytes with NaN viewport.width must be rejected —
        // codex round 1 MEDIUM (reader-side viewport finiteness).
        let mut bytes = INITIAL_LAYOUT_MAGIC.to_vec();
        bytes.extend_from_slice(&INITIAL_LAYOUT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&f32::NAN.to_le_bytes());
        bytes.extend_from_slice(&600.0f32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let err = InitialLayoutSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(
            err,
            InitialLayoutError::InvalidViewport { component: "width" }
        );
    }

    #[test]
    fn read_bytes_rejects_zero_viewport() {
        let mut bytes = INITIAL_LAYOUT_MAGIC.to_vec();
        bytes.extend_from_slice(&INITIAL_LAYOUT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&800.0f32.to_le_bytes());
        bytes.extend_from_slice(&0.0f32.to_le_bytes()); // height = 0
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let err = InitialLayoutSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(
            err,
            InitialLayoutError::InvalidViewport {
                component: "height"
            }
        );
    }

    #[test]
    fn read_bytes_rejects_trailing_bytes() {
        // A well-formed snapshot followed by garbage must be
        // rejected so zip-level corruption can't masquerade as a
        // valid blob — codex round 1 MEDIUM.
        let s = InitialLayoutSnapshot {
            viewport: DefaultViewport {
                width: 320.0,
                height: 240.0,
            },
            rects: BTreeMap::new(),
        };
        let mut bytes = s.write_bytes().expect("encode");
        bytes.extend_from_slice(&[0xff, 0xff, 0xff]);
        let err = InitialLayoutSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(err, InitialLayoutError::TrailingBytes { leftover: 3 });
    }

    #[test]
    fn read_bytes_rejects_duplicate_ids() {
        // Codex round 2 MEDIUM: a wire blob with two rects sharing
        // an id used to silently overwrite via BTreeMap insert. The
        // writer can't produce duplicates, so a duplicate on the
        // wire signals corruption — surface it as an error rather
        // than dropping a row silently.
        let mut bytes = INITIAL_LAYOUT_MAGIC.to_vec();
        bytes.extend_from_slice(&INITIAL_LAYOUT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&800.0f32.to_le_bytes());
        bytes.extend_from_slice(&600.0f32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes()); // declare 2 rects
                                                      // First rect: id="a", rect (1,2,3,4).
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(b"a");
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        bytes.extend_from_slice(&2.0f32.to_le_bytes());
        bytes.extend_from_slice(&3.0f32.to_le_bytes());
        bytes.extend_from_slice(&4.0f32.to_le_bytes());
        // Second rect: id="a" again, rect (5,6,7,8) — duplicate.
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(b"a");
        bytes.extend_from_slice(&5.0f32.to_le_bytes());
        bytes.extend_from_slice(&6.0f32.to_le_bytes());
        bytes.extend_from_slice(&7.0f32.to_le_bytes());
        bytes.extend_from_slice(&8.0f32.to_le_bytes());
        let err = InitialLayoutSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(err, InitialLayoutError::DuplicateId { rect_index: 1 });
    }

    #[test]
    fn write_then_read_finite_negative_values_round_trip() {
        // Negative coordinates / sizes are legitimate (off-viewport
        // anchors, drag handles) — only NaN/inf are forbidden. Pin
        // that the writer accepts negative finite values and the
        // reader hands them back unchanged.
        let s = snap(&[("a", -10.0, -20.0, 5.0, 5.0), ("b", 0.0, 0.0, 100.0, 50.0)]);
        let bytes = s.write_bytes().expect("encode");
        let back = InitialLayoutSnapshot::read_bytes(&bytes).expect("decode");
        assert_eq!(back, s);
    }
}
