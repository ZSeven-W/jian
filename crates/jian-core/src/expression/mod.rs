//! Tier 1 expression language.
//!
//! Compiles a source string like `"$state.count + 1"` to bytecode, and
//! evaluates it against a runtime context (StateGraph + scope overrides for
//! `$self`, `$item`, etc.). Evaluation auto-subscribes to every Signal
//! that `get()` reads, so effects built on top of `eval_with_tracker` get
//! proper fine-grained reactivity.

pub mod ast;
pub mod bytecode;
pub mod compiler;
pub mod diag;
pub mod lexer;
pub mod parser;
pub mod scope;
pub mod token;
pub mod vm;

pub use diag::{DiagKind, Diagnostic, Span};
