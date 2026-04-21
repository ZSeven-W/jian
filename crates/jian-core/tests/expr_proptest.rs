//! Property-based tests for the Tier 1 expression language.
//!
//! We don't generate random source strings (that would mostly produce parse
//! errors). Instead we generate random strings from a grammar, parse them,
//! and verify the full pipeline never panics.

use jian_core::expression::Expression;
use proptest::prelude::*;

fn arb_leaf() -> impl Strategy<Value = String> {
    prop_oneof![
        (-1000i64..1000i64).prop_map(|n| n.to_string()),
        proptest::string::string_regex("[a-z]{1,3}")
            .unwrap()
            .prop_map(|s| format!("\"{}\"", s)),
        Just("true".to_owned()),
        Just("false".to_owned()),
        Just("null".to_owned()),
    ]
}

fn arb_binop() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("+"),
        Just("-"),
        Just("*"),
        Just("/"),
        Just("%"),
        Just("=="),
        Just("!="),
        Just(">"),
        Just("<"),
        Just(">="),
        Just("<="),
        Just("&&"),
        Just("||"),
        Just("??"),
    ]
}

fn arb_expr() -> impl Strategy<Value = String> {
    let leaf = arb_leaf();
    leaf.prop_recursive(3, 32, 2, |inner| {
        prop_oneof![
            (inner.clone(), arb_binop(), inner.clone())
                .prop_map(|(l, op, r)| format!("({} {} {})", l, op, r)),
            (inner.clone(), inner.clone(), inner.clone())
                .prop_map(|(c, t, e)| format!("({} ? {} : {})", c, t, e)),
            inner.clone().prop_map(|e| format!("!({})", e)),
            inner.clone().prop_map(|e| format!("-({})", e)),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn random_expressions_never_panic(src in arb_expr()) {
        let sched = std::rc::Rc::new(jian_core::signal::scheduler::Scheduler::new());
        let state = jian_core::state::StateGraph::new(sched);
        match Expression::compile(&src) {
            Err(_) => {}
            Ok(expr) => {
                let (_v, _warnings) = expr.eval(&state, None, None);
            }
        }
    }
}
