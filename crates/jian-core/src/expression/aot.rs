//! `Chunk` ↔ `PackedChunk` glue (Plan 19 D2).
//!
//! The wire-stable mirror types live in `jian_ops_schema::pack::
//! expressions`; this module converts between them and the live
//! `jian_core::expression::bytecode` types so the AOT writer + reader
//! never need to import each other's internals.
//!
//! Conversion is total in both directions — there's no fallible
//! state because the mirror enum's variant set matches `OpCode`'s
//! variant set 1:1 (a pinned-tag invariant tested in
//! `pack::expressions::tests::tag_assignments_are_pinned`).

use super::bytecode::{Chunk, OpCode};
use jian_ops_schema::pack::{ExpressionsSnapshot, PackedChunk, PackedOpCode};
use std::collections::BTreeMap;

impl From<&OpCode> for PackedOpCode {
    fn from(op: &OpCode) -> Self {
        match op {
            OpCode::PushNum(n) => PackedOpCode::PushNum(*n),
            OpCode::PushBool(b) => PackedOpCode::PushBool(*b),
            OpCode::PushNull => PackedOpCode::PushNull,
            OpCode::PushString(i) => PackedOpCode::PushString(*i),
            OpCode::PushScopeRef(i) => PackedOpCode::PushScopeRef(*i),
            OpCode::MakeArray(i) => PackedOpCode::MakeArray(*i),
            OpCode::MakeObject(i) => PackedOpCode::MakeObject(*i),
            OpCode::PushObjectKey(i) => PackedOpCode::PushObjectKey(*i),
            OpCode::MemberGet(i) => PackedOpCode::MemberGet(*i),
            OpCode::IndexGet => PackedOpCode::IndexGet,
            OpCode::Not => PackedOpCode::Not,
            OpCode::Negate => PackedOpCode::Negate,
            OpCode::UnaryPlus => PackedOpCode::UnaryPlus,
            OpCode::Add => PackedOpCode::Add,
            OpCode::Sub => PackedOpCode::Sub,
            OpCode::Mul => PackedOpCode::Mul,
            OpCode::Div => PackedOpCode::Div,
            OpCode::Mod => PackedOpCode::Mod,
            OpCode::Eq => PackedOpCode::Eq,
            OpCode::NotEq => PackedOpCode::NotEq,
            OpCode::EqStrict => PackedOpCode::EqStrict,
            OpCode::NotEqStrict => PackedOpCode::NotEqStrict,
            OpCode::Lt => PackedOpCode::Lt,
            OpCode::Gt => PackedOpCode::Gt,
            OpCode::LtEq => PackedOpCode::LtEq,
            OpCode::GtEq => PackedOpCode::GtEq,
            OpCode::JumpIfFalse(o) => PackedOpCode::JumpIfFalse(*o),
            OpCode::JumpIfTrue(o) => PackedOpCode::JumpIfTrue(*o),
            OpCode::Jump(o) => PackedOpCode::Jump(*o),
            OpCode::NullCoalesce => PackedOpCode::NullCoalesce,
            OpCode::TemplateAppend => PackedOpCode::TemplateAppend,
            OpCode::CallBuiltin(name, argc) => PackedOpCode::CallBuiltin(*name, *argc),
            OpCode::Return => PackedOpCode::Return,
        }
    }
}

impl From<&PackedOpCode> for OpCode {
    fn from(op: &PackedOpCode) -> Self {
        match op {
            PackedOpCode::PushNum(n) => OpCode::PushNum(*n),
            PackedOpCode::PushBool(b) => OpCode::PushBool(*b),
            PackedOpCode::PushNull => OpCode::PushNull,
            PackedOpCode::PushString(i) => OpCode::PushString(*i),
            PackedOpCode::PushScopeRef(i) => OpCode::PushScopeRef(*i),
            PackedOpCode::MakeArray(i) => OpCode::MakeArray(*i),
            PackedOpCode::MakeObject(i) => OpCode::MakeObject(*i),
            PackedOpCode::PushObjectKey(i) => OpCode::PushObjectKey(*i),
            PackedOpCode::MemberGet(i) => OpCode::MemberGet(*i),
            PackedOpCode::IndexGet => OpCode::IndexGet,
            PackedOpCode::Not => OpCode::Not,
            PackedOpCode::Negate => OpCode::Negate,
            PackedOpCode::UnaryPlus => OpCode::UnaryPlus,
            PackedOpCode::Add => OpCode::Add,
            PackedOpCode::Sub => OpCode::Sub,
            PackedOpCode::Mul => OpCode::Mul,
            PackedOpCode::Div => OpCode::Div,
            PackedOpCode::Mod => OpCode::Mod,
            PackedOpCode::Eq => OpCode::Eq,
            PackedOpCode::NotEq => OpCode::NotEq,
            PackedOpCode::EqStrict => OpCode::EqStrict,
            PackedOpCode::NotEqStrict => OpCode::NotEqStrict,
            PackedOpCode::Lt => OpCode::Lt,
            PackedOpCode::Gt => OpCode::Gt,
            PackedOpCode::LtEq => OpCode::LtEq,
            PackedOpCode::GtEq => OpCode::GtEq,
            PackedOpCode::JumpIfFalse(o) => OpCode::JumpIfFalse(*o),
            PackedOpCode::JumpIfTrue(o) => OpCode::JumpIfTrue(*o),
            PackedOpCode::Jump(o) => OpCode::Jump(*o),
            PackedOpCode::NullCoalesce => OpCode::NullCoalesce,
            PackedOpCode::TemplateAppend => OpCode::TemplateAppend,
            PackedOpCode::CallBuiltin(name, argc) => OpCode::CallBuiltin(*name, *argc),
            PackedOpCode::Return => OpCode::Return,
        }
    }
}

impl From<&Chunk> for PackedChunk {
    fn from(c: &Chunk) -> Self {
        PackedChunk {
            ops: c.ops.iter().map(PackedOpCode::from).collect(),
            strings: c.strings.clone(),
            scope_paths: c.scope_paths.clone(),
        }
    }
}

impl From<&PackedChunk> for Chunk {
    fn from(c: &PackedChunk) -> Self {
        Chunk {
            ops: c.ops.iter().map(OpCode::from).collect(),
            strings: c.strings.clone(),
            scope_paths: c.scope_paths.clone(),
        }
    }
}

/// Convert the full [`ExpressionsSnapshot`] into a
/// `BTreeMap<String, Chunk>` ready for
/// [`super::cache::ExpressionCache::install_precompiled`]. Stand-
/// alone helper so call-sites don't repeat the
/// `iter().map(...).collect()` chain.
pub fn snapshot_to_chunks(snap: &ExpressionsSnapshot) -> BTreeMap<String, Chunk> {
    snap.entries
        .iter()
        .map(|(src, packed)| (src.clone(), Chunk::from(packed)))
        .collect()
}

/// Convert a `BTreeMap<String, Chunk>` (e.g. from
/// [`super::cache::ExpressionCache::dump`]) into an
/// [`ExpressionsSnapshot`] ready for the writer.
pub fn chunks_to_snapshot(chunks: &BTreeMap<String, Chunk>) -> ExpressionsSnapshot {
    ExpressionsSnapshot {
        entries: chunks
            .iter()
            .map(|(src, chunk)| (src.clone(), PackedChunk::from(chunk)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expression::Expression;

    #[test]
    fn round_trip_compiled_expression_through_snapshot() {
        // Take a real compiled expression, snapshot it, decode back,
        // confirm bit-for-bit equality of the chunk.
        let expr = Expression::compile("$app.count + 1").expect("compile");
        let mut chunks = BTreeMap::new();
        chunks.insert(expr.source.clone(), expr.chunk.clone());
        let snap = chunks_to_snapshot(&chunks);
        let bytes = snap.write_bytes().expect("encode");
        let back = jian_ops_schema::pack::ExpressionsSnapshot::read_bytes(&bytes).expect("decode");
        let restored = snapshot_to_chunks(&back);
        let restored_chunk = restored.get(&expr.source).expect("source preserved");
        assert_eq!(restored_chunk.ops, expr.chunk.ops);
        assert_eq!(restored_chunk.strings, expr.chunk.strings);
        assert_eq!(restored_chunk.scope_paths, expr.chunk.scope_paths);
    }

    #[test]
    fn round_trip_preserves_string_pool() {
        // Template expressions intern strings; round-trip must
        // preserve them as-is for `PushString(idx)` to resolve to
        // the same byte sequence.
        let expr = Expression::compile("\"hello \" + $app.name").expect("compile");
        let mut chunks = BTreeMap::new();
        chunks.insert(expr.source.clone(), expr.chunk.clone());
        let snap = chunks_to_snapshot(&chunks);
        let restored = snapshot_to_chunks(&snap);
        let r = restored.get(&expr.source).unwrap();
        assert_eq!(r.strings, expr.chunk.strings);
    }
}
