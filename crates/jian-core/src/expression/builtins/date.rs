use super::{arity_mismatch, BuiltinFn};
use crate::expression::vm::EvalContext;
use crate::value::RuntimeValue;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// MVP: `now()` returns milliseconds-since-epoch as a Number; `date` and
/// `format_date` are pass-through stubs until proper chrono integration
/// lands in a later plan.
pub fn register(map: &mut BTreeMap<String, BuiltinFn>) {
    map.insert(
        "now".into(),
        Box::new(|_: &dyn EvalContext, args: &[RuntimeValue]| {
            if !args.is_empty() {
                return Err(arity_mismatch("now", "0", args.len()));
            }
            let ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as f64)
                .unwrap_or(0.0);
            Ok(RuntimeValue::from_f64(ms))
        }),
    );
    map.insert(
        "date".into(),
        Box::new(|_, args| {
            if args.len() != 1 {
                return Err(arity_mismatch("date", "1", args.len()));
            }
            Ok(args[0].clone())
        }),
    );
    map.insert(
        "format_date".into(),
        Box::new(|_, args| {
            if args.len() != 2 {
                return Err(arity_mismatch("format_date", "2", args.len()));
            }
            Ok(args[0].clone())
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
    fn now_positive() {
        let v = eval("now()").as_f64().unwrap();
        assert!(v > 0.0);
    }
}
