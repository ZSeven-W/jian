use super::{arity_mismatch, BuiltinFn};
use crate::expression::vm::EvalContext;
use crate::value::RuntimeValue;
use serde_json::Value;
use std::collections::BTreeMap;

fn to_f(v: &RuntimeValue) -> f64 {
    match &v.0 {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

pub fn register(map: &mut BTreeMap<String, BuiltinFn>) {
    map.insert(
        "abs".into(),
        Box::new(|_: &dyn EvalContext, args: &[RuntimeValue]| {
            if args.len() != 1 {
                return Err(arity_mismatch("abs", "1", args.len()));
            }
            Ok(RuntimeValue::from_f64(to_f(&args[0]).abs()))
        }),
    );
    map.insert(
        "min".into(),
        Box::new(|_, args| {
            if args.is_empty() {
                return Err(arity_mismatch("min", "1+", 0));
            }
            let mut m = f64::INFINITY;
            for a in args {
                let v = to_f(a);
                if v < m {
                    m = v;
                }
            }
            Ok(RuntimeValue::from_f64(m))
        }),
    );
    map.insert(
        "max".into(),
        Box::new(|_, args| {
            if args.is_empty() {
                return Err(arity_mismatch("max", "1+", 0));
            }
            let mut m = f64::NEG_INFINITY;
            for a in args {
                let v = to_f(a);
                if v > m {
                    m = v;
                }
            }
            Ok(RuntimeValue::from_f64(m))
        }),
    );
    map.insert(
        "floor".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("floor", "1", args.len()));
            }
            Ok(RuntimeValue::from_f64(to_f(&args[0]).floor()))
        }),
    );
    map.insert(
        "ceil".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("ceil", "1", args.len()));
            }
            Ok(RuntimeValue::from_f64(to_f(&args[0]).ceil()))
        }),
    );
    map.insert(
        "round".into(),
        Box::new(|_, args| {
            let (x, digits) = match args.len() {
                1 => (to_f(&args[0]), 0),
                2 => (to_f(&args[0]), to_f(&args[1]) as i32),
                n => return Err(arity_mismatch("round", "1 or 2", n)),
            };
            let factor = 10f64.powi(digits);
            Ok(RuntimeValue::from_f64((x * factor).round() / factor))
        }),
    );
    map.insert(
        "clamp".into(),
        Box::new(|_, args| {
            if args.len() != 3 {
                return Err(arity_mismatch("clamp", "3", args.len()));
            }
            let (x, lo, hi) = (to_f(&args[0]), to_f(&args[1]), to_f(&args[2]));
            Ok(RuntimeValue::from_f64(x.max(lo).min(hi)))
        }),
    );
    map.insert(
        "lerp".into(),
        Box::new(|_, args| {
            if args.len() != 3 {
                return Err(arity_mismatch("lerp", "3", args.len()));
            }
            let (a, b, t) = (to_f(&args[0]), to_f(&args[1]), to_f(&args[2]));
            Ok(RuntimeValue::from_f64(a + (b - a) * t))
        }),
    );
    map.insert(
        "sum".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("sum", "1", args.len()));
            }
            let arr = match &args[0].0 {
                Value::Array(a) => a,
                _ => return Ok(RuntimeValue::from_f64(0.0)),
            };
            let total: f64 = arr.iter().filter_map(|v| v.as_f64()).sum();
            Ok(RuntimeValue::from_f64(total))
        }),
    );
    map.insert(
        "avg".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("avg", "1", args.len()));
            }
            let arr = match &args[0].0 {
                Value::Array(a) => a,
                _ => return Ok(RuntimeValue::null()),
            };
            if arr.is_empty() {
                return Ok(RuntimeValue::null());
            }
            let total: f64 = arr.iter().filter_map(|v| v.as_f64()).sum();
            Ok(RuntimeValue::from_f64(total / arr.len() as f64))
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
    fn abs() {
        assert_eq!(eval("abs(-5)").as_f64(), Some(5.0));
    }
    #[test]
    fn min() {
        assert_eq!(eval("min(3, 1, 2)").as_f64(), Some(1.0));
    }
    #[test]
    fn max() {
        assert_eq!(eval("max(3, 1, 2)").as_f64(), Some(3.0));
    }
    #[test]
    fn floor() {
        assert_eq!(eval("floor(3.7)").as_f64(), Some(3.0));
    }
    #[test]
    fn ceil() {
        assert_eq!(eval("ceil(3.2)").as_f64(), Some(4.0));
    }
    #[test]
    fn round_test() {
        assert_eq!(eval("round(3.456, 1)").as_f64(), Some(3.5));
    }
    #[test]
    fn clamp_hi() {
        assert_eq!(eval("clamp(100, 0, 10)").as_f64(), Some(10.0));
    }
    #[test]
    fn clamp_lo() {
        assert_eq!(eval("clamp(-5, 0, 10)").as_f64(), Some(0.0));
    }
    #[test]
    fn lerp_fn() {
        assert_eq!(eval("lerp(0, 10, 0.5)").as_f64(), Some(5.0));
    }
    #[test]
    fn sum_arr() {
        assert_eq!(eval("sum([1,2,3,4])").as_f64(), Some(10.0));
    }
    #[test]
    fn avg_arr() {
        assert_eq!(eval("avg([2,4,6])").as_f64(), Some(4.0));
    }
}
