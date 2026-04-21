//! Builtin function registry. Each builtin is a `Box<dyn Fn(&dyn EvalContext, &[RuntimeValue])
//! -> Result<RuntimeValue, Diagnostic>>`.
//!
//! Sub-modules register their functions here; the runtime composes a single
//! registry from `default_builtins()`.

pub mod math;
pub mod string;

use super::diag::{DiagKind, Diagnostic, Span};
pub use super::scope::BuiltinFn;
use std::collections::BTreeMap;

pub fn default_builtins() -> BTreeMap<String, BuiltinFn> {
    let mut m: BTreeMap<String, BuiltinFn> = BTreeMap::new();
    math::register(&mut m);
    string::register(&mut m);
    m
}

pub fn arity_mismatch(name: &str, expected: &str, actual: usize) -> Diagnostic {
    Diagnostic {
        kind: DiagKind::ArityMismatch,
        message: format!("`{}` expects {} arg(s), got {}", name, expected, actual),
        span: Span::zero(),
    }
}

pub fn type_error(name: &str, message: &str) -> Diagnostic {
    Diagnostic {
        kind: DiagKind::TypeError,
        message: format!("`{}`: {}", name, message),
        span: Span::zero(),
    }
}
