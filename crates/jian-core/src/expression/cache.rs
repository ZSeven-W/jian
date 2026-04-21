//! Per-runtime compilation cache.
//!
//! Keyed by source string. Stores the compiled `Chunk` (not `Expression`
//! because the latter also owns the source, which we'd duplicate).

use super::bytecode::Chunk;
use super::compiler::compile;
use super::diag::Diagnostic;
use super::parser::parse;
use std::cell::RefCell;
use std::collections::HashMap;

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
}
