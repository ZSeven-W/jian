//! `aot/default_state.bin` — serialised StateGraph initial values
//! (Plan 19 Task 6 D1).
//!
//! ## Wire format (little-endian)
//!
//! ```text
//! [0..4]   magic        b"OPS1"           (4 bytes)
//! [4..6]   version      u16 = 1           (2 bytes)
//! [6..10]  payload_len  u32 (LE)          (4 bytes)
//! [10..N]  payload      JSON              (payload_len bytes)
//! ```
//!
//! The payload is canonical, deterministic JSON of a
//! [`DefaultStateSnapshot`] — six maps keyed by scope name (`app`,
//! `page`, `self`, `route`, `storage`, `vars`). `BTreeMap` (vs
//! `HashMap`) gives a stable iteration order so the on-disk bytes
//! are deterministic, important for content-addressed pack hashes
//! and diff-friendly CI fixtures.
//!
//! ## Why JSON inside a binary frame
//!
//! State values are arbitrary `serde_json::Value` (signals carry
//! JSON-shaped data — strings, numbers, booleans, nested objects /
//! arrays). The set of distinct types is exactly what JSON already
//! describes; a hand-rolled binary serialiser would re-implement
//! `serde_json` for no parse-cost win at the typical 5-key state
//! footprint. The binary frame around the JSON payload still
//! carries:
//!
//! - magic bytes (so a reader can detect "this is a state-bin, not
//!   any other 4-byte-prefixed file"),
//! - format version (so future wire-breaking changes refuse to
//!   silently misparse),
//! - explicit payload length (so a truncated tail is detected up
//!   front, before serde-json scans byte-by-byte for an unbalanced
//!   brace).
//!
//! ## Why `OPS1` and not `OPL1`
//!
//! `OPS1` is the format magic; the trailing digit signals a
//! breaking wire change. `OPL1` is reserved for the layout snapshot
//! (see [`super::initial_layout`]). Both share the same magic-prefix
//! framing convention so a future tool could route by sniffing the
//! first four bytes.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Magic header bytes identifying this format. A reader that doesn't
/// see these four bytes at offset 0 must reject the input.
pub const DEFAULT_STATE_MAGIC: [u8; 4] = *b"OPS1";

/// Current format version. Bump on a wire-breaking change. Additive
/// changes (a new optional scope) need a new magic instead so old
/// readers reject loudly rather than misparse.
pub const DEFAULT_STATE_VERSION: u16 = 1;

/// Errors the writer / reader can produce. Two-axis: structural
/// (wrong magic / truncated) vs content (payload not valid JSON,
/// payload_len overflows). Both are unrecoverable; the reader logs
/// and falls back to running `SeedStateGraph` from the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultStateError {
    /// Input shorter than the fixed header (10 bytes).
    TooShort {
        /// Actual byte count seen.
        got: usize,
        /// Minimum bytes required to read the header.
        need: usize,
    },
    /// Magic bytes don't match [`DEFAULT_STATE_MAGIC`]. Probably not
    /// a state snapshot at all — caller should fall back rather
    /// than retry.
    BadMagic { got: [u8; 4] },
    /// Version field doesn't match a supported version.
    UnsupportedVersion { got: u16 },
    /// `payload_len` declared more bytes than the buffer holds.
    PayloadTruncated {
        /// Bytes actually available after the header.
        available: usize,
        /// Bytes the header declared.
        declared: usize,
    },
    /// Reader saw bytes after the declared payload was consumed. A
    /// trailing-byte tolerance would mask zip-level corruption that
    /// happens to land at exactly the state-bin offset.
    TrailingBytes {
        /// Number of bytes left over.
        leftover: usize,
    },
    /// Payload bytes failed JSON parse. Cause string is the
    /// `serde_json::Error`'s `Display` form, since the error type
    /// itself isn't `Eq` and we want the variant comparable for
    /// tests.
    InvalidJson { cause: String },
    /// State value nests deeper than [`MAX_CANONICALIZE_DEPTH`].
    /// Codex round 2 MEDIUM: `write_bytes` accepts programmatically
    /// constructed snapshots that can bypass `serde_json`'s default
    /// 128-level parse guard, so the canonicaliser needs its own
    /// depth bound to prevent a stack-overflow blow-up. Codex round
    /// 3 LOW: the field carries the LIMIT, not the offending depth
    /// (which is always `limit + 1` by definition); renamed to
    /// `limit` so callers don't read it as the actual depth.
    DepthExceeded {
        /// The limit that was exceeded. Equal to
        /// [`MAX_CANONICALIZE_DEPTH`] for the writer; offending
        /// depth is `limit + 1` by definition.
        limit: usize,
    },
}

impl std::fmt::Display for DefaultStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort { got, need } => {
                write!(f, "input too short: got {got} bytes, need at least {need}")
            }
            Self::BadMagic { got } => write!(
                f,
                "bad magic: got {:02x?}, expected {:02x?}",
                got, DEFAULT_STATE_MAGIC
            ),
            Self::UnsupportedVersion { got } => {
                write!(f, "unsupported default-state version {got}")
            }
            Self::PayloadTruncated {
                available,
                declared,
            } => {
                write!(
                    f,
                    "payload truncated: header declared {declared} bytes, only {available} available"
                )
            }
            Self::TrailingBytes { leftover } => {
                write!(f, "trailing {leftover} byte(s) after declared payload")
            }
            Self::InvalidJson { cause } => write!(f, "invalid json payload: {cause}"),
            Self::DepthExceeded { limit } => {
                write!(f, "state value nests deeper than {limit} levels")
            }
        }
    }
}

/// Maximum nesting depth `canonicalize_value` will descend before
/// rejecting with [`DefaultStateError::DepthExceeded`]. Set well
/// below `serde_json`'s default 128-level parser guard so the
/// writer rejects pathological snapshots before the recursion blows
/// the host's thread stack. Real `.op` state values are typically
/// 1-3 levels deep; 64 is a generous ceiling.
pub const MAX_CANONICALIZE_DEPTH: usize = 64;

impl std::error::Error for DefaultStateError {}

/// Initial values for every StateGraph scope captured at pack time.
/// Empty maps are fine — a doc that declares no `state` block at all
/// rounds-trips as six empty maps, ~80 bytes once framed. The
/// runtime preload path simply seeds nothing in that case.
///
/// `BTreeMap` (vs `HashMap`) gives a stable iteration order so the
/// serialised bytes are deterministic — important for content-
/// addressed pack hashes and for diff-friendly CI fixtures.
///
/// ### Why six top-level maps, not one
///
/// `StateGraph` segregates `app` / `page` / `self` / `route` /
/// `storage` / `vars` into distinct `RefCell<BTreeMap>`s with
/// distinct lifecycles (`storage` syncs to a backend, `route` is
/// re-seeded on navigation). Mirroring the runtime's shape on disk
/// keeps the dump → restore round-trip a one-liner per scope and
/// avoids inventing a "default scope" that downstream readers
/// would have to remember to skip.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DefaultStateSnapshot {
    /// `$app.<name>` initial values. Most-common scope; the typical
    /// counter / form / shopping-cart `.op` populates only this map.
    #[serde(default)]
    pub app: BTreeMap<String, Value>,
    /// `$page.<name>` initial values, keyed by page id.
    #[serde(default)]
    pub page: BTreeMap<String, BTreeMap<String, Value>>,
    /// `$self.<name>` initial values, keyed by node id. Per-instance
    /// values that the runtime resolves under a `self` context (e.g.
    /// hover state on a button).
    ///
    /// Wire field name is `self_node` (not `self`) because `self` is
    /// reserved in JSON schema terms and downstream tooling that
    /// inspects the manifest may stumble on a literal `"self"` key.
    #[serde(default, rename = "self_node")]
    pub self_node: BTreeMap<String, BTreeMap<String, Value>>,
    /// `$route.<name>` initial values. Hosts that re-seed on
    /// navigation typically write nothing here — but a doc may pin
    /// route defaults for a fresh launch.
    #[serde(default)]
    pub route: BTreeMap<String, Value>,
    /// `$storage.<name>` initial values. `storage` syncs to the
    /// host's backend at runtime; the AOT snapshot is the
    /// pre-restore default so a first launch (no backend hit) still
    /// has the schema-declared values.
    #[serde(default)]
    pub storage: BTreeMap<String, Value>,
    /// `$vars.<name>` design-token values.
    #[serde(default)]
    pub vars: BTreeMap<String, Value>,
}

impl DefaultStateSnapshot {
    /// Serialise to the `aot/default_state.bin` wire format. See the
    /// module-level doc for the byte-by-byte layout.
    ///
    /// Nested `Value::Object` keys are recursively canonicalised
    /// before encode (codex round 1 MEDIUM): the workspace enables
    /// `serde_json`'s `preserve_order` feature, which makes
    /// `Value::Object` an `IndexMap` rather than a `BTreeMap`. Two
    /// otherwise-identical snapshots whose nested objects were
    /// constructed with different insertion order would otherwise
    /// produce different bytes — defeating the content-addressed
    /// pack-hash invariant.
    pub fn write_bytes(&self) -> Result<Vec<u8>, DefaultStateError> {
        let canonical = self.canonicalize()?;
        let payload = serde_json::to_vec(&canonical).map_err(|e| DefaultStateError::InvalidJson {
            cause: e.to_string(),
        })?;
        // Length must fit in u32 — saturating cast would silently
        // truncate, so explicitly reject. 4 GiB of state is far
        // beyond anything the runtime expects.
        let payload_len: u32 = payload.len().try_into().map_err(|_| {
            DefaultStateError::InvalidJson {
                cause: format!("payload {} bytes exceeds u32::MAX", payload.len()),
            }
        })?;
        let mut out = Vec::with_capacity(10 + payload.len());
        out.extend_from_slice(&DEFAULT_STATE_MAGIC);
        out.extend_from_slice(&DEFAULT_STATE_VERSION.to_le_bytes());
        out.extend_from_slice(&payload_len.to_le_bytes());
        out.extend_from_slice(&payload);
        Ok(out)
    }

    /// Deserialise from the wire format. Validates magic, version,
    /// payload length, and JSON structure; rejects with a typed
    /// error on any mismatch so the runtime can fall back to a
    /// fresh `SeedStateGraph` pass without misinterpreting bytes.
    pub fn read_bytes(buf: &[u8]) -> Result<Self, DefaultStateError> {
        const HEADER: usize = 10;
        if buf.len() < HEADER {
            return Err(DefaultStateError::TooShort {
                got: buf.len(),
                need: HEADER,
            });
        }
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&buf[0..4]);
        if magic != DEFAULT_STATE_MAGIC {
            return Err(DefaultStateError::BadMagic { got: magic });
        }
        let version = u16::from_le_bytes([buf[4], buf[5]]);
        if version != DEFAULT_STATE_VERSION {
            return Err(DefaultStateError::UnsupportedVersion { got: version });
        }
        let payload_len =
            u32::from_le_bytes([buf[6], buf[7], buf[8], buf[9]]) as usize;
        let body = &buf[HEADER..];
        if body.len() < payload_len {
            return Err(DefaultStateError::PayloadTruncated {
                available: body.len(),
                declared: payload_len,
            });
        }
        let payload = &body[..payload_len];
        let trailing = body.len() - payload_len;
        if trailing > 0 {
            return Err(DefaultStateError::TrailingBytes { leftover: trailing });
        }
        serde_json::from_slice::<DefaultStateSnapshot>(payload).map_err(|e| {
            DefaultStateError::InvalidJson {
                cause: e.to_string(),
            }
        })
    }

    /// True when every scope is empty. Hosts can short-circuit a
    /// "no-op restore" without entering the loop.
    pub fn is_empty(&self) -> bool {
        self.app.is_empty()
            && self.page.is_empty()
            && self.self_node.is_empty()
            && self.route.is_empty()
            && self.storage.is_empty()
            && self.vars.is_empty()
    }

    /// Return a copy with every nested `Value::Object` re-keyed in
    /// sorted order, recursively. Codex round 1 MEDIUM: necessary
    /// because the workspace pulls `serde_json` with `preserve_order`,
    /// making `Value::Object` an insertion-ordered `IndexMap` rather
    /// than a `BTreeMap`. The top-level scope `BTreeMap`s already
    /// iterate sorted, but inner objects (e.g. `{"user":{"name":...,
    /// "age":...}}`) need explicit canonicalisation.
    ///
    /// Returns [`DefaultStateError::DepthExceeded`] when a value
    /// nests deeper than [`MAX_CANONICALIZE_DEPTH`] (codex round 2
    /// MEDIUM): the writer accepts programmatically constructed
    /// snapshots, which can bypass `serde_json`'s default 128-level
    /// parser guard.
    fn canonicalize(&self) -> Result<Self, DefaultStateError> {
        Ok(Self {
            app: canonicalize_scope_map(&self.app)?,
            page: canonicalize_nested_scope_map(&self.page)?,
            self_node: canonicalize_nested_scope_map(&self.self_node)?,
            route: canonicalize_scope_map(&self.route)?,
            storage: canonicalize_scope_map(&self.storage)?,
            vars: canonicalize_scope_map(&self.vars)?,
        })
    }
}

/// Walk a scope's `BTreeMap<String, Value>` and canonicalise each
/// value's nested objects. The scope-map iteration itself is not
/// counted toward [`MAX_CANONICALIZE_DEPTH`] — the depth limit
/// guards user-supplied JSON nesting only, so it's uniform across
/// scopes regardless of whether the scope happens to be flat
/// (`app` / `route` / `storage` / `vars`) or nested-keyed (`page`,
/// `self_node`).
fn canonicalize_scope_map(
    m: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, DefaultStateError> {
    let mut out = BTreeMap::new();
    for (k, v) in m {
        // Each top-level scope value enters canonicalize_value at
        // depth=1, so MAX_CANONICALIZE_DEPTH=N means "N levels of
        // value nesting allowed" (codex round 3 HIGH: scope-uniform).
        out.insert(k.clone(), canonicalize_value(v, 1)?);
    }
    Ok(out)
}

fn canonicalize_nested_scope_map(
    m: &BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<BTreeMap<String, BTreeMap<String, Value>>, DefaultStateError> {
    let mut out = BTreeMap::new();
    for (k, inner) in m {
        out.insert(k.clone(), canonicalize_scope_map(inner)?);
    }
    Ok(out)
}

/// Recursively rewrite `Value::Object` so its keys iterate in sorted
/// order. `serde_json::Value` is an `IndexMap` under `preserve_order`;
/// we materialise the sorted view through a `BTreeMap` round-trip.
///
/// `depth` counts user-supplied value-nesting levels (1 == the top-
/// level value sitting under a scope key; deeper means recursion
/// through Object members or Array elements). Returns
/// [`DefaultStateError::DepthExceeded`] when `depth >
/// MAX_CANONICALIZE_DEPTH` so a pathological programmatic snapshot
/// can't blow the host's thread stack before serde-json would
/// itself reject. A primitive at exactly the depth limit
/// round-trips; the offending value is at limit + 1.
fn canonicalize_value(v: &Value, depth: usize) -> Result<Value, DefaultStateError> {
    if depth > MAX_CANONICALIZE_DEPTH {
        return Err(DefaultStateError::DepthExceeded {
            limit: MAX_CANONICALIZE_DEPTH,
        });
    }
    match v {
        Value::Object(map) => {
            // Pre-sort via BTreeMap so the output `serde_json::Map`
            // is populated in lexicographic order. `serde_json::Map`
            // under `preserve_order` is `IndexMap`, which honours
            // insertion order on serialisation — so sorted-insert →
            // sorted-output.
            let mut sorted = BTreeMap::new();
            for (k, inner) in map.iter() {
                sorted.insert(k.clone(), canonicalize_value(inner, depth + 1)?);
            }
            let mut out = serde_json::Map::with_capacity(sorted.len());
            for (k, inner) in sorted {
                out.insert(k, inner);
            }
            Ok(Value::Object(out))
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for inner in items {
                out.push(canonicalize_value(inner, depth + 1)?);
            }
            Ok(Value::Array(out))
        }
        // Primitives are already canonical.
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(v.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_snapshot() -> DefaultStateSnapshot {
        let mut snap = DefaultStateSnapshot::default();
        snap.app.insert("count".into(), json!(0));
        snap.app
            .insert("user".into(), json!({"name": "Alice", "age": 30}));
        let mut page_home = BTreeMap::new();
        page_home.insert("scrollTop".into(), json!(0.0));
        snap.page.insert("home".into(), page_home);
        let mut self_btn = BTreeMap::new();
        self_btn.insert("hover".into(), json!(false));
        snap.self_node.insert("btn".into(), self_btn);
        snap.route.insert("path".into(), json!("/"));
        snap.storage.insert("theme".into(), json!("dark"));
        snap.vars
            .insert("primary".into(), json!("#3b82f6"));
        snap
    }

    #[test]
    fn round_trip_full() {
        let original = sample_snapshot();
        let bytes = original.write_bytes().expect("encode");
        let back = DefaultStateSnapshot::read_bytes(&bytes).expect("decode");
        assert_eq!(original, back);
    }

    #[test]
    fn round_trip_empty() {
        let snap = DefaultStateSnapshot::default();
        assert!(snap.is_empty());
        let bytes = snap.write_bytes().expect("encode");
        let back = DefaultStateSnapshot::read_bytes(&bytes).expect("decode");
        assert!(back.is_empty());
        assert_eq!(snap, back);
    }

    #[test]
    fn deterministic_byte_order() {
        // Two snapshots constructed with insert-order swapped must
        // produce byte-identical output (BTreeMap iteration is
        // sorted by key).
        let mut a = DefaultStateSnapshot::default();
        a.app.insert("z".into(), json!(1));
        a.app.insert("a".into(), json!(2));
        let mut b = DefaultStateSnapshot::default();
        b.app.insert("a".into(), json!(2));
        b.app.insert("z".into(), json!(1));
        assert_eq!(a.write_bytes().unwrap(), b.write_bytes().unwrap());
    }

    #[test]
    fn deterministic_byte_order_with_nested_object_keys() {
        // Codex round 1 MEDIUM: `serde_json` is on with
        // `preserve_order`, so `Value::Object` is insertion-ordered.
        // Two snapshots whose nested objects were built with reversed
        // key order must still produce byte-identical output —
        // `canonicalize` recursively sorts.
        let mut obj_az = serde_json::Map::new();
        obj_az.insert("a".into(), json!(1));
        obj_az.insert("z".into(), json!(2));
        let mut obj_za = serde_json::Map::new();
        obj_za.insert("z".into(), json!(2));
        obj_za.insert("a".into(), json!(1));

        let mut a = DefaultStateSnapshot::default();
        a.app.insert("user".into(), Value::Object(obj_az));
        let mut b = DefaultStateSnapshot::default();
        b.app.insert("user".into(), Value::Object(obj_za));
        assert_eq!(
            a.write_bytes().unwrap(),
            b.write_bytes().unwrap(),
            "nested object keys must canonicalise"
        );
    }

    #[test]
    fn canonicalize_recurses_into_arrays_of_objects() {
        // Arrays carry their own iteration order (insertion order
        // matters for arrays — that's not what we canonicalise), but
        // an array OF objects must still see each inner object's
        // keys sorted.
        let mut item_az = serde_json::Map::new();
        item_az.insert("a".into(), json!(1));
        item_az.insert("z".into(), json!(2));
        let mut item_za = serde_json::Map::new();
        item_za.insert("z".into(), json!(2));
        item_za.insert("a".into(), json!(1));

        let mut a = DefaultStateSnapshot::default();
        a.app
            .insert("items".into(), Value::Array(vec![Value::Object(item_az)]));
        let mut b = DefaultStateSnapshot::default();
        b.app
            .insert("items".into(), Value::Array(vec![Value::Object(item_za)]));
        assert_eq!(a.write_bytes().unwrap(), b.write_bytes().unwrap());
    }

    #[test]
    fn rejects_too_short() {
        let err = DefaultStateSnapshot::read_bytes(&[]).unwrap_err();
        assert!(matches!(err, DefaultStateError::TooShort { .. }));
        let err = DefaultStateSnapshot::read_bytes(&[1, 2, 3]).unwrap_err();
        assert!(matches!(err, DefaultStateError::TooShort { .. }));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = sample_snapshot().write_bytes().unwrap();
        bytes[0] = b'X';
        let err = DefaultStateSnapshot::read_bytes(&bytes).unwrap_err();
        assert!(matches!(err, DefaultStateError::BadMagic { .. }));
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut bytes = sample_snapshot().write_bytes().unwrap();
        bytes[4] = 99;
        bytes[5] = 0;
        let err = DefaultStateSnapshot::read_bytes(&bytes).unwrap_err();
        assert!(matches!(
            err,
            DefaultStateError::UnsupportedVersion { got: 99 }
        ));
    }

    #[test]
    fn rejects_truncated_payload() {
        let bytes = sample_snapshot().write_bytes().unwrap();
        // Drop the last byte of the JSON payload.
        let truncated = &bytes[..bytes.len() - 1];
        let err = DefaultStateSnapshot::read_bytes(truncated).unwrap_err();
        assert!(matches!(
            err,
            DefaultStateError::PayloadTruncated { .. } | DefaultStateError::InvalidJson { .. }
        ));
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut bytes = sample_snapshot().write_bytes().unwrap();
        // Lie in the header — claim the payload is one byte shorter
        // than it actually is, leaving a "trailing" byte in the buf.
        let actual_payload_len = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]);
        let lie = (actual_payload_len - 1).to_le_bytes();
        bytes[6] = lie[0];
        bytes[7] = lie[1];
        bytes[8] = lie[2];
        bytes[9] = lie[3];
        let err = DefaultStateSnapshot::read_bytes(&bytes).unwrap_err();
        assert!(matches!(err, DefaultStateError::TrailingBytes { .. }));
    }

    #[test]
    fn rejects_invalid_json() {
        // Hand-build a frame whose payload is garbage.
        let payload = b"not json at all";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&DEFAULT_STATE_MAGIC);
        bytes.extend_from_slice(&DEFAULT_STATE_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        let err = DefaultStateSnapshot::read_bytes(&bytes).unwrap_err();
        assert!(matches!(err, DefaultStateError::InvalidJson { .. }));
    }

    #[test]
    fn wire_constants_pinned() {
        assert_eq!(DEFAULT_STATE_MAGIC, *b"OPS1");
        assert_eq!(DEFAULT_STATE_VERSION, 1);
    }

    #[test]
    fn canonicalize_rejects_pathological_nesting() {
        // Codex round 2 MEDIUM: a programmatically constructed
        // snapshot with > MAX_CANONICALIZE_DEPTH levels of nesting
        // must produce a typed error rather than blow the stack
        // during canonicalisation.
        let mut value = Value::Null;
        // Build MAX_CANONICALIZE_DEPTH + 5 levels of array nesting,
        // safely past the limit but well under what serde_json's
        // own 128-level parser guard would catch.
        for _ in 0..(MAX_CANONICALIZE_DEPTH + 5) {
            value = Value::Array(vec![value]);
        }
        let mut snap = DefaultStateSnapshot::default();
        snap.app.insert("deep".into(), value);
        let err = snap.write_bytes().unwrap_err();
        assert!(
            matches!(err, DefaultStateError::DepthExceeded { .. }),
            "expected DepthExceeded, got {err:?}"
        );
    }

    #[test]
    fn canonicalize_accepts_realistic_depth() {
        // 3-level nesting (object containing array containing object)
        // is well within the limit and round-trips cleanly.
        let mut snap = DefaultStateSnapshot::default();
        snap.app.insert(
            "user".into(),
            json!({
                "items": [
                    {"name": "a"},
                    {"name": "b"}
                ]
            }),
        );
        let bytes = snap.write_bytes().expect("realistic depth ok");
        let back = DefaultStateSnapshot::read_bytes(&bytes).expect("decode");
        assert_eq!(back, snap);
    }

    #[test]
    fn canonicalize_depth_is_scope_uniform() {
        // Codex round 3 HIGH: previous off-by-one allowed `app` 62
        // levels but `page`/`self_node` only 61. Now every scope
        // accepts EXACTLY MAX_CANONICALIZE_DEPTH levels of value
        // nesting. We probe with a value that's right at the limit
        // (MAX levels) and one that's one over (MAX+1 levels), and
        // assert both flat-scope (`app`) and nested-scope (`page`)
        // see the same accept/reject boundary.
        fn nest(n: usize) -> Value {
            let mut v = Value::Null;
            for _ in 0..n {
                v = Value::Array(vec![v]);
            }
            v
        }

        // `nest(N)` wraps the inner Null in N arrays. The Null
        // primitive then sits at depth = 1 (entered as the scope
        // value) + N (each array adds one wrapper). MAX-1 wrappers
        // place the primitive at exactly MAX; MAX wrappers push it
        // to MAX+1 — the rejection boundary.
        let at_limit = nest(MAX_CANONICALIZE_DEPTH - 1);
        let over_limit = nest(MAX_CANONICALIZE_DEPTH);

        // app scope (flat).
        let mut a_ok = DefaultStateSnapshot::default();
        a_ok.app.insert("k".into(), at_limit.clone());
        a_ok.write_bytes().expect("app at limit ok");

        let mut a_bad = DefaultStateSnapshot::default();
        a_bad.app.insert("k".into(), over_limit.clone());
        let err = a_bad.write_bytes().unwrap_err();
        assert!(matches!(err, DefaultStateError::DepthExceeded { .. }));

        // page scope (nested).
        let mut p_ok = DefaultStateSnapshot::default();
        p_ok.page
            .entry("home".into())
            .or_default()
            .insert("k".into(), at_limit);
        p_ok.write_bytes().expect("page at limit ok");

        let mut p_bad = DefaultStateSnapshot::default();
        p_bad.page
            .entry("home".into())
            .or_default()
            .insert("k".into(), over_limit);
        let err = p_bad.write_bytes().unwrap_err();
        assert!(matches!(err, DefaultStateError::DepthExceeded { .. }));
    }
}
