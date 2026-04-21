use super::{arity_mismatch, type_error, BuiltinFn};
use crate::expression::diag::Diagnostic;
use crate::expression::vm::EvalContext;
use crate::value::RuntimeValue;
use serde_json::Value;
use std::collections::BTreeMap;

fn as_array(v: &RuntimeValue) -> Option<&Vec<Value>> {
    match &v.0 {
        Value::Array(a) => Some(a),
        _ => None,
    }
}

pub fn register(map: &mut BTreeMap<String, BuiltinFn>) {
    map.insert(
        "first".into(),
        Box::new(|_: &dyn EvalContext, args: &[RuntimeValue]| {
            if args.len() != 1 {
                return Err(arity_mismatch("first", "1", args.len()));
            }
            Ok(RuntimeValue(
                as_array(&args[0])
                    .and_then(|a| a.first().cloned())
                    .unwrap_or(Value::Null),
            ))
        }),
    );
    map.insert(
        "last".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("last", "1", args.len()));
            }
            Ok(RuntimeValue(
                as_array(&args[0])
                    .and_then(|a| a.last().cloned())
                    .unwrap_or(Value::Null),
            ))
        }),
    );
    map.insert(
        "slice".into(),
        Box::new(|_, args| {
            let (arr, s, e) = match args.len() {
                2 => {
                    let a = as_array(&args[0])
                        .ok_or_else(|| type_error("slice", "first arg must be array"))?;
                    let s = args[1].as_i64().unwrap_or(0) as usize;
                    (a, s, a.len())
                }
                3 => {
                    let a = as_array(&args[0])
                        .ok_or_else(|| type_error("slice", "first arg must be array"))?;
                    let s = args[1].as_i64().unwrap_or(0) as usize;
                    let e = (args[2].as_i64().unwrap_or(0) as usize).min(a.len());
                    (a, s, e)
                }
                n => return Err(arity_mismatch("slice", "2 or 3", n)),
            };
            let clamped_s = s.min(arr.len());
            let end = e.max(clamped_s);
            Ok(RuntimeValue(Value::Array(arr[clamped_s..end].to_vec())))
        }),
    );
    map.insert(
        "reverse".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("reverse", "1", args.len()));
            }
            let a = as_array(&args[0]).ok_or_else(|| type_error("reverse", "arg must be array"))?;
            let mut out = a.clone();
            out.reverse();
            Ok(RuntimeValue(Value::Array(out)))
        }),
    );
    map.insert(
        "includes".into(),
        Box::new(|_, args| {
            if args.len() != 2 {
                return Err(arity_mismatch("includes", "2", args.len()));
            }
            let a = as_array(&args[0])
                .ok_or_else(|| type_error("includes", "first arg must be array"))?;
            let needle = &args[1].0;
            Ok(RuntimeValue::from_bool(a.iter().any(|v| v == needle)))
        }),
    );

    // HOF: filter / map / sort / reduce
    map.insert("filter".into(), Box::new(hof_filter));
    map.insert("map".into(), Box::new(hof_map));
    map.insert("sort".into(), Box::new(hof_sort));
    map.insert("reduce".into(), Box::new(hof_reduce));
}

// --- HOF implementations ---
// MVP: re-parse + re-compile the sub-expression per call. Task 15 adds caching.

use crate::expression::{compiler::compile, parser::parse};

fn extract_sub_expr(arg: &RuntimeValue) -> Result<String, Diagnostic> {
    match &arg.0 {
        Value::String(s) => Ok(s.clone()),
        _ => Err(type_error("HOF", "sub-expression arg must be a string")),
    }
}

fn run_sub_with_locals(
    ctx: &dyn EvalContext,
    source: &str,
    locals: &[(&str, RuntimeValue)],
) -> Result<RuntimeValue, Diagnostic> {
    let chunk = if let Some(cache) = ctx.cache() {
        cache.get_or_compile(source)?
    } else {
        let ast = parse(source)?;
        compile(&ast)?
    };

    struct Overlay<'a> {
        inner: &'a dyn EvalContext,
        locals: BTreeMap<String, RuntimeValue>,
    }
    impl<'a> EvalContext for Overlay<'a> {
        fn lookup_scope(&self, path: &str) -> Option<RuntimeValue> {
            // Handle dotted paths whose root is one of our locals (e.g.
            // `$item.title`) — walk into the local JSON value. Fall through to
            // `inner` for everything else (app state, $vars, etc.).
            if let Some(dot) = path.find('.') {
                let root = &path[..dot];
                let name = root.trim_start_matches('$');
                if let Some(local) = self
                    .locals
                    .get(name)
                    .cloned()
                    .or_else(|| self.locals.get(root).cloned())
                {
                    let rest = &path[dot + 1..];
                    let mut val = local.0;
                    for seg in rest.split('.') {
                        val = match val {
                            serde_json::Value::Object(ref m) => {
                                m.get(seg).cloned().unwrap_or(serde_json::Value::Null)
                            }
                            _ => serde_json::Value::Null,
                        };
                    }
                    return Some(RuntimeValue(val));
                }
            } else {
                let name = path.trim_start_matches('$');
                if let Some(v) = self
                    .locals
                    .get(name)
                    .cloned()
                    .or_else(|| self.locals.get(path).cloned())
                {
                    return Some(v);
                }
            }
            self.inner.lookup_scope(path)
        }
        fn call_builtin(
            &self,
            name: &str,
            args: &[RuntimeValue],
        ) -> Option<Result<RuntimeValue, Diagnostic>> {
            self.inner.call_builtin(name, args)
        }
        fn warn(&self, d: Diagnostic) {
            self.inner.warn(d);
        }
        fn cache(&self) -> Option<&crate::expression::cache::ExpressionCache> {
            self.inner.cache()
        }
    }

    let overlay = Overlay {
        inner: ctx,
        locals: locals.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
    };
    crate::expression::vm::run(&chunk, &overlay)
}

fn hof_filter(
    ctx: &dyn EvalContext,
    args: &[RuntimeValue],
) -> Result<RuntimeValue, Diagnostic> {
    if args.len() != 2 {
        return Err(arity_mismatch("filter", "2", args.len()));
    }
    let arr = as_array(&args[0]).ok_or_else(|| type_error("filter", "first arg must be array"))?;
    let source = extract_sub_expr(&args[1])?;
    let mut out = Vec::new();
    for (i, v) in arr.iter().enumerate() {
        let keep = run_sub_with_locals(
            ctx,
            &source,
            &[
                ("item", RuntimeValue(v.clone())),
                ("index", RuntimeValue::from_i64(i as i64)),
            ],
        )?;
        let truthy = match &keep.0 {
            Value::Null | Value::Bool(false) => false,
            Value::Bool(true) => true,
            Value::Number(n) => n.as_f64().map(|x| x != 0.0).unwrap_or(false),
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
        };
        if truthy {
            out.push(v.clone());
        }
    }
    Ok(RuntimeValue(Value::Array(out)))
}

fn hof_map(ctx: &dyn EvalContext, args: &[RuntimeValue]) -> Result<RuntimeValue, Diagnostic> {
    if args.len() != 2 {
        return Err(arity_mismatch("map", "2", args.len()));
    }
    let arr = as_array(&args[0]).ok_or_else(|| type_error("map", "first arg must be array"))?;
    let source = extract_sub_expr(&args[1])?;
    let mut out = Vec::with_capacity(arr.len());
    for (i, v) in arr.iter().enumerate() {
        let mapped = run_sub_with_locals(
            ctx,
            &source,
            &[
                ("item", RuntimeValue(v.clone())),
                ("index", RuntimeValue::from_i64(i as i64)),
            ],
        )?;
        out.push(mapped.0);
    }
    Ok(RuntimeValue(Value::Array(out)))
}

fn hof_sort(ctx: &dyn EvalContext, args: &[RuntimeValue]) -> Result<RuntimeValue, Diagnostic> {
    let (arr, key_source) = match args.len() {
        1 => (
            as_array(&args[0])
                .ok_or_else(|| type_error("sort", "first arg must be array"))?,
            None,
        ),
        2 => (
            as_array(&args[0])
                .ok_or_else(|| type_error("sort", "first arg must be array"))?,
            Some(extract_sub_expr(&args[1])?),
        ),
        n => return Err(arity_mismatch("sort", "1 or 2", n)),
    };
    let mut pairs: Vec<(f64, Value)> = Vec::with_capacity(arr.len());
    for (i, v) in arr.iter().enumerate() {
        let key = if let Some(ref src) = key_source {
            run_sub_with_locals(
                ctx,
                src,
                &[
                    ("item", RuntimeValue(v.clone())),
                    ("index", RuntimeValue::from_i64(i as i64)),
                ],
            )?
            .as_f64()
            .unwrap_or(0.0)
        } else {
            v.as_f64().unwrap_or(0.0)
        };
        pairs.push((key, v.clone()));
    }
    pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(RuntimeValue(Value::Array(
        pairs.into_iter().map(|(_, v)| v).collect(),
    )))
}

fn hof_reduce(
    ctx: &dyn EvalContext,
    args: &[RuntimeValue],
) -> Result<RuntimeValue, Diagnostic> {
    if args.len() != 3 {
        return Err(arity_mismatch("reduce", "3", args.len()));
    }
    let arr = as_array(&args[0]).ok_or_else(|| type_error("reduce", "first arg must be array"))?;
    let seed = args[1].clone();
    let source = extract_sub_expr(&args[2])?;
    let mut acc = seed;
    for (i, v) in arr.iter().enumerate() {
        acc = run_sub_with_locals(
            ctx,
            &source,
            &[
                ("acc", acc.clone()),
                ("item", RuntimeValue(v.clone())),
                ("index", RuntimeValue::from_i64(i as i64)),
            ],
        )?;
    }
    Ok(acc)
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
    fn filter_kept() {
        assert_eq!(
            eval(r#"filter([1,2,3,4], "$item > 2")"#).0,
            serde_json::json!([3, 4])
        );
    }

    #[test]
    fn map_doubled() {
        assert_eq!(
            eval(r#"map([1,2,3], "$item * 2")"#).0,
            serde_json::json!([2, 4, 6])
        );
    }

    #[test]
    fn sort_simple() {
        assert_eq!(eval("sort([3, 1, 2])").0, serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn sort_by_key() {
        let v = eval(r#"sort([{a:3},{a:1},{a:2}], "$item.a")"#);
        assert_eq!(v.0, serde_json::json!([{"a":1},{"a":2},{"a":3}]));
    }

    #[test]
    fn reduce_sum() {
        assert_eq!(
            eval(r#"reduce([1,2,3,4], 0, "$acc + $item")"#).as_f64(),
            Some(10.0)
        );
    }

    #[test]
    fn slice_mid() {
        assert_eq!(
            eval("slice([10,20,30,40], 1, 3)").0,
            serde_json::json!([20, 30])
        );
    }

    #[test]
    fn first_last() {
        assert_eq!(eval("first([1,2,3])").as_i64(), Some(1));
        assert_eq!(eval("last([1,2,3])").as_i64(), Some(3));
    }

    #[test]
    fn includes_fn() {
        assert_eq!(eval("includes([1,2,3], 2)").as_bool(), Some(true));
        assert_eq!(eval("includes([1,2,3], 99)").as_bool(), Some(false));
    }
}
