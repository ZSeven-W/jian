//! Compiled-SkSL `RuntimeEffect` cache.
//!
//! [`DrawOp::ShaderRect`](jian_core::render::DrawOp) carries RAW SkSL
//! source. The editor repaints every frame, so compiling the program on
//! each draw would be a per-frame `make_for_shader` call — the same
//! mistake the typeface cache in `op-host-native` exists to avoid. This
//! cache compiles a given source exactly once (keyed on a hash of the
//! source bytes) and reuses the refcounted `RuntimeEffect` handle
//! thereafter.
//!
//! Compile *failures* are cached too (as `None`) so an invalid shader
//! doesn't burn the SkSL compiler on every redraw. A `None` entry tells
//! the caller to paint the solid fallback.
//!
//! `RuntimeEffect` is an `RCHandle` (atomic-refcounted) — cloning it to
//! hand into a fresh `RuntimeShaderBuilder` per frame is a cheap
//! refcount bump, NOT a recompile.

use skia_safe::RuntimeEffect;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Hash an SkSL source string to its cache key. Public so render paths
/// in other crates (the native host's own cache) can key on the same
/// value if they share source.
pub fn sksl_hash(sksl: &str) -> u64 {
    let mut h = DefaultHasher::new();
    sksl.hash(&mut h);
    h.finish()
}

#[derive(Default)]
pub struct ShaderCache {
    /// `None` value = a previously-attempted source that failed to
    /// compile (don't retry every frame).
    effects: HashMap<u64, Option<RuntimeEffect>>,
    /// Count of distinct `make_for_shader` invocations — the
    /// compile-cache proof reads this to confirm compile-once.
    compile_count: u64,
}

impl ShaderCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the compiled effect for `sksl`, compiling + caching it on
    /// first sight. Returns `None` (cached) when the source fails to
    /// compile — the caller then paints its solid fallback. Never
    /// panics: `make_for_shader` returns a `Result` and the `Err` arm is
    /// stored as a cached `None`.
    pub fn get_or_compile(&mut self, sksl: &str) -> Option<RuntimeEffect> {
        let key = sksl_hash(sksl);
        if let Some(hit) = self.effects.get(&key) {
            return hit.clone();
        }
        let compiled = match RuntimeEffect::make_for_shader(sksl, None) {
            Ok(effect) => Some(effect),
            Err(_msg) => None,
        };
        self.compile_count += 1;
        self.effects.insert(key, compiled.clone());
        compiled
    }

    /// Number of distinct compile attempts performed so far. Used by the
    /// render-proof / tests to assert compile-once-per-unique-source.
    pub fn compile_count(&self) -> u64 {
        self.compile_count
    }
}
