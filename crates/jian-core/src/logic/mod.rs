//! LogicProvider trait — the Tier 3 extension point (reserved for L4).
//!
//! MVP does not ship any provider implementation. The trait exists so that
//! `jian-core` can compile against it and Action-DSL code (Plan 4)
//! can call into it when present. WASM and Native implementations land in
//! Stage G.

use crate::value::RuntimeValue;

#[derive(Debug, Clone)]
pub struct FunctionSignature {
    pub name: &'static str,
    pub is_async: bool,
}

pub trait LogicProvider {
    fn id(&self) -> &str;
    fn exports(&self) -> &[FunctionSignature];
    fn call(&self, function: &str, args: &[RuntimeValue]) -> Result<RuntimeValue, String>;
}

/// Null provider returned when a logic module is referenced but the runtime
/// has no provider available (e.g. MVP builds).
pub struct NullLogicProvider;

impl LogicProvider for NullLogicProvider {
    fn id(&self) -> &str {
        "null"
    }
    fn exports(&self) -> &[FunctionSignature] {
        &[]
    }
    fn call(&self, function: &str, _: &[RuntimeValue]) -> Result<RuntimeValue, String> {
        Err(format!(
            "logic provider is not installed: cannot call `{}`",
            function
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_provider_rejects_call() {
        let p = NullLogicProvider;
        let err = p.call("foo", &[]).unwrap_err();
        assert!(err.contains("not installed"));
    }
}
