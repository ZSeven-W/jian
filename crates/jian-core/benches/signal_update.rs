use criterion::{criterion_group, criterion_main, Criterion};
use jian_core::effect::{EffectHandle, EffectRegistry};
use jian_core::signal::{scheduler::Scheduler, Signal};
use jian_core::value::RuntimeValue;
use serde_json::json;
use std::cell::Cell;
use std::rc::Rc;

fn setup(
    n_subs: usize,
) -> (
    Rc<Scheduler>,
    Rc<EffectRegistry>,
    Signal<RuntimeValue>,
    Vec<EffectHandle>,
) {
    let sched = Rc::new(Scheduler::new());
    let reg = EffectRegistry::new();
    reg.install_on(&sched);
    let sig = Signal::new(RuntimeValue::from_i64(0), sched.clone());

    let hits = Rc::new(Cell::new(0));
    let mut handles = Vec::with_capacity(n_subs);
    for _ in 0..n_subs {
        let s2 = sig.clone();
        let h = hits.clone();
        let handle = reg.register(move || {
            let _ = s2.get();
            h.set(h.get() + 1);
        });
        handles.push(handle);
    }
    (sched, reg, sig, handles)
}

fn bench(c: &mut Criterion) {
    for &n in &[10, 100, 1000] {
        c.bench_function(&format!("signal_set_flush_{}_subs", n), |b| {
            let (sched, _reg, sig, _h) = setup(n);
            b.iter(|| {
                sig.set(RuntimeValue(json!(42)));
                sched.flush();
            });
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
