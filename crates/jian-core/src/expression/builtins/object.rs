use super::{arity_mismatch, type_error, BuiltinFn};
use crate::expression::vm::EvalContext;
use crate::value::RuntimeValue;
use serde_json::Value;
use std::collections::BTreeMap;

pub fn register(map: &mut BTreeMap<String, BuiltinFn>) {
    map.insert(
        "keys".into(),
        Box::new(|_: &dyn EvalContext, args: &[RuntimeValue]| {
            if args.len() != 1 {
                return Err(arity_mismatch("keys", "1", args.len()));
            }
            let m = match &args[0].0 {
                Value::Object(o) => o,
                _ => return Err(type_error("keys", "arg must be object")),
            };
            Ok(RuntimeValue(Value::Array(
                m.keys().cloned().map(Value::String).collect(),
            )))
        }),
    );
    map.insert(
        "values".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("values", "1", args.len()));
            }
            let m = match &args[0].0 {
                Value::Object(o) => o,
                _ => return Err(type_error("values", "arg must be object")),
            };
            Ok(RuntimeValue(Value::Array(m.values().cloned().collect())))
        }),
    );
    map.insert(
        "has".into(),
        Box::new(|_, args| {
            if args.len() != 2 {
                return Err(arity_mismatch("has", "2", args.len()));
            }
            let m = match &args[0].0 {
                Value::Object(o) => o,
                _ => return Err(type_error("has", "first arg must be object")),
            };
            let k = args[1]
                .as_str()
                .ok_or_else(|| type_error("has", "second arg must be string"))?;
            Ok(RuntimeValue::from_bool(m.contains_key(k)))
        }),
    );
    map.insert(
        "merge".into(),
        Box::new(|_, args| {
            if args.len() != 2 {
                return Err(arity_mismatch("merge", "2", args.len()));
            }
            let a = match &args[0].0 {
                Value::Object(o) => o.clone(),
                _ => return Err(type_error("merge", "first arg must be object")),
            };
            let b = match &args[1].0 {
                Value::Object(o) => o.clone(),
                _ => return Err(type_error("merge", "second arg must be object")),
            };
            let mut out = a;
            for (k, v) in b {
                out.insert(k, v);
            }
            Ok(RuntimeValue(Value::Object(out)))
        }),
    );
}

#[cfg(test)]
mod tests {
    use crate::expression::scope::StateGraphContext;
    use crate::expression::{compiler::compile, parser::parse, vm::run};
    use crate::signal::scheduler::Scheduler;
    use crate::state::StateGraph;
    use crate::value::RuntimeValue;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    fn eval(src: &str) -> RuntimeValue {
        let sched = Rc::new(Scheduler::new());
        let state = StateGraph::new(sched);
        let locals = BTreeMap::new();
        let builtins = super::super::default_builtins();
        let ctx = StateGraphContext::new(&state, None, None, &locals, &builtins);
        let chunk = compile(&parse(src).unwrap()).unwrap();
        run(&chunk, &ctx).unwrap()
    }

    #[test]
    fn keys_values() {
        assert_eq!(eval(r#"keys({a:1, b:2})"#).0, serde_json::json!(["a", "b"]));
        assert_eq!(eval(r#"values({a:1, b:2})"#).0, serde_json::json!([1, 2]));
    }

    #[test]
    fn has() {
        assert_eq!(eval(r#"has({a:1}, "a")"#).as_bool(), Some(true));
        assert_eq!(eval(r#"has({a:1}, "b")"#).as_bool(), Some(false));
    }

    #[test]
    fn merge() {
        assert_eq!(
            eval(r#"merge({a:1}, {b:2, a:99})"#).0,
            serde_json::json!({"a": 99, "b": 2})
        );
    }
}
