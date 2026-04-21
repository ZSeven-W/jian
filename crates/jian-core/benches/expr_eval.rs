use criterion::{criterion_group, criterion_main, Criterion};
use jian_core::expression::Expression;
use jian_core::signal::scheduler::Scheduler;
use jian_core::state::StateGraph;
use serde_json::json;
use std::rc::Rc;

fn bench(c: &mut Criterion) {
    let sched = Rc::new(Scheduler::new());
    let state = StateGraph::new(sched);
    state.app_set("count", json!(42));
    state.app_set("active", json!(true));
    state.app_set("items", json!([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]));

    let cases = [
        ("literal", "42"),
        ("arith", "1 + 2 * 3"),
        ("scope_member", "$app.count + 1"),
        ("ternary", "$app.active ? $app.count * 2 : 0"),
        ("template", "`count=${$app.count}`"),
        ("filter", "len(filter($app.items, \"$item > 5\"))"),
        ("reduce", "reduce($app.items, 0, \"$acc + $item\")"),
    ];

    for (name, src) in cases {
        let expr = Expression::compile(src).unwrap();
        c.bench_function(&format!("eval_{}", name), |b| {
            b.iter(|| {
                let _ = expr.eval(&state, None, None);
            });
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
