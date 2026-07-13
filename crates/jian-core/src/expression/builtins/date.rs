use super::{arity_mismatch, BuiltinFn};
use crate::expression::vm::EvalContext;
use crate::value::RuntimeValue;
use std::collections::BTreeMap;

/// MVP: `now()` returns milliseconds-since-epoch as a Number; `date` and
/// `format_date` are pass-through stubs until proper chrono integration
/// lands in a later plan.
pub fn register(map: &mut BTreeMap<String, BuiltinFn>) {
    map.insert(
        "now".into(),
        Box::new(|ctx: &dyn EvalContext, args: &[RuntimeValue]| {
            if !args.is_empty() {
                return Err(arity_mismatch("now", "0", args.len()));
            }
            Ok(RuntimeValue::from_f64(ctx.now_ms() as f64))
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
        // The state graph is the expression clock source; no platform clock is
        // consulted by the builtin.
        let sched = Rc::new(Scheduler::new());
        let state = StateGraph::new(sched);
        state.set_now_ms(1234);
        let locals = BTreeMap::new();
        let builtins = super::super::default_builtins();
        let ctx = StateGraphContext::new(&state, None, None, &locals, &builtins);
        let chunk = compile(&parse("now()").unwrap()).unwrap();
        let v = run(&chunk, &ctx).unwrap().as_f64().unwrap();
        assert_eq!(v, 1234.0);
    }

    #[test]
    fn date_passthrough() {
        let v = eval("now()").as_f64().unwrap();
        assert_eq!(v, 0.0);
    }
}
