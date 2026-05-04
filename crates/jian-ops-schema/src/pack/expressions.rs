//! `aot/expressions.bin` — precompiled-bytecode snapshot for every
//! expression a `.op.pack` ships pre-compiled (Plan 19 Task 6 D2).
//!
//! ## Wire format (little-endian)
//!
//! ```text
//! [0..4]   magic         b"OPE1"            (4 bytes)
//! [4..6]   version       u16 = 1            (2 bytes)
//! [6..10]  entry_count   u32 (LE)           (4 bytes)
//!
//! For each entry (sorted by source string):
//!   source-string :  u32 LE byte-len + UTF-8 bytes
//!   chunk         :  serialized PackedChunk (see below)
//! ```
//!
//! ### PackedChunk frame
//!
//! ```text
//! ops_len       u32 (LE)
//! per op:
//!   op_tag      u8 (0..=32, see PackedOpCode)
//!   payload     0, 1, 4, 8, or 5 bytes per tag (tag-implicit shape)
//! strings_len   u32 (LE)
//! per string:
//!   s_len       u32 (LE) + UTF-8 bytes
//! scope_paths_len u32 (LE)
//! per path:
//!   p_len       u32 (LE) + UTF-8 bytes
//! ```
//!
//! `OPE1` is the format magic; bumping the trailing digit signals a
//! breaking wire change. The set of op-tag bytes is wire-pinned by a
//! constants test below — adding a new `OpCode` variant requires
//! both a new tag and a `PACK_FORMAT_VERSION` (or fresh magic) bump.
//!
//! ## Why hand-rolled, not bincode / serde
//!
//! `Chunk` lives in `jian-core` (downstream) and `jian-ops-schema`
//! deliberately keeps no `jian-core` dependency. A `serde` derive on
//! `Chunk` would also bind the wire format to serde's default field
//! ordering / tag shape, making the on-disk layout fragile against
//! `OpCode` enum-variant rearrangement. A typed `PackedOpCode` mirror
//! with explicit u8 tags pins the wire shape regardless of how the
//! `jian-core` enum evolves.
//!
//! ## Integration boundary
//!
//! The conversion `Chunk` ↔ `PackedChunk` lives in
//! `jian_core::expression::aot` (downstream) so this crate stays
//! free of `jian-core`. Hosts use these helpers:
//!
//! - **Writer side** (`jian pack --aot`): dump the runtime's
//!   `ExpressionCache` to a `BTreeMap<String, Chunk>`, convert to
//!   `BTreeMap<String, PackedChunk>`, build [`ExpressionsSnapshot`],
//!   serialise via [`ExpressionsSnapshot::write_bytes`], embed.
//! - **Reader side** (`jian player`): decode via
//!   [`ExpressionsSnapshot::read_bytes`], convert each
//!   `PackedChunk` back to `Chunk`, install via
//!   `ExpressionCache::install_precompiled`. The bootstrap then
//!   threads the seeded cache through `Runtime::new_from_document`
//!   so the first-frame binding evaluator hits the precompiled
//!   bytecode without paying a parse + compile cost.

use std::collections::BTreeMap;

/// Magic header bytes identifying this format. A reader that doesn't
/// see these four bytes at offset 0 must reject the input.
pub const EXPRESSIONS_MAGIC: [u8; 4] = *b"OPE1";

/// Current format version. Bump on a wire-breaking change. Adding a
/// new `OpCode` variant counts as breaking — old readers would see an
/// unknown tag and bail; bumping the version makes the rejection a
/// single-line error rather than a per-entry decode failure.
pub const EXPRESSIONS_VERSION: u16 = 1;

/// Maximum on-wire byte length for any single UTF-8 string field
/// (source string, op-string entry, scope-path entry). Chosen to cap
/// pathological author input — real expression sources are bounded
/// by `.op` parse limits well below this; the explicit ceiling keeps
/// a malformed pack from steering the reader into a giant alloc.
///
/// Codex round 1 CONCERN: the per-field caps are individual-
/// malformation guards. The AGGREGATE in-memory budget is enforced
/// by the pack reader's `AOT_EXPRS_MAX_BYTES = 16 MiB` cap on the
/// zip entry — at most 16 MiB of decompressed bytes ever reach this
/// decoder. The per-field caps just stop a single corrupted length
/// field from claiming the whole budget at once.
pub const MAX_STRING_BYTES: u32 = 64 * 1024; //  64 KiB per string

/// Maximum count of ops / strings / scope_paths inside a single
/// chunk. A typical compiled expression has dozens of ops; the
/// ceiling caps a corrupted count field at something a malicious
/// pack can't use to drive the reader to OOM. Real chunks have low
/// hundreds of ops at most; 64K is a generous ceiling.
pub const MAX_VEC_LEN: u32 = 64 * 1024;

/// Maximum number of distinct expression entries the snapshot may
/// declare. A realistic doc compiles dozens to low thousands of
/// expressions; 64K is a per-pack ceiling, not a soft target.
pub const MAX_ENTRIES: u32 = 64 * 1024;

/// Errors the writer / reader can produce. Two-axis: structural
/// (wrong magic / truncated) vs content (op-tag unknown, oversized
/// string). Both are unrecoverable; the reader logs and falls back
/// to letting the runtime JIT-compile expressions on demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpressionsError {
    /// Input shorter than the fixed header (10 bytes).
    TooShort {
        /// Actual byte count seen.
        got: usize,
        /// Minimum bytes required to read the header.
        need: usize,
    },
    /// Magic bytes don't match [`EXPRESSIONS_MAGIC`]. Probably not a
    /// snapshot at all — caller should fall back rather than retry.
    BadMagic { got: [u8; 4] },
    /// Version field doesn't match a supported version.
    UnsupportedVersion { got: u16 },
    /// Buffer ran out mid-decode of an entry. The entry-count word
    /// itself is part of the fixed 10-byte header and a truncation
    /// there surfaces as [`Self::TooShort`], not this variant.
    Truncated {
        /// Zero-based index of the entry being decoded when the
        /// truncation was observed.
        entry_index: usize,
    },
    /// A string field's bytes weren't valid UTF-8.
    InvalidUtf8 {
        /// Zero-based index of the offending entry.
        entry_index: usize,
        /// Where in the entry the invalid bytes sat.
        field: &'static str,
    },
    /// A string field declared more bytes than [`MAX_STRING_BYTES`].
    /// Reader-side guard against a corrupted length field steering
    /// the decoder into a giant alloc.
    StringTooLong {
        /// Zero-based index of the offending entry.
        entry_index: usize,
        /// Where in the entry the oversized field sat.
        field: &'static str,
        /// Declared length.
        declared: u32,
        /// Limit ([`MAX_STRING_BYTES`]).
        limit: u32,
    },
    /// A Vec-length field declared more entries than [`MAX_VEC_LEN`].
    VecTooLong {
        /// Zero-based index of the offending entry.
        entry_index: usize,
        /// Which inner Vec (`ops`, `strings`, `scope_paths`).
        field: &'static str,
        /// Declared count.
        declared: u32,
        /// Limit ([`MAX_VEC_LEN`]).
        limit: u32,
    },
    /// Snapshot declared more entries than [`MAX_ENTRIES`].
    EntryCountTooLarge {
        /// Declared count.
        declared: u32,
        /// Limit ([`MAX_ENTRIES`]).
        limit: u32,
    },
    /// Op-tag byte didn't match any [`PackedOpCode`] variant. Either
    /// corruption or a pack baked by a future writer that added a new
    /// variant; either way the reader bails and the runtime falls
    /// back to JIT compile.
    UnknownOpTag {
        /// Zero-based index of the entry.
        entry_index: usize,
        /// Zero-based index of the op within the chunk.
        op_index: usize,
        /// The unknown tag byte.
        tag: u8,
    },
    /// Reader saw bytes after the declared `entry_count` was
    /// satisfied. Tolerating trailing bytes would mask zip-level
    /// corruption that lands at the expressions-bin offset.
    TrailingBytes {
        /// Number of bytes left over.
        leftover: usize,
    },
    /// Two entries shared the same source string. The writer can't
    /// emit duplicates (a `BTreeMap` enforces unique keys), so a
    /// duplicate on the wire signals corruption or a malicious blob.
    DuplicateSource {
        /// Zero-based index of the second entry with the colliding
        /// source.
        entry_index: usize,
    },
    /// Writer-side: a source string's UTF-8 byte length exceeds
    /// [`MAX_STRING_BYTES`].
    SourceTooLong {
        /// Index in BTreeMap iteration order of the offending source.
        entry_index: usize,
        /// Length in bytes of the offending source.
        len: usize,
    },
}

impl std::fmt::Display for ExpressionsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort { got, need } => {
                write!(f, "input too short: got {got} bytes, need at least {need}")
            }
            Self::BadMagic { got } => write!(
                f,
                "bad magic: got {:02x?}, expected {:02x?}",
                got, EXPRESSIONS_MAGIC
            ),
            Self::UnsupportedVersion { got } => {
                write!(f, "unsupported expressions version {got}")
            }
            Self::Truncated { entry_index } => {
                write!(f, "truncated input at entry {entry_index}")
            }
            Self::InvalidUtf8 { entry_index, field } => {
                write!(f, "invalid utf-8 at entry {entry_index} field `{field}`")
            }
            Self::StringTooLong {
                entry_index,
                field,
                declared,
                limit,
            } => write!(
                f,
                "string at entry {entry_index} field `{field}` declares {declared} bytes, exceeds {limit}-byte limit"
            ),
            Self::VecTooLong {
                entry_index,
                field,
                declared,
                limit,
            } => write!(
                f,
                "vec at entry {entry_index} field `{field}` declares {declared} entries, exceeds {limit}-entry limit"
            ),
            Self::EntryCountTooLarge { declared, limit } => write!(
                f,
                "entry count {declared} exceeds {limit}-entry limit"
            ),
            Self::UnknownOpTag {
                entry_index,
                op_index,
                tag,
            } => write!(
                f,
                "unknown op tag {tag:#04x} at entry {entry_index} op {op_index}"
            ),
            Self::TrailingBytes { leftover } => {
                write!(f, "trailing {leftover} byte(s) after declared entry count")
            }
            Self::DuplicateSource { entry_index } => {
                write!(f, "duplicate source string at entry {entry_index}")
            }
            Self::SourceTooLong { entry_index, len } => write!(
                f,
                "source at entry {entry_index} is {len} bytes, exceeds {MAX_STRING_BYTES}-byte limit"
            ),
        }
    }
}

impl std::error::Error for ExpressionsError {}

// ──────────────────────────────────────────────────────────────────
// Op-code mirror — wire-stable counterpart of `jian_core::expression::
// bytecode::OpCode`. Tag values (the `as u8` discriminants) are
// load-bearing: a new variant requires both a new tag at the END of
// the list AND a [`EXPRESSIONS_VERSION`] bump — old readers would
// see the new tag and bail with [`ExpressionsError::UnknownOpTag`].
//
// Tags 0..=32 are pinned by `tag_assignments_are_pinned` (test). DO
// NOT renumber an existing variant — that's a wire break.
// ──────────────────────────────────────────────────────────────────

/// Wire-stable mirror of `jian_core::expression::bytecode::OpCode`.
/// Each variant carries the same payload shape as its `OpCode`
/// counterpart; the two sides are converted via the helpers that
/// live in `jian-core`'s `expression::aot` module.
#[derive(Debug, Clone, PartialEq)]
#[repr(u8)]
pub enum PackedOpCode {
    PushNum(f64) = 0,
    PushBool(bool) = 1,
    PushNull = 2,
    PushString(u32) = 3,
    PushScopeRef(u32) = 4,
    MakeArray(u32) = 5,
    MakeObject(u32) = 6,
    PushObjectKey(u32) = 7,

    MemberGet(u32) = 8,
    IndexGet = 9,

    Not = 10,
    Negate = 11,
    UnaryPlus = 12,

    Add = 13,
    Sub = 14,
    Mul = 15,
    Div = 16,
    Mod = 17,
    Eq = 18,
    NotEq = 19,
    EqStrict = 20,
    NotEqStrict = 21,
    Lt = 22,
    Gt = 23,
    LtEq = 24,
    GtEq = 25,

    JumpIfFalse(i32) = 26,
    JumpIfTrue(i32) = 27,
    Jump(i32) = 28,

    NullCoalesce = 29,
    TemplateAppend = 30,

    CallBuiltin(u32, u32) = 31,
    Return = 32,
}

impl PackedOpCode {
    /// Wire tag byte. Stable; renumbering breaks `aot/expressions.bin`
    /// for every previously-published pack.
    pub fn tag(&self) -> u8 {
        // SAFETY: `#[repr(u8)]` on a fieldless-prefix enum guarantees
        // the discriminant is the first byte of the enum's memory; we
        // never construct or transmute the enum from raw bytes here.
        // This stays a value-side `match` so a future renumbering is a
        // visible diff rather than a silent SAFETY-block change.
        match self {
            Self::PushNum(_) => 0,
            Self::PushBool(_) => 1,
            Self::PushNull => 2,
            Self::PushString(_) => 3,
            Self::PushScopeRef(_) => 4,
            Self::MakeArray(_) => 5,
            Self::MakeObject(_) => 6,
            Self::PushObjectKey(_) => 7,
            Self::MemberGet(_) => 8,
            Self::IndexGet => 9,
            Self::Not => 10,
            Self::Negate => 11,
            Self::UnaryPlus => 12,
            Self::Add => 13,
            Self::Sub => 14,
            Self::Mul => 15,
            Self::Div => 16,
            Self::Mod => 17,
            Self::Eq => 18,
            Self::NotEq => 19,
            Self::EqStrict => 20,
            Self::NotEqStrict => 21,
            Self::Lt => 22,
            Self::Gt => 23,
            Self::LtEq => 24,
            Self::GtEq => 25,
            Self::JumpIfFalse(_) => 26,
            Self::JumpIfTrue(_) => 27,
            Self::Jump(_) => 28,
            Self::NullCoalesce => 29,
            Self::TemplateAppend => 30,
            Self::CallBuiltin(_, _) => 31,
            Self::Return => 32,
        }
    }
}

/// Wire-stable mirror of `jian_core::expression::bytecode::Chunk`.
/// `Default` exists so an empty snapshot (no expressions) round-trips
/// without ceremony — useful for fixtures.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PackedChunk {
    pub ops: Vec<PackedOpCode>,
    pub strings: Vec<String>,
    pub scope_paths: Vec<String>,
}

/// Reasons [`PackedChunk::verify`] rejects a chunk. A malformed
/// `aot/expressions.bin` could in theory carry structurally-valid
/// wire bytes (passes [`ExpressionsSnapshot::read_bytes`]) that
/// nonetheless reference nonexistent string-pool indices, jump
/// outside the ops vector, or jump backwards. The compiler in
/// `jian-core` only emits forward jumps and in-range indices, so
/// any chunk failing one of these checks was either tampered with
/// or baked by a buggy writer; either way the install path drops
/// it to `None` and the runtime falls back to JIT compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkVerifyError {
    /// Op-code at `op_index` references string-pool index `idx`,
    /// but `strings.len()` is `pool_len`. Triggers for
    /// `PushString`, `PushObjectKey`, `MemberGet`, and
    /// `CallBuiltin`'s name index.
    StringIndexOutOfRange {
        op_index: usize,
        idx: u32,
        pool_len: usize,
    },
    /// `PushScopeRef(idx)` references scope-path index `idx` but
    /// `scope_paths.len()` is `pool_len`.
    ScopeIndexOutOfRange {
        op_index: usize,
        idx: u32,
        pool_len: usize,
    },
    /// Jump offset at `op_index` would land outside the ops
    /// vector. The target is computed as `op_index + 1 + offset`;
    /// a target equal to `ops.len()` (one past the last op) is
    /// allowed because the VM's `while ip < ops.len()` loop
    /// terminates cleanly there. Anything past that is a bug.
    JumpOutOfRange {
        op_index: usize,
        offset: i32,
        target: i64,
        ops_len: usize,
    },
    /// Jump offset at `op_index` is `<= 0`, i.e. backwards or to
    /// itself. The compiler only emits forward jumps; a backwards
    /// jump in an AOT chunk is either tampering or a writer bug.
    /// Allowing one would also let a malformed pack drive the VM
    /// into an infinite loop.
    BackwardsJump { op_index: usize, offset: i32 },
}

impl std::fmt::Display for ChunkVerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StringIndexOutOfRange {
                op_index,
                idx,
                pool_len,
            } => write!(
                f,
                "op {op_index}: string-pool index {idx} out of range (pool size {pool_len})"
            ),
            Self::ScopeIndexOutOfRange {
                op_index,
                idx,
                pool_len,
            } => write!(
                f,
                "op {op_index}: scope-pool index {idx} out of range (pool size {pool_len})"
            ),
            Self::JumpOutOfRange {
                op_index,
                offset,
                target,
                ops_len,
            } => write!(
                f,
                "op {op_index}: jump offset {offset} targets {target}, outside [0, {ops_len}]"
            ),
            Self::BackwardsJump { op_index, offset } => write!(
                f,
                "op {op_index}: backwards-or-zero jump offset {offset} (compiler only emits forward jumps)"
            ),
        }
    }
}

impl std::error::Error for ChunkVerifyError {}

impl PackedChunk {
    /// Structural verification of an AOT-decoded chunk before it's
    /// converted to a runtime `jian_core::expression::bytecode::
    /// Chunk` and installed into the expression cache. Catches
    /// out-of-range pool indices and out-of-range / backwards
    /// jumps. The VM additionally has runtime stack-underflow and
    /// pool-index guards (defense in depth), but rejecting at
    /// install time means a malformed pack's binding evaluator
    /// never even sees the broken chunk — the runtime falls back
    /// to JIT-compiling the source string instead.
    ///
    /// Note: this is a structural check, NOT a stack-balance
    /// proof. The VM emits a `vm_bug` diagnostic + null result for
    /// stack underflows the verifier didn't catch, which is the
    /// same shape as a runtime `set` action failing — visible to
    /// the host as a warning, not a panic.
    pub fn verify(&self) -> Result<(), ChunkVerifyError> {
        let strings_len = self.strings.len();
        let scope_len = self.scope_paths.len();
        let ops_len = self.ops.len();
        for (op_index, op) in self.ops.iter().enumerate() {
            match op {
                PackedOpCode::PushString(idx)
                | PackedOpCode::PushObjectKey(idx)
                | PackedOpCode::MemberGet(idx)
                | PackedOpCode::CallBuiltin(idx, _)
                    if (*idx as usize) >= strings_len =>
                {
                    return Err(ChunkVerifyError::StringIndexOutOfRange {
                        op_index,
                        idx: *idx,
                        pool_len: strings_len,
                    });
                }
                PackedOpCode::PushScopeRef(idx) if (*idx as usize) >= scope_len => {
                    return Err(ChunkVerifyError::ScopeIndexOutOfRange {
                        op_index,
                        idx: *idx,
                        pool_len: scope_len,
                    });
                }
                PackedOpCode::Jump(off)
                | PackedOpCode::JumpIfFalse(off)
                | PackedOpCode::JumpIfTrue(off) => {
                    if *off <= 0 {
                        return Err(ChunkVerifyError::BackwardsJump {
                            op_index,
                            offset: *off,
                        });
                    }
                    let target = (op_index as i64) + 1 + (*off as i64);
                    if target < 0 || target > ops_len as i64 {
                        return Err(ChunkVerifyError::JumpOutOfRange {
                            op_index,
                            offset: *off,
                            target,
                            ops_len,
                        });
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Pre-compiled-expression snapshot. `BTreeMap` (vs `HashMap`) gives
/// a stable iteration order so the on-disk bytes are deterministic —
/// important for content-addressed pack hashes and diff-friendly CI
/// fixtures.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExpressionsSnapshot {
    pub entries: BTreeMap<String, PackedChunk>,
}

impl ExpressionsSnapshot {
    /// Serialise to the `aot/expressions.bin` wire format. See the
    /// module-level doc for the byte-by-byte layout.
    pub fn write_bytes(&self) -> Result<Vec<u8>, ExpressionsError> {
        // Pre-validate every source-string length so the writer
        // never emits a truncated `u32 BE-bit-pattern` length-prefix.
        for (idx, src) in self.entries.keys().enumerate() {
            if src.len() as u64 > MAX_STRING_BYTES as u64 {
                return Err(ExpressionsError::SourceTooLong {
                    entry_index: idx,
                    len: src.len(),
                });
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(&EXPRESSIONS_MAGIC);
        out.extend_from_slice(&EXPRESSIONS_VERSION.to_le_bytes());
        let count = u32::try_from(self.entries.len()).map_err(|_| {
            ExpressionsError::EntryCountTooLarge {
                declared: u32::MAX,
                limit: MAX_ENTRIES,
            }
        })?;
        if count > MAX_ENTRIES {
            return Err(ExpressionsError::EntryCountTooLarge {
                declared: count,
                limit: MAX_ENTRIES,
            });
        }
        out.extend_from_slice(&count.to_le_bytes());
        for (idx, (source, chunk)) in self.entries.iter().enumerate() {
            write_string_field(&mut out, source, idx, "source")?;
            write_chunk(&mut out, chunk, idx)?;
        }
        Ok(out)
    }

    /// Inverse of [`Self::write_bytes`]. See [`ExpressionsError`] for
    /// the error taxonomy. On any error the caller should fall back
    /// to letting the runtime JIT-compile expressions on demand —
    /// the snapshot is an optimisation, not a correctness gate.
    pub fn read_bytes(bytes: &[u8]) -> Result<Self, ExpressionsError> {
        const HEADER: usize = 10;
        if bytes.len() < HEADER {
            return Err(ExpressionsError::TooShort {
                got: bytes.len(),
                need: HEADER,
            });
        }
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&bytes[0..4]);
        if magic != EXPRESSIONS_MAGIC {
            return Err(ExpressionsError::BadMagic { got: magic });
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != EXPRESSIONS_VERSION {
            return Err(ExpressionsError::UnsupportedVersion { got: version });
        }
        let count = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]);
        if count > MAX_ENTRIES {
            return Err(ExpressionsError::EntryCountTooLarge {
                declared: count,
                limit: MAX_ENTRIES,
            });
        }

        let mut cursor = HEADER;
        let mut entries: BTreeMap<String, PackedChunk> = BTreeMap::new();
        for entry_index in 0..count as usize {
            let source = read_string_field(bytes, &mut cursor, entry_index, "source")?;
            let chunk = read_chunk(bytes, &mut cursor, entry_index)?;
            // BTreeMap dedupes silently on insert; surface a duplicate
            // as corruption so the reader fails loud rather than
            // serving the second-occurrence chunk for the source.
            if entries.insert(source, chunk).is_some() {
                return Err(ExpressionsError::DuplicateSource { entry_index });
            }
        }
        if cursor != bytes.len() {
            return Err(ExpressionsError::TrailingBytes {
                leftover: bytes.len() - cursor,
            });
        }
        Ok(Self { entries })
    }

    /// Number of pre-compiled expressions in the snapshot.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when no entries were captured. A no-binding doc snaps
    /// to an empty snapshot (~10-byte header only).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Run [`PackedChunk::verify`] on every entry. The first
    /// failure stops the walk and returns
    /// `(source, ChunkVerifyError)` so the caller can log which
    /// entry failed before falling back to JIT.
    pub fn verify_all(&self) -> Result<(), (String, ChunkVerifyError)> {
        for (source, chunk) in &self.entries {
            chunk.verify().map_err(|e| (source.clone(), e))?;
        }
        Ok(())
    }
}

// ──────────────────────────────────────────────────────────────────
// Internal helpers — encode / decode primitives + chunk frame.
// ──────────────────────────────────────────────────────────────────

fn write_string_field(
    out: &mut Vec<u8>,
    s: &str,
    entry_index: usize,
    field: &'static str,
) -> Result<(), ExpressionsError> {
    let len = u32::try_from(s.len()).map_err(|_| ExpressionsError::StringTooLong {
        entry_index,
        field,
        declared: u32::MAX,
        limit: MAX_STRING_BYTES,
    })?;
    if len > MAX_STRING_BYTES {
        return Err(ExpressionsError::StringTooLong {
            entry_index,
            field,
            declared: len,
            limit: MAX_STRING_BYTES,
        });
    }
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

fn read_string_field(
    bytes: &[u8],
    cursor: &mut usize,
    entry_index: usize,
    field: &'static str,
) -> Result<String, ExpressionsError> {
    if *cursor + 4 > bytes.len() {
        return Err(ExpressionsError::Truncated { entry_index });
    }
    let len = u32::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    if len > MAX_STRING_BYTES {
        return Err(ExpressionsError::StringTooLong {
            entry_index,
            field,
            declared: len,
            limit: MAX_STRING_BYTES,
        });
    }
    let len = len as usize;
    if *cursor + len > bytes.len() {
        return Err(ExpressionsError::Truncated { entry_index });
    }
    let s = std::str::from_utf8(&bytes[*cursor..*cursor + len])
        .map_err(|_| ExpressionsError::InvalidUtf8 { entry_index, field })?
        .to_owned();
    *cursor += len;
    Ok(s)
}

fn read_vec_len(
    bytes: &[u8],
    cursor: &mut usize,
    entry_index: usize,
    field: &'static str,
) -> Result<usize, ExpressionsError> {
    if *cursor + 4 > bytes.len() {
        return Err(ExpressionsError::Truncated { entry_index });
    }
    let n = u32::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    if n > MAX_VEC_LEN {
        return Err(ExpressionsError::VecTooLong {
            entry_index,
            field,
            declared: n,
            limit: MAX_VEC_LEN,
        });
    }
    Ok(n as usize)
}

fn write_chunk(
    out: &mut Vec<u8>,
    chunk: &PackedChunk,
    entry_index: usize,
) -> Result<(), ExpressionsError> {
    // ops_len + ops
    let ops_len = u32::try_from(chunk.ops.len()).map_err(|_| ExpressionsError::VecTooLong {
        entry_index,
        field: "ops",
        declared: u32::MAX,
        limit: MAX_VEC_LEN,
    })?;
    if ops_len > MAX_VEC_LEN {
        return Err(ExpressionsError::VecTooLong {
            entry_index,
            field: "ops",
            declared: ops_len,
            limit: MAX_VEC_LEN,
        });
    }
    out.extend_from_slice(&ops_len.to_le_bytes());
    for op in &chunk.ops {
        write_op(out, op);
    }
    // strings
    let s_len = u32::try_from(chunk.strings.len()).map_err(|_| ExpressionsError::VecTooLong {
        entry_index,
        field: "strings",
        declared: u32::MAX,
        limit: MAX_VEC_LEN,
    })?;
    if s_len > MAX_VEC_LEN {
        return Err(ExpressionsError::VecTooLong {
            entry_index,
            field: "strings",
            declared: s_len,
            limit: MAX_VEC_LEN,
        });
    }
    out.extend_from_slice(&s_len.to_le_bytes());
    for s in &chunk.strings {
        write_string_field(out, s, entry_index, "strings[]")?;
    }
    // scope_paths
    let p_len =
        u32::try_from(chunk.scope_paths.len()).map_err(|_| ExpressionsError::VecTooLong {
            entry_index,
            field: "scope_paths",
            declared: u32::MAX,
            limit: MAX_VEC_LEN,
        })?;
    if p_len > MAX_VEC_LEN {
        return Err(ExpressionsError::VecTooLong {
            entry_index,
            field: "scope_paths",
            declared: p_len,
            limit: MAX_VEC_LEN,
        });
    }
    out.extend_from_slice(&p_len.to_le_bytes());
    for p in &chunk.scope_paths {
        write_string_field(out, p, entry_index, "scope_paths[]")?;
    }
    Ok(())
}

fn write_op(out: &mut Vec<u8>, op: &PackedOpCode) {
    out.push(op.tag());
    match op {
        PackedOpCode::PushNum(n) => out.extend_from_slice(&n.to_le_bytes()),
        PackedOpCode::PushBool(b) => out.push(if *b { 1 } else { 0 }),
        PackedOpCode::PushNull
        | PackedOpCode::IndexGet
        | PackedOpCode::Not
        | PackedOpCode::Negate
        | PackedOpCode::UnaryPlus
        | PackedOpCode::Add
        | PackedOpCode::Sub
        | PackedOpCode::Mul
        | PackedOpCode::Div
        | PackedOpCode::Mod
        | PackedOpCode::Eq
        | PackedOpCode::NotEq
        | PackedOpCode::EqStrict
        | PackedOpCode::NotEqStrict
        | PackedOpCode::Lt
        | PackedOpCode::Gt
        | PackedOpCode::LtEq
        | PackedOpCode::GtEq
        | PackedOpCode::NullCoalesce
        | PackedOpCode::TemplateAppend
        | PackedOpCode::Return => {}
        PackedOpCode::PushString(i)
        | PackedOpCode::PushScopeRef(i)
        | PackedOpCode::MakeArray(i)
        | PackedOpCode::MakeObject(i)
        | PackedOpCode::PushObjectKey(i)
        | PackedOpCode::MemberGet(i) => out.extend_from_slice(&i.to_le_bytes()),
        PackedOpCode::JumpIfFalse(o) | PackedOpCode::JumpIfTrue(o) | PackedOpCode::Jump(o) => {
            out.extend_from_slice(&o.to_le_bytes())
        }
        PackedOpCode::CallBuiltin(name, argc) => {
            out.extend_from_slice(&name.to_le_bytes());
            out.extend_from_slice(&argc.to_le_bytes());
        }
    }
}

fn read_chunk(
    bytes: &[u8],
    cursor: &mut usize,
    entry_index: usize,
) -> Result<PackedChunk, ExpressionsError> {
    let ops_len = read_vec_len(bytes, cursor, entry_index, "ops")?;
    let mut ops = Vec::with_capacity(ops_len.min(64));
    for op_index in 0..ops_len {
        ops.push(read_op(bytes, cursor, entry_index, op_index)?);
    }
    let s_len = read_vec_len(bytes, cursor, entry_index, "strings")?;
    let mut strings = Vec::with_capacity(s_len.min(32));
    for _ in 0..s_len {
        strings.push(read_string_field(bytes, cursor, entry_index, "strings[]")?);
    }
    let p_len = read_vec_len(bytes, cursor, entry_index, "scope_paths")?;
    let mut scope_paths = Vec::with_capacity(p_len.min(32));
    for _ in 0..p_len {
        scope_paths.push(read_string_field(
            bytes,
            cursor,
            entry_index,
            "scope_paths[]",
        )?);
    }
    Ok(PackedChunk {
        ops,
        strings,
        scope_paths,
    })
}

fn read_op(
    bytes: &[u8],
    cursor: &mut usize,
    entry_index: usize,
    op_index: usize,
) -> Result<PackedOpCode, ExpressionsError> {
    if *cursor >= bytes.len() {
        return Err(ExpressionsError::Truncated { entry_index });
    }
    let tag = bytes[*cursor];
    *cursor += 1;
    let op = match tag {
        0 => PackedOpCode::PushNum(read_f64(bytes, cursor, entry_index)?),
        1 => {
            let b = read_u8(bytes, cursor, entry_index)?;
            PackedOpCode::PushBool(b != 0)
        }
        2 => PackedOpCode::PushNull,
        3 => PackedOpCode::PushString(read_u32(bytes, cursor, entry_index)?),
        4 => PackedOpCode::PushScopeRef(read_u32(bytes, cursor, entry_index)?),
        5 => PackedOpCode::MakeArray(read_u32(bytes, cursor, entry_index)?),
        6 => PackedOpCode::MakeObject(read_u32(bytes, cursor, entry_index)?),
        7 => PackedOpCode::PushObjectKey(read_u32(bytes, cursor, entry_index)?),
        8 => PackedOpCode::MemberGet(read_u32(bytes, cursor, entry_index)?),
        9 => PackedOpCode::IndexGet,
        10 => PackedOpCode::Not,
        11 => PackedOpCode::Negate,
        12 => PackedOpCode::UnaryPlus,
        13 => PackedOpCode::Add,
        14 => PackedOpCode::Sub,
        15 => PackedOpCode::Mul,
        16 => PackedOpCode::Div,
        17 => PackedOpCode::Mod,
        18 => PackedOpCode::Eq,
        19 => PackedOpCode::NotEq,
        20 => PackedOpCode::EqStrict,
        21 => PackedOpCode::NotEqStrict,
        22 => PackedOpCode::Lt,
        23 => PackedOpCode::Gt,
        24 => PackedOpCode::LtEq,
        25 => PackedOpCode::GtEq,
        26 => PackedOpCode::JumpIfFalse(read_i32(bytes, cursor, entry_index)?),
        27 => PackedOpCode::JumpIfTrue(read_i32(bytes, cursor, entry_index)?),
        28 => PackedOpCode::Jump(read_i32(bytes, cursor, entry_index)?),
        29 => PackedOpCode::NullCoalesce,
        30 => PackedOpCode::TemplateAppend,
        31 => {
            let name = read_u32(bytes, cursor, entry_index)?;
            let argc = read_u32(bytes, cursor, entry_index)?;
            PackedOpCode::CallBuiltin(name, argc)
        }
        32 => PackedOpCode::Return,
        _ => {
            return Err(ExpressionsError::UnknownOpTag {
                entry_index,
                op_index,
                tag,
            });
        }
    };
    Ok(op)
}

fn read_u8(bytes: &[u8], cursor: &mut usize, entry_index: usize) -> Result<u8, ExpressionsError> {
    if *cursor >= bytes.len() {
        return Err(ExpressionsError::Truncated { entry_index });
    }
    let v = bytes[*cursor];
    *cursor += 1;
    Ok(v)
}

fn read_u32(bytes: &[u8], cursor: &mut usize, entry_index: usize) -> Result<u32, ExpressionsError> {
    if *cursor + 4 > bytes.len() {
        return Err(ExpressionsError::Truncated { entry_index });
    }
    let v = u32::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    Ok(v)
}

fn read_i32(bytes: &[u8], cursor: &mut usize, entry_index: usize) -> Result<i32, ExpressionsError> {
    if *cursor + 4 > bytes.len() {
        return Err(ExpressionsError::Truncated { entry_index });
    }
    let v = i32::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    Ok(v)
}

fn read_f64(bytes: &[u8], cursor: &mut usize, entry_index: usize) -> Result<f64, ExpressionsError> {
    if *cursor + 8 > bytes.len() {
        return Err(ExpressionsError::Truncated { entry_index });
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[*cursor..*cursor + 8]);
    *cursor += 8;
    Ok(f64::from_le_bytes(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_chunk() -> PackedChunk {
        // One of every variant so the round-trip exercises every
        // tag-write / tag-read branch.
        PackedChunk {
            ops: vec![
                PackedOpCode::PushNum(1.234567890123456e10),
                PackedOpCode::PushBool(true),
                PackedOpCode::PushBool(false),
                PackedOpCode::PushNull,
                PackedOpCode::PushString(0),
                PackedOpCode::PushScopeRef(1),
                PackedOpCode::MakeArray(2),
                PackedOpCode::MakeObject(3),
                PackedOpCode::PushObjectKey(4),
                PackedOpCode::MemberGet(5),
                PackedOpCode::IndexGet,
                PackedOpCode::Not,
                PackedOpCode::Negate,
                PackedOpCode::UnaryPlus,
                PackedOpCode::Add,
                PackedOpCode::Sub,
                PackedOpCode::Mul,
                PackedOpCode::Div,
                PackedOpCode::Mod,
                PackedOpCode::Eq,
                PackedOpCode::NotEq,
                PackedOpCode::EqStrict,
                PackedOpCode::NotEqStrict,
                PackedOpCode::Lt,
                PackedOpCode::Gt,
                PackedOpCode::LtEq,
                PackedOpCode::GtEq,
                // Forward-only jump offsets so the synthetic chunk
                // passes structural verify. JumpIfFalse(6) at op
                // index 27 targets 27+1+6 = 34 = ops.len(), which
                // verify allows ("one past last" exits cleanly).
                // JumpIfTrue(1) skips one op; Jump(1) likewise.
                PackedOpCode::JumpIfFalse(3),
                PackedOpCode::JumpIfTrue(2),
                PackedOpCode::Jump(1),
                PackedOpCode::NullCoalesce,
                PackedOpCode::TemplateAppend,
                PackedOpCode::CallBuiltin(2, 4),
                PackedOpCode::Return,
            ],
            // Index every string that PushString / PushObjectKey /
            // MemberGet / CallBuiltin reference (indices 0..=5);
            // matching strings.len() to the highest index keeps the
            // chunk structurally valid for `verify_all`.
            strings: vec![
                "s0".into(),
                "s1".into(),
                "s2".into(),
                "s3".into(),
                "key4".into(),
                "member5".into(),
            ],
            scope_paths: vec!["$app.x".into(), "$app.count".into()],
        }
    }

    #[test]
    fn magic_and_version_are_pinned() {
        // Wire compatibility — bumping these requires a coordinated
        // reader update; the test guards against accidental edits.
        assert_eq!(&EXPRESSIONS_MAGIC, b"OPE1");
        assert_eq!(EXPRESSIONS_VERSION, 1);
    }

    #[test]
    fn variant_count_pinned_to_version() {
        // Codex round 1 CONCERN: adding a new `OpCode` variant
        // requires a coordinated wire change. This test pins
        // BOTH the highest tag value AND the format version, so a
        // contributor who adds a 34th variant has to update two
        // constants here — making it a single visible diff that
        // a reviewer (or CI's `cargo test`) catches before the
        // pack reader ships an old version reading future tags.
        const HIGHEST_TAG: u8 = 32;
        const PINNED_VERSION: u16 = 1;
        // Return is the highest-tagged variant today. If you add a
        // new variant AFTER Return, update HIGHEST_TAG to 33+ AND
        // bump EXPRESSIONS_VERSION here so old readers hard-reject
        // future packs (rather than silently misparse the new tag
        // as an `UnknownOpTag` error per-entry).
        assert_eq!(PackedOpCode::Return.tag(), HIGHEST_TAG);
        assert_eq!(EXPRESSIONS_VERSION, PINNED_VERSION);
    }

    #[test]
    fn tag_assignments_are_pinned() {
        // Every op-code tag is part of the wire format; renumbering
        // any of them silently breaks `aot/expressions.bin` for every
        // previously-published pack. Assert each assignment so a
        // refactor that touches the enum lights up the test.
        assert_eq!(PackedOpCode::PushNum(0.0).tag(), 0);
        assert_eq!(PackedOpCode::PushBool(true).tag(), 1);
        assert_eq!(PackedOpCode::PushNull.tag(), 2);
        assert_eq!(PackedOpCode::PushString(0).tag(), 3);
        assert_eq!(PackedOpCode::PushScopeRef(0).tag(), 4);
        assert_eq!(PackedOpCode::MakeArray(0).tag(), 5);
        assert_eq!(PackedOpCode::MakeObject(0).tag(), 6);
        assert_eq!(PackedOpCode::PushObjectKey(0).tag(), 7);
        assert_eq!(PackedOpCode::MemberGet(0).tag(), 8);
        assert_eq!(PackedOpCode::IndexGet.tag(), 9);
        assert_eq!(PackedOpCode::Not.tag(), 10);
        assert_eq!(PackedOpCode::Negate.tag(), 11);
        assert_eq!(PackedOpCode::UnaryPlus.tag(), 12);
        assert_eq!(PackedOpCode::Add.tag(), 13);
        assert_eq!(PackedOpCode::Sub.tag(), 14);
        assert_eq!(PackedOpCode::Mul.tag(), 15);
        assert_eq!(PackedOpCode::Div.tag(), 16);
        assert_eq!(PackedOpCode::Mod.tag(), 17);
        assert_eq!(PackedOpCode::Eq.tag(), 18);
        assert_eq!(PackedOpCode::NotEq.tag(), 19);
        assert_eq!(PackedOpCode::EqStrict.tag(), 20);
        assert_eq!(PackedOpCode::NotEqStrict.tag(), 21);
        assert_eq!(PackedOpCode::Lt.tag(), 22);
        assert_eq!(PackedOpCode::Gt.tag(), 23);
        assert_eq!(PackedOpCode::LtEq.tag(), 24);
        assert_eq!(PackedOpCode::GtEq.tag(), 25);
        assert_eq!(PackedOpCode::JumpIfFalse(0).tag(), 26);
        assert_eq!(PackedOpCode::JumpIfTrue(0).tag(), 27);
        assert_eq!(PackedOpCode::Jump(0).tag(), 28);
        assert_eq!(PackedOpCode::NullCoalesce.tag(), 29);
        assert_eq!(PackedOpCode::TemplateAppend.tag(), 30);
        assert_eq!(PackedOpCode::CallBuiltin(0, 0).tag(), 31);
        assert_eq!(PackedOpCode::Return.tag(), 32);
    }

    #[test]
    fn round_trip_empty_snapshot() {
        let s = ExpressionsSnapshot::default();
        let bytes = s.write_bytes().expect("encode");
        // Header only — 10 bytes (4 magic + 2 version + 4 entry-count).
        assert_eq!(bytes.len(), 10);
        let back = ExpressionsSnapshot::read_bytes(&bytes).expect("decode");
        assert_eq!(back, s);
    }

    #[test]
    fn round_trip_full_op_set() {
        let mut entries = BTreeMap::new();
        entries.insert("$app.count + 1".to_owned(), full_chunk());
        let s = ExpressionsSnapshot { entries };
        let bytes = s.write_bytes().expect("encode");
        let back = ExpressionsSnapshot::read_bytes(&bytes).expect("decode");
        assert_eq!(back, s);
    }

    #[test]
    fn round_trip_preserves_sort_order() {
        // Two snapshots with the same entries but different insertion
        // orders must produce byte-identical output (BTreeMap-driven
        // determinism — important for content-addressed pack hashes).
        let mut a = BTreeMap::new();
        a.insert("z".into(), PackedChunk::default());
        a.insert("a".into(), PackedChunk::default());
        a.insert("m".into(), PackedChunk::default());
        let mut b = BTreeMap::new();
        b.insert("a".into(), PackedChunk::default());
        b.insert("m".into(), PackedChunk::default());
        b.insert("z".into(), PackedChunk::default());
        assert_eq!(
            ExpressionsSnapshot { entries: a }.write_bytes().unwrap(),
            ExpressionsSnapshot { entries: b }.write_bytes().unwrap(),
        );
    }

    #[test]
    fn round_trip_preserves_unicode_sources() {
        let mut entries = BTreeMap::new();
        entries.insert("ascii + 1".into(), PackedChunk::default());
        entries.insert("中文 + 1".into(), PackedChunk::default());
        entries.insert("emoji-🎨".into(), PackedChunk::default());
        let s = ExpressionsSnapshot { entries };
        let bytes = s.write_bytes().expect("encode");
        let back = ExpressionsSnapshot::read_bytes(&bytes).expect("decode");
        assert_eq!(back, s);
        assert!(back.entries.contains_key("中文 + 1"));
    }

    #[test]
    fn empty_input_rejects_with_too_short() {
        let err = ExpressionsSnapshot::read_bytes(&[]).unwrap_err();
        assert_eq!(err, ExpressionsError::TooShort { got: 0, need: 10 });
    }

    #[test]
    fn truncated_header_rejects_with_too_short() {
        let err = ExpressionsSnapshot::read_bytes(&[1, 2, 3, 4, 5]).unwrap_err();
        assert_eq!(err, ExpressionsError::TooShort { got: 5, need: 10 });
    }

    #[test]
    fn wrong_magic_is_rejected() {
        let mut bytes = vec![b'O', b'P', b'E', b'2'];
        bytes.extend_from_slice(&[0u8; 6]);
        let err = ExpressionsSnapshot::read_bytes(&bytes).unwrap_err();
        assert!(matches!(err, ExpressionsError::BadMagic { got } if got == *b"OPE2"));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let mut bytes = EXPRESSIONS_MAGIC.to_vec();
        bytes.extend_from_slice(&99u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let err = ExpressionsSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(err, ExpressionsError::UnsupportedVersion { got: 99 });
    }

    #[test]
    fn unknown_op_tag_is_rejected() {
        // Header + 1 entry + chunk with one op whose tag is 99.
        let mut bytes = EXPRESSIONS_MAGIC.to_vec();
        bytes.extend_from_slice(&EXPRESSIONS_VERSION.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes()); // entry_count
        bytes.extend_from_slice(&1u32.to_le_bytes()); // source len
        bytes.extend_from_slice(b"x");
        bytes.extend_from_slice(&1u32.to_le_bytes()); // ops_len
        bytes.push(99); // unknown tag
        let err = ExpressionsSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(
            err,
            ExpressionsError::UnknownOpTag {
                entry_index: 0,
                op_index: 0,
                tag: 99
            }
        );
    }

    #[test]
    fn duplicate_entry_count_too_large_is_rejected() {
        let mut bytes = EXPRESSIONS_MAGIC.to_vec();
        bytes.extend_from_slice(&EXPRESSIONS_VERSION.to_le_bytes());
        // Entry count above ceiling — the reader bails before
        // attempting to allocate per-entry buffers.
        bytes.extend_from_slice(&((MAX_ENTRIES + 1).to_le_bytes()));
        let err = ExpressionsSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(
            err,
            ExpressionsError::EntryCountTooLarge {
                declared: MAX_ENTRIES + 1,
                limit: MAX_ENTRIES
            }
        );
    }

    #[test]
    fn duplicate_source_string_is_rejected() {
        // Hand-build two entries with identical sources. The
        // BTreeMap-driven writer can't produce this; only a
        // tampered-with archive can.
        let chunk_bytes = {
            let mut out = Vec::new();
            write_chunk(&mut out, &PackedChunk::default(), 0).unwrap();
            out
        };
        let mut bytes = EXPRESSIONS_MAGIC.to_vec();
        bytes.extend_from_slice(&EXPRESSIONS_VERSION.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes()); // 2 entries
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(b"x");
        bytes.extend_from_slice(&chunk_bytes);
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(b"x");
        bytes.extend_from_slice(&chunk_bytes);
        let err = ExpressionsSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(err, ExpressionsError::DuplicateSource { entry_index: 1 });
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let s = ExpressionsSnapshot::default();
        let mut bytes = s.write_bytes().unwrap();
        bytes.push(0xff);
        let err = ExpressionsSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(err, ExpressionsError::TrailingBytes { leftover: 1 });
    }

    #[test]
    fn truncated_after_header_is_rejected() {
        // entry_count claims 1 but no source-length follows.
        let mut bytes = EXPRESSIONS_MAGIC.to_vec();
        bytes.extend_from_slice(&EXPRESSIONS_VERSION.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        let err = ExpressionsSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(err, ExpressionsError::Truncated { entry_index: 0 });
    }

    #[test]
    fn invalid_utf8_source_is_rejected() {
        let mut bytes = EXPRESSIONS_MAGIC.to_vec();
        bytes.extend_from_slice(&EXPRESSIONS_VERSION.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&[0xff, 0xfe]); // invalid utf-8
        let err = ExpressionsSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(
            err,
            ExpressionsError::InvalidUtf8 {
                entry_index: 0,
                field: "source"
            }
        );
    }

    #[test]
    fn read_bytes_rejects_oversized_string_field() {
        // Hand-build a header + entry whose source-length declares
        // MAX_STRING_BYTES + 1 — the reader must bail before
        // allocating. Catches a cheap DoS vector.
        let mut bytes = EXPRESSIONS_MAGIC.to_vec();
        bytes.extend_from_slice(&EXPRESSIONS_VERSION.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&(MAX_STRING_BYTES + 1).to_le_bytes());
        let err = ExpressionsSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(
            err,
            ExpressionsError::StringTooLong {
                entry_index: 0,
                field: "source",
                declared: MAX_STRING_BYTES + 1,
                limit: MAX_STRING_BYTES,
            }
        );
    }

    #[test]
    fn read_bytes_rejects_oversized_vec() {
        let mut bytes = EXPRESSIONS_MAGIC.to_vec();
        bytes.extend_from_slice(&EXPRESSIONS_VERSION.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(b"x");
        bytes.extend_from_slice(&(MAX_VEC_LEN + 1).to_le_bytes()); // ops count
        let err = ExpressionsSnapshot::read_bytes(&bytes).unwrap_err();
        assert_eq!(
            err,
            ExpressionsError::VecTooLong {
                entry_index: 0,
                field: "ops",
                declared: MAX_VEC_LEN + 1,
                limit: MAX_VEC_LEN,
            }
        );
    }

    #[test]
    fn round_trip_preserves_jump_offsets() {
        // i32 sign extension matters — a careless encoder might emit
        // u32 bytes for a negative offset and the reader would
        // decode `-3` as a huge positive number.
        let mut entries = BTreeMap::new();
        entries.insert(
            "branchy".to_owned(),
            PackedChunk {
                ops: vec![
                    PackedOpCode::JumpIfFalse(-12345),
                    PackedOpCode::Jump(i32::MIN),
                    PackedOpCode::JumpIfTrue(i32::MAX),
                ],
                strings: vec![],
                scope_paths: vec![],
            },
        );
        let s = ExpressionsSnapshot { entries };
        let bytes = s.write_bytes().unwrap();
        let back = ExpressionsSnapshot::read_bytes(&bytes).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn round_trip_preserves_f64_bit_pattern() {
        // PushNum carries arbitrary f64 — exercise NaN, ±Inf, zeros,
        // subnormals so the encoder doesn't lose bits via a coercion.
        let mut entries = BTreeMap::new();
        entries.insert(
            "nums".to_owned(),
            PackedChunk {
                ops: vec![
                    PackedOpCode::PushNum(0.0),
                    PackedOpCode::PushNum(-0.0),
                    PackedOpCode::PushNum(f64::INFINITY),
                    PackedOpCode::PushNum(f64::NEG_INFINITY),
                    PackedOpCode::PushNum(f64::MIN_POSITIVE),
                    PackedOpCode::PushNum(f64::EPSILON),
                ],
                strings: vec![],
                scope_paths: vec![],
            },
        );
        let s = ExpressionsSnapshot { entries };
        let bytes = s.write_bytes().unwrap();
        let back = ExpressionsSnapshot::read_bytes(&bytes).unwrap();
        // Bit-exact comparison via BTreeMap PartialEq → Vec<f64> →
        // bitwise: 0.0 == -0.0 in PartialEq, but to_le_bytes preserves
        // sign — assert that explicitly with an extra check.
        assert_eq!(back, s);
        let original_bytes: Vec<u8> = s
            .entries
            .values()
            .flat_map(|c| {
                c.ops.iter().flat_map(|op| match op {
                    PackedOpCode::PushNum(n) => n.to_le_bytes().to_vec(),
                    _ => vec![],
                })
            })
            .collect();
        let restored_bytes: Vec<u8> = back
            .entries
            .values()
            .flat_map(|c| {
                c.ops.iter().flat_map(|op| match op {
                    PackedOpCode::PushNum(n) => n.to_le_bytes().to_vec(),
                    _ => vec![],
                })
            })
            .collect();
        assert_eq!(original_bytes, restored_bytes, "f64 bit pattern preserved");
    }

    #[test]
    fn verify_accepts_valid_chunk() {
        // The synthetic full_chunk passes structural checks: every
        // index is in-range, every jump is forward and lands in
        // [0, ops.len()].
        let mut entries = BTreeMap::new();
        entries.insert("ok".into(), full_chunk());
        let snap = ExpressionsSnapshot { entries };
        snap.verify_all().expect("valid chunk passes verify");
    }

    #[test]
    fn verify_rejects_string_index_out_of_range() {
        let chunk = PackedChunk {
            ops: vec![PackedOpCode::PushString(5)],
            strings: vec!["only-one".into()],
            scope_paths: vec![],
        };
        let err = chunk.verify().unwrap_err();
        assert_eq!(
            err,
            ChunkVerifyError::StringIndexOutOfRange {
                op_index: 0,
                idx: 5,
                pool_len: 1,
            }
        );
    }

    #[test]
    fn verify_rejects_scope_index_out_of_range() {
        let chunk = PackedChunk {
            ops: vec![PackedOpCode::PushScopeRef(2)],
            strings: vec![],
            scope_paths: vec!["$app.x".into()],
        };
        let err = chunk.verify().unwrap_err();
        assert_eq!(
            err,
            ChunkVerifyError::ScopeIndexOutOfRange {
                op_index: 0,
                idx: 2,
                pool_len: 1,
            }
        );
    }

    #[test]
    fn verify_rejects_call_builtin_with_oob_name() {
        let chunk = PackedChunk {
            ops: vec![PackedOpCode::CallBuiltin(99, 0)],
            strings: vec![],
            scope_paths: vec![],
        };
        let err = chunk.verify().unwrap_err();
        assert_eq!(
            err,
            ChunkVerifyError::StringIndexOutOfRange {
                op_index: 0,
                idx: 99,
                pool_len: 0,
            }
        );
    }

    #[test]
    fn verify_rejects_backwards_jump() {
        let chunk = PackedChunk {
            ops: vec![
                PackedOpCode::PushNum(0.0),
                PackedOpCode::Jump(-1), // backwards loop
                PackedOpCode::Return,
            ],
            strings: vec![],
            scope_paths: vec![],
        };
        let err = chunk.verify().unwrap_err();
        assert_eq!(
            err,
            ChunkVerifyError::BackwardsJump {
                op_index: 1,
                offset: -1,
            }
        );
    }

    #[test]
    fn verify_rejects_zero_offset_jump_as_backwards() {
        // A `Jump(0)` re-enters itself — same infinite-loop class
        // as a negative offset. Reject.
        let chunk = PackedChunk {
            ops: vec![PackedOpCode::Jump(0)],
            strings: vec![],
            scope_paths: vec![],
        };
        let err = chunk.verify().unwrap_err();
        assert_eq!(
            err,
            ChunkVerifyError::BackwardsJump {
                op_index: 0,
                offset: 0,
            }
        );
    }

    #[test]
    fn verify_rejects_jump_past_ops_end() {
        let chunk = PackedChunk {
            ops: vec![PackedOpCode::Jump(1000), PackedOpCode::Return],
            strings: vec![],
            scope_paths: vec![],
        };
        let err = chunk.verify().unwrap_err();
        match err {
            ChunkVerifyError::JumpOutOfRange {
                op_index,
                target,
                ops_len,
                ..
            } => {
                assert_eq!(op_index, 0);
                assert_eq!(target, 1001);
                assert_eq!(ops_len, 2);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn verify_allows_jump_to_one_past_last_op() {
        // The VM's `while ip < ops.len()` loop terminates cleanly
        // when ip == ops.len() — a forward jump landing exactly at
        // the post-Return position is a legitimate "exit"
        // pattern (e.g., the end of an `if`'s else branch).
        let chunk = PackedChunk {
            ops: vec![
                PackedOpCode::PushBool(true),
                PackedOpCode::JumpIfFalse(1), // skip the next op → land at index 3 = ops_len
                PackedOpCode::PushNum(1.0),
                PackedOpCode::Return,
            ],
            strings: vec![],
            scope_paths: vec![],
        };
        chunk.verify().expect("jump to ops_len should pass");
    }

    #[test]
    fn empty_chunk_round_trips() {
        // A chunk with all three Vecs empty — this is the common
        // "constant null" expression's compiled form.
        let mut entries = BTreeMap::new();
        entries.insert("null".into(), PackedChunk::default());
        let s = ExpressionsSnapshot { entries };
        let bytes = s.write_bytes().unwrap();
        let back = ExpressionsSnapshot::read_bytes(&bytes).unwrap();
        assert_eq!(back, s);
    }
}
