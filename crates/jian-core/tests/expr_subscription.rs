use jian_core::effect::EffectRegistry;
use jian_core::expression::Expression;
use jian_core::signal::scheduler::Scheduler;
use jian_core::state::StateGraph;
use serde_json::json;
use std::cell::Cell;
use std::rc::Rc;

#[test]
fn changing_a_does_not_trigger_effect_reading_only_b() {
    let sched = Rc::new(Scheduler::new());
    let reg = EffectRegistry::new();
    reg.install_on(&sched);
    let state = Rc::new(StateGraph::new(sched.clone()));

    state.app_set("a", json!(1));
    state.app_set("b", json!(100));

    let expr = Expression::compile("$app.b * 2").unwrap();
    let runs = Rc::new(Cell::new(0));
    let state2 = state.clone();
    let runs2 = runs.clone();
    let _h = reg.register(move || {
        let (v, _) = expr.eval(&state2, None, None);
        runs2.set(runs2.get() + 1);
        assert_eq!(
            v.as_i64(),
            Some(state2.app_get("b").unwrap().as_i64().unwrap() * 2)
        );
    });
    let before = runs.get();

    // Change $app.a only. Should NOT trigger the effect.
    state.app_set("a", json!(999));
    sched.flush();
    assert_eq!(
        runs.get(),
        before,
        "effect should not re-run on unrelated var change"
    );

    // Change $app.b. Should trigger.
    state.app_set("b", json!(200));
    sched.flush();
    assert_eq!(
        runs.get(),
        before + 1,
        "effect should re-run when b changes"
    );
}
