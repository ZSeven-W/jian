use super::{arity_mismatch, BuiltinFn};
use crate::expression::vm::EvalContext;
use crate::value::RuntimeValue;
use serde_json::Value;
use std::collections::BTreeMap;

pub fn register(map: &mut BTreeMap<String, BuiltinFn>) {
    map.insert(
        "typeof".into(),
        Box::new(|_: &dyn EvalContext, args: &[RuntimeValue]| {
            if args.len() != 1 {
                return Err(arity_mismatch("typeof", "1", args.len()));
            }
            let t = match &args[0].0 {
                Value::Null => "null",
                Value::Bool(_) => "bool",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            };
            Ok(RuntimeValue::from_string(t))
        }),
    );
    map.insert(
        "is_null".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("is_null", "1", args.len()));
            }
            Ok(RuntimeValue::from_bool(args[0].is_null()))
        }),
    );
    map.insert(
        "to_num".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("to_num", "1", args.len()));
            }
            let n = match &args[0].0 {
                Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
                Value::String(s) => s.parse::<f64>().unwrap_or(f64::NAN),
                Value::Bool(b) => {
                    if *b {
                        1.0
                    } else {
                        0.0
                    }
                }
                Value::Null => 0.0,
                _ => f64::NAN,
            };
            if n.is_nan() {
                Ok(RuntimeValue::null())
            } else {
                Ok(RuntimeValue::from_f64(n))
            }
        }),
    );
    map.insert(
        "to_str".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("to_str", "1", args.len()));
            }
            let s = match &args[0].0 {
                Value::String(s) => s.clone(),
                Value::Null => "null".to_owned(),
                Value::Number(n) => match n.as_f64() {
                    Some(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e17 => {
                        format!("{}", f as i64)
                    }
                    Some(f) => format!("{}", f),
                    None => n.to_string(),
                },
                other => other.to_string(),
            };
            Ok(RuntimeValue::from_string(s))
        }),
    );
    map.insert(
        "to_bool".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("to_bool", "1", args.len()));
            }
            let b = match &args[0].0 {
                Value::Bool(b) => *b,
                Value::Null => false,
                Value::Number(n) => n.as_f64().map(|x| x != 0.0).unwrap_or(false),
                Value::String(s) => !s.is_empty(),
                Value::Array(a) => !a.is_empty(),
                Value::Object(o) => !o.is_empty(),
            };
            Ok(RuntimeValue::from_bool(b))
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
    fn typeof_() {
        assert_eq!(eval(r#"typeof("x")"#).as_str(), Some("string"));
        assert_eq!(eval("typeof(42)").as_str(), Some("number"));
        assert_eq!(eval("typeof(null)").as_str(), Some("null"));
        assert_eq!(eval("typeof([1,2])").as_str(), Some("array"));
    }
    #[test]
    fn to_num_parse() {
        assert_eq!(eval(r#"to_num("42")"#).as_f64(), Some(42.0));
    }
    #[test]
    fn to_num_bad_null() {
        assert!(eval(r#"to_num("nope")"#).is_null());
    }
    #[test]
    fn to_str_num() {
        assert_eq!(eval("to_str(3.14)").as_str(), Some("3.14"));
    }
    #[test]
    fn to_bool_truthy() {
        assert_eq!(eval(r#"to_bool("x")"#).as_bool(), Some(true));
        assert_eq!(eval(r#"to_bool("")"#).as_bool(), Some(false));
        assert_eq!(eval("to_bool([])").as_bool(), Some(false));
    }
}
