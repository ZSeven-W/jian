use super::{arity_mismatch, type_error, BuiltinFn};
use crate::expression::vm::EvalContext;
use crate::value::RuntimeValue;
use serde_json::Value;
use std::collections::BTreeMap;

fn to_s(v: &RuntimeValue) -> String {
    match &v.0 {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e17 => {
                format!("{}", f as i64)
            }
            Some(f) => format!("{}", f),
            None => n.to_string(),
        },
        other => other.to_string(),
    }
}

pub fn register(map: &mut BTreeMap<String, BuiltinFn>) {
    map.insert(
        "len".into(),
        Box::new(|_: &dyn EvalContext, args: &[RuntimeValue]| {
            if args.len() != 1 {
                return Err(arity_mismatch("len", "1", args.len()));
            }
            let n = match &args[0].0 {
                Value::String(s) => s.chars().count() as i64,
                Value::Array(a) => a.len() as i64,
                Value::Object(o) => o.len() as i64,
                _ => 0,
            };
            Ok(RuntimeValue::from_i64(n))
        }),
    );
    map.insert(
        "upper".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("upper", "1", args.len()));
            }
            Ok(RuntimeValue::from_string(to_s(&args[0]).to_uppercase()))
        }),
    );
    map.insert(
        "lower".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("lower", "1", args.len()));
            }
            Ok(RuntimeValue::from_string(to_s(&args[0]).to_lowercase()))
        }),
    );
    map.insert(
        "trim".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("trim", "1", args.len()));
            }
            Ok(RuntimeValue::from_string(to_s(&args[0]).trim().to_owned()))
        }),
    );
    map.insert(
        "split".into(),
        Box::new(|_, args| {
            if args.len() != 2 {
                return Err(arity_mismatch("split", "2", args.len()));
            }
            let s = to_s(&args[0]);
            let sep = to_s(&args[1]);
            let parts: Vec<Value> = s.split(&sep).map(|p| Value::String(p.to_owned())).collect();
            Ok(RuntimeValue(Value::Array(parts)))
        }),
    );
    map.insert(
        "join".into(),
        Box::new(|_, args| {
            if args.len() != 2 {
                return Err(arity_mismatch("join", "2", args.len()));
            }
            let arr = match &args[0].0 {
                Value::Array(a) => a,
                _ => return Err(type_error("join", "first arg must be array")),
            };
            let sep = to_s(&args[1]);
            let parts: Vec<String> = arr.iter().map(|v| to_s(&RuntimeValue(v.clone()))).collect();
            Ok(RuntimeValue::from_string(parts.join(&sep)))
        }),
    );
    map.insert(
        "replace".into(),
        Box::new(|_, args| {
            if args.len() != 3 {
                return Err(arity_mismatch("replace", "3", args.len()));
            }
            let s = to_s(&args[0]);
            let pat = to_s(&args[1]);
            let rep = to_s(&args[2]);
            Ok(RuntimeValue::from_string(s.replace(&pat, &rep)))
        }),
    );
    map.insert(
        "contains".into(),
        Box::new(|_, args| {
            if args.len() != 2 {
                return Err(arity_mismatch("contains", "2", args.len()));
            }
            let hay = to_s(&args[0]);
            let needle = to_s(&args[1]);
            Ok(RuntimeValue::from_bool(hay.contains(&needle)))
        }),
    );
    map.insert(
        "startsWith".into(),
        Box::new(|_, args| {
            if args.len() != 2 {
                return Err(arity_mismatch("startsWith", "2", args.len()));
            }
            Ok(RuntimeValue::from_bool(
                to_s(&args[0]).starts_with(&to_s(&args[1])),
            ))
        }),
    );
    map.insert(
        "endsWith".into(),
        Box::new(|_, args| {
            if args.len() != 2 {
                return Err(arity_mismatch("endsWith", "2", args.len()));
            }
            Ok(RuntimeValue::from_bool(
                to_s(&args[0]).ends_with(&to_s(&args[1])),
            ))
        }),
    );
    map.insert(
        "format".into(),
        Box::new(|_, args| {
            if args.is_empty() {
                return Err(arity_mismatch("format", "1+", 0));
            }
            let mut tpl = to_s(&args[0]);
            for (i, a) in args[1..].iter().enumerate() {
                tpl = tpl.replace(&format!("{{{}}}", i), &to_s(a));
            }
            Ok(RuntimeValue::from_string(tpl))
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
    fn len_str() {
        assert_eq!(eval("len(\"hello\")").as_i64(), Some(5));
    }
    #[test]
    fn upper() {
        assert_eq!(eval("upper(\"hi\")").as_str(), Some("HI"));
    }
    #[test]
    fn split_fn() {
        assert_eq!(eval("len(split(\"a,b,c\", \",\"))").as_i64(), Some(3));
    }
    #[test]
    fn join_fn() {
        assert_eq!(
            eval("join([\"a\",\"b\",\"c\"], \"-\")").as_str(),
            Some("a-b-c")
        );
    }
    #[test]
    fn replace_fn() {
        assert_eq!(
            eval("replace(\"foo bar foo\", \"foo\", \"X\")").as_str(),
            Some("X bar X")
        );
    }
    #[test]
    fn contains_fn() {
        assert_eq!(
            eval("contains(\"hello world\", \"world\")").as_bool(),
            Some(true)
        );
    }
    #[test]
    fn starts_fn() {
        assert_eq!(eval("startsWith(\"hello\", \"hel\")").as_bool(), Some(true));
    }
    #[test]
    fn format_pos() {
        assert_eq!(
            eval("format(\"{0} + {1} = {2}\", 1, 2, 3)").as_str(),
            Some("1 + 2 = 3")
        );
    }
}
