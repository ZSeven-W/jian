//! Per-runtime compilation cache.
//!
//! Keyed by source string. Stores the compiled `Chunk` (not `Expression`
//! because the latter also owns the source, which we'd duplicate).

use super::bytecode::Chunk;
use super::compiler::compile;
use super::diag::Diagnostic;
use super::parser::parse;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

pub struct ExpressionCache {
    entries: RefCell<HashMap<String, Chunk>>,
    hits: RefCell<u64>,
    misses: RefCell<u64>,
}

impl ExpressionCache {
    pub fn new() -> Self {
        Self {
            entries: RefCell::new(HashMap::new()),
            hits: RefCell::new(0),
            misses: RefCell::new(0),
        }
    }

    /// Look up or compile the source. Returns a cloned Chunk.
    pub fn get_or_compile(&self, source: &str) -> Result<Chunk, Diagnostic> {
        if let Some(c) = self.entries.borrow().get(source) {
            *self.hits.borrow_mut() += 1;
            return Ok(c.clone());
        }
        *self.misses.borrow_mut() += 1;
        let ast = parse(source)?;
        let chunk = compile(&ast)?;
        self.entries
            .borrow_mut()
            .insert(source.to_owned(), chunk.clone());
        Ok(chunk)
    }

    pub fn hit_rate(&self) -> (u64, u64) {
        (*self.hits.borrow(), *self.misses.borrow())
    }

    pub fn len(&self) -> usize {
        self.entries.borrow().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        self.entries.borrow_mut().clear();
        *self.hits.borrow_mut() = 0;
        *self.misses.borrow_mut() = 0;
    }

    /// Sorted snapshot of every cached source → compiled `Chunk`.
    /// `BTreeMap` (vs the inner `HashMap`) gives deterministic
    /// iteration so AOT pack writers can produce content-addressed
    /// byte-identical output across runs (Plan 19 D2). Returns
    /// cloned chunks so the caller can serialise without holding a
    /// borrow on the cache.
    pub fn dump(&self) -> BTreeMap<String, Chunk> {
        self.entries
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Seed the cache with pre-compiled chunks. Used by the cold-
    /// start bootstrap when a `.op.pack` ships an
    /// `aot/expressions.bin` snapshot — a subsequent
    /// [`Self::get_or_compile`] for a seeded source returns the
    /// pre-compiled chunk and skips parse+compile.
    ///
    /// `entries` is taken by value because the typical caller has
    /// just decoded the snapshot and has no further use for it.
    /// Pre-existing entries with the same source are preserved (the
    /// already-cached chunk wins) — a host that calls this AFTER
    /// some lazy compilation sees no surprise displacement. A fresh
    /// cache (the cold-start case) trivially has no collisions and
    /// gets every seeded entry.
    ///
    /// Counters are NOT incremented for the seed: a seeded entry
    /// that's never read shouldn't poison the hit-rate metric.
    pub fn install_precompiled(&self, entries: BTreeMap<String, Chunk>) {
        let mut map = self.entries.borrow_mut();
        for (source, chunk) in entries {
            map.entry(source).or_insert(chunk);
        }
    }
}

impl Default for ExpressionCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_hit_is_miss() {
        let c = ExpressionCache::new();
        c.get_or_compile("1 + 2").unwrap();
        assert_eq!(c.hit_rate(), (0, 1));
    }

    #[test]
    fn second_same_source_is_hit() {
        let c = ExpressionCache::new();
        c.get_or_compile("1 + 2").unwrap();
        c.get_or_compile("1 + 2").unwrap();
        assert_eq!(c.hit_rate(), (1, 1));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn different_sources_are_distinct() {
        let c = ExpressionCache::new();
        c.get_or_compile("1 + 2").unwrap();
        c.get_or_compile("3 + 4").unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c.hit_rate(), (0, 2));
    }

    #[test]
    fn compile_error_not_cached() {
        let c = ExpressionCache::new();
        let err = c.get_or_compile("1 +").unwrap_err();
        assert_eq!(err.kind, super::super::diag::DiagKind::ParseError);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn clear_resets() {
        let c = ExpressionCache::new();
        c.get_or_compile("x").unwrap();
        c.clear();
        assert_eq!(c.len(), 0);
        assert_eq!(c.hit_rate(), (0, 0));
    }

    #[test]
    fn dump_returns_sorted_clones() {
        let c = ExpressionCache::new();
        c.get_or_compile("c").unwrap();
        c.get_or_compile("a").unwrap();
        c.get_or_compile("b").unwrap();
        let dumped = c.dump();
        let keys: Vec<_> = dumped.keys().cloned().collect();
        assert_eq!(keys, vec!["a", "b", "c"]);
    }

    #[test]
    fn install_precompiled_seeds_fresh_cache() {
        let donor = ExpressionCache::new();
        donor.get_or_compile("$app.count + 1").unwrap();
        let seed = donor.dump();

        let recipient = ExpressionCache::new();
        recipient.install_precompiled(seed);
        assert_eq!(recipient.len(), 1);
        // Counters not bumped by the seed — pristine hit-rate state.
        assert_eq!(recipient.hit_rate(), (0, 0));

        // Subsequent get_or_compile for the seeded source becomes a
        // hit: parse+compile was skipped (the runtime contract that
        // makes AOT useful).
        recipient.get_or_compile("$app.count + 1").unwrap();
        assert_eq!(recipient.hit_rate(), (1, 0));
    }

    #[test]
    fn install_precompiled_does_not_displace_existing() {
        // A host that calls install_precompiled AFTER some lazy
        // compilation should keep the already-compiled chunk; the
        // seed only fills empty slots.
        let c = ExpressionCache::new();
        c.get_or_compile("a + 1").unwrap();
        let already = c.dump().get("a + 1").cloned().unwrap();

        // Fabricate a different chunk for the same source by
        // compiling a different expression and renaming the key.
        let donor = ExpressionCache::new();
        donor.get_or_compile("b + 2").unwrap();
        let mut seed = BTreeMap::new();
        seed.insert("a + 1".to_owned(), donor.dump().remove("b + 2").unwrap());

        c.install_precompiled(seed);
        let after = c.dump().get("a + 1").cloned().unwrap();
        assert_eq!(after.ops, already.ops, "existing entry preserved");
    }
}
