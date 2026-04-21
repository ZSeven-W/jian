//! Tier 1 expression language.
//!
//! Compiles a source string like `"$state.count + 1"` to bytecode, and
//! evaluates it against a runtime context (StateGraph + scope overrides for
//! `$self`, `$item`, etc.). Evaluation auto-subscribes to every Signal
//! that `get()` reads, so effects built on top of `eval_with_tracker` get
//! proper fine-grained reactivity.

pub mod ast;
pub mod builtins;
pub mod bytecode;
pub mod cache;
pub mod compiler;
pub mod diag;
pub mod lexer;
pub mod parser;
pub mod scope;
pub mod token;
pub mod vm;

pub use bytecode::Chunk;
pub use cache::ExpressionCache;
pub use diag::{DiagKind, Diagnostic, Span};
pub use scope::StateGraphContext;
pub use vm::EvalContext;

use crate::state::StateGraph;
use crate::value::RuntimeValue;
use std::collections::BTreeMap;

/// A compiled expression; holds the source + bytecode. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Expression {
    pub source: String,
    pub chunk: Chunk,
}

impl Expression {
    /// Compile a source string to bytecode.
    pub fn compile(source: &str) -> Result<Self, Diagnostic> {
        let ast = parser::parse(source)?;
        let chunk = compiler::compile(&ast)?;
        Ok(Self {
            source: source.to_owned(),
            chunk,
        })
    }

    /// Evaluate against a StateGraph with node/page context. Returns
    /// `(value, warnings)`; runtime errors become warnings + `null`.
    pub fn eval(
        &self,
        state: &StateGraph,
        page: Option<&str>,
        node: Option<&str>,
    ) -> (RuntimeValue, Vec<Diagnostic>) {
        let locals: BTreeMap<String, RuntimeValue> = BTreeMap::new();
        let builtins = builtins::default_builtins();
        let ctx = StateGraphContext::new(state, page, node, &locals, &builtins);
        let v = match vm::run(&self.chunk, &ctx) {
            Ok(v) => v,
            Err(d) => {
                ctx.warn(d);
                RuntimeValue::null()
            }
        };
        (v, ctx.take_warnings())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::scheduler::Scheduler;
    use serde_json::json;
    use std::rc::Rc;

    #[test]
    fn compile_then_eval() {
        let sched = Rc::new(Scheduler::new());
        let state = StateGraph::new(sched);
        state.app_set("count", json!(7));

        let expr = Expression::compile("$app.count + 1").unwrap();
        let (v, warnings) = expr.eval(&state, None, None);
        assert_eq!(v.as_i64(), Some(8));
        assert!(warnings.is_empty());
    }

    #[test]
    fn compile_error_returns_diagnostic() {
        let err = Expression::compile("1 +").unwrap_err();
        assert_eq!(err.kind, DiagKind::ParseError);
    }
}
