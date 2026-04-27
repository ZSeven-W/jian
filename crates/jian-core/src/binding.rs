//! BindingEffect — attaches a compiled Expression to a target mutation
//! callback and registers it with the Effect registry.
//!
//! Semantics: every time any Signal read during the Expression's evaluation
//! changes, the effect re-runs, recomputes the value, and calls the target
//! callback with the new value. Integrators (scene-property wiring) provide
//! the callback.
//!
//! ## Lazy and deferred variants (Plan 19 Task 3)
//!
//! - [`BindingEffect::new`] is the **eager** path: caller pre-compiles via
//!   [`Expression::compile`] and pays the parse + compile cost up-front.
//! - [`BindingEffect::new_lazy`] uses the runtime's [`ExpressionCache`] —
//!   the source string is compiled on the *first* effect run, deduplicated
//!   across every binding that shares the same source. A document with
//!   200 nodes all bound to `$app.darkMode` compiles the expression once,
//!   not 200 times.
//! - [`DeferredBindingQueue`] holds `(source, apply)` pairs that don't
//!   need to evaluate during the cold-start critical path — typically
//!   bindings on nodes that aren't in the first-frame visible set. After
//!   `StartupPhase::EventPumpReady` fires, the host drains the queue
//!   into real `BindingEffect::new_lazy` registrations one entry at a
//!   time, so the (potentially expensive) compile + initial-eval cost
//!   spreads across post-paint frames instead of blocking the first
//!   render.

use crate::effect::{EffectHandle, EffectRegistry};
use crate::expression::{Diagnostic, Expression, ExpressionCache};
use crate::state::StateGraph;
use crate::value::RuntimeValue;
use std::cell::RefCell;
use std::rc::Rc;

/// Apply callback shape: receives the freshly-evaluated value plus any
/// runtime / compile diagnostics produced during evaluation.
pub type ApplyFn = dyn FnMut(RuntimeValue, Vec<Diagnostic>) + 'static;

pub struct BindingEffect {
    _handle: EffectHandle,
}

impl BindingEffect {
    /// Eager constructor: caller has already compiled the source via
    /// [`Expression::compile`] and supplies the resulting [`Expression`].
    /// The effect runs once at register time to discover signal deps,
    /// then re-runs on every dependency change.
    pub fn new(
        reg: &Rc<EffectRegistry>,
        expr: Expression,
        state: Rc<StateGraph>,
        page_id: Option<String>,
        node_id: Option<String>,
        apply: impl FnMut(RuntimeValue, Vec<Diagnostic>) + 'static,
    ) -> Self {
        let apply = RefCell::new(apply);
        let handle = reg.register(move || {
            let (v, warnings) = expr.eval(&state, page_id.as_deref(), node_id.as_deref());
            (apply.borrow_mut())(v, warnings);
        });
        Self { _handle: handle }
    }

    /// Lazy constructor: the source string is compiled through `cache` on
    /// the first effect run. Identical sources share a single
    /// [`crate::expression::Chunk`]; compile errors are surfaced as
    /// diagnostics on the apply callback (the value is `null`).
    ///
    /// Use this when binding registration happens at scale (e.g. scanning
    /// every node in a document during schema load): the cache amortises
    /// the parse + compile cost across every binding that shares a source.
    pub fn new_lazy<F>(
        reg: &Rc<EffectRegistry>,
        cache: Rc<ExpressionCache>,
        source: String,
        state: Rc<StateGraph>,
        page_id: Option<String>,
        node_id: Option<String>,
        apply: F,
    ) -> Self
    where
        F: FnMut(RuntimeValue, Vec<Diagnostic>) + 'static,
    {
        Self::new_lazy_boxed(reg, cache, source, state, page_id, node_id, Box::new(apply))
    }

    /// Internal entry point shared by `new_lazy` and
    /// `DeferredBindingQueue::drain_into_effects`. Keeps the public surface
    /// flexible (`impl FnMut`) while letting the queue store boxed
    /// callbacks without re-wrapping at drain time.
    fn new_lazy_boxed(
        reg: &Rc<EffectRegistry>,
        cache: Rc<ExpressionCache>,
        source: String,
        state: Rc<StateGraph>,
        page_id: Option<String>,
        node_id: Option<String>,
        apply: Box<ApplyFn>,
    ) -> Self {
        let apply = RefCell::new(apply);
        // Compile-once memo. `None` means "not yet compiled"; `Some(_)`
        // means we already paid the cost via `cache` and the resulting
        // Expression is reused on every subsequent effect run.
        let expr_cell: RefCell<Option<Expression>> = RefCell::new(None);
        let handle = reg.register(move || {
            if expr_cell.borrow().is_none() {
                match cache.get_or_compile(&source) {
                    Ok(chunk) => {
                        *expr_cell.borrow_mut() = Some(Expression {
                            source: source.clone(),
                            chunk,
                        });
                    }
                    Err(d) => {
                        // Compile error → null value + diagnostic. The
                        // effect still fires (so the integrator can show
                        // a fallback), but no chunk is cached and the
                        // closure will retry compile on the next run.
                        (apply.borrow_mut())(RuntimeValue::null(), vec![d]);
                        return;
                    }
                }
            }
            let expr_ref = expr_cell.borrow();
            let expr = expr_ref.as_ref().expect("expr populated above");
            let (v, warnings) = expr.eval(&state, page_id.as_deref(), node_id.as_deref());
            drop(expr_ref);
            (apply.borrow_mut())(v, warnings);
        });
        Self { _handle: handle }
    }
}

/// One entry in [`DeferredBindingQueue`]. The apply callback is boxed so
/// the queue can be drained without per-entry generic monomorphisation.
///
/// Crate-private with private fields: the only construction path is
/// [`DeferredBindingQueue::push`], the only consumption path is
/// [`DeferredBindingQueue::drain_into_effects`], and both live in this
/// module so direct field access works without a `pub` annotation.
/// Keeping the fields private avoids the public-fields-on-private-struct
/// asymmetry Codex round 2 flagged.
pub(crate) struct DeferredBinding {
    source: String,
    page: Option<String>,
    node: Option<String>,
    apply: Box<ApplyFn>,
}

/// FIFO queue of binding registrations whose evaluation is deferred past
/// the cold-start critical path.
///
/// Plan 19 Task 3 use case: during schema load, off-viewport bindings are
/// pushed into this queue with their source strings (no compile). After
/// `StartupPhase::EventPumpReady` fires, the host calls
/// [`Self::drain_into_effects`] which converts each entry into a real
/// [`BindingEffect::new_lazy`] registration via the runtime's
/// [`ExpressionCache`]. The cost (parse + compile + first eval + signal
/// subscription) spreads across post-paint idle frames instead of
/// blocking first paint.
#[derive(Default)]
pub struct DeferredBindingQueue {
    entries: Vec<DeferredBinding>,
}

impl DeferredBindingQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an entry. Generic over the apply callback so call sites
    /// don't have to box manually.
    pub fn push<F>(
        &mut self,
        source: impl Into<String>,
        page: Option<String>,
        node: Option<String>,
        apply: F,
    ) where
        F: FnMut(RuntimeValue, Vec<Diagnostic>) + 'static,
    {
        self.entries.push(DeferredBinding {
            source: source.into(),
            page,
            node,
            apply: Box::new(apply),
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drain every queued entry into a registered `BindingEffect`. The
    /// returned vector must be kept alive for the runtime's lifetime —
    /// dropping a `BindingEffect` deregisters the underlying effect and
    /// breaks reactivity.
    #[must_use = "keep the returned BindingEffect handles alive; dropping them \
                  deregisters the drained bindings and silently disables reactivity"]
    pub fn drain_into_effects(
        &mut self,
        reg: &Rc<EffectRegistry>,
        cache: Rc<ExpressionCache>,
        state: Rc<StateGraph>,
    ) -> Vec<BindingEffect> {
        let mut out = Vec::with_capacity(self.entries.len());
        for entry in self.entries.drain(..) {
            let DeferredBinding {
                source,
                page,
                node,
                apply,
            } = entry;
            out.push(BindingEffect::new_lazy_boxed(
                reg,
                Rc::clone(&cache),
                source,
                Rc::clone(&state),
                page,
                node,
                apply,
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::EffectRegistry;
    use crate::signal::scheduler::Scheduler;
    use serde_json::json;
    use std::cell::RefCell;

    fn fixture() -> (
        Rc<Scheduler>,
        Rc<EffectRegistry>,
        Rc<StateGraph>,
        Rc<ExpressionCache>,
    ) {
        let sched = Rc::new(Scheduler::new());
        let reg = EffectRegistry::new();
        reg.install_on(&sched);
        let state = Rc::new(StateGraph::new(sched.clone()));
        let cache = Rc::new(ExpressionCache::new());
        (sched, reg, state, cache)
    }

    #[test]
    fn binding_updates_target_on_signal_change() {
        let (sched, reg, state, _cache) = fixture();
        state.app_set("count", json!(1));

        let last = Rc::new(RefCell::new(RuntimeValue::null()));
        let last2 = last.clone();
        let expr = Expression::compile("$app.count * 2").unwrap();
        let _b = BindingEffect::new(&reg, expr, state.clone(), None, None, move |v, _| {
            *last2.borrow_mut() = v;
        });

        assert_eq!(last.borrow().as_i64(), Some(2));

        state.app_set("count", json!(5));
        sched.flush();
        assert_eq!(last.borrow().as_i64(), Some(10));
    }

    #[test]
    fn binding_warnings_flow_through() {
        let (_sched, reg, state, _cache) = fixture();

        let warns = Rc::new(RefCell::new(Vec::new()));
        let warns2 = warns.clone();
        let expr = Expression::compile("unknownFn(42)").unwrap();
        let _b = BindingEffect::new(&reg, expr, state.clone(), None, None, move |_, ws| {
            warns2.borrow_mut().extend(ws);
        });
        assert!(!warns.borrow().is_empty());
    }

    // --- new_lazy tests ---

    #[test]
    fn new_lazy_evaluates_correctly() {
        let (_sched, reg, state, cache) = fixture();
        state.app_set("count", json!(7));

        let last = Rc::new(RefCell::new(RuntimeValue::null()));
        let last2 = last.clone();
        let _b = BindingEffect::new_lazy(
            &reg,
            cache,
            "$app.count + 1".into(),
            state.clone(),
            None,
            None,
            move |v, _| *last2.borrow_mut() = v,
        );
        assert_eq!(last.borrow().as_i64(), Some(8));
    }

    #[test]
    fn new_lazy_reacts_to_state_changes() {
        let (sched, reg, state, cache) = fixture();
        state.app_set("count", json!(0));

        let last = Rc::new(RefCell::new(RuntimeValue::null()));
        let last2 = last.clone();
        let _b = BindingEffect::new_lazy(
            &reg,
            cache,
            "$app.count * 3".into(),
            state.clone(),
            None,
            None,
            move |v, _| *last2.borrow_mut() = v,
        );
        assert_eq!(last.borrow().as_i64(), Some(0));

        state.app_set("count", json!(4));
        sched.flush();
        assert_eq!(last.borrow().as_i64(), Some(12));
    }

    #[test]
    fn new_lazy_dedupes_compilation_via_cache() {
        let (_sched, reg, state, cache) = fixture();
        state.app_set("flag", json!(true));

        // Three lazy bindings sharing the same source. The cache should
        // contain exactly one entry — three identical compiles collapse
        // into a single cache miss.
        let _b1 = BindingEffect::new_lazy(
            &reg,
            Rc::clone(&cache),
            "!$app.flag".into(),
            state.clone(),
            None,
            None,
            |_, _| {},
        );
        let _b2 = BindingEffect::new_lazy(
            &reg,
            Rc::clone(&cache),
            "!$app.flag".into(),
            state.clone(),
            None,
            None,
            |_, _| {},
        );
        let _b3 = BindingEffect::new_lazy(
            &reg,
            Rc::clone(&cache),
            "!$app.flag".into(),
            state.clone(),
            None,
            None,
            |_, _| {},
        );
        assert_eq!(cache.len(), 1, "expected one cached chunk");
        let (hits, misses) = cache.hit_rate();
        assert_eq!(misses, 1, "first compile is a miss");
        assert_eq!(hits, 2, "subsequent two compiles hit the cache");
    }

    #[test]
    fn new_lazy_compile_error_surfaces_as_diagnostic() {
        let (_sched, reg, state, cache) = fixture();
        let warns = Rc::new(RefCell::new(Vec::new()));
        let warns2 = warns.clone();
        let last = Rc::new(RefCell::new(RuntimeValue::null()));
        let last2 = last.clone();

        let _b = BindingEffect::new_lazy(
            &reg,
            cache,
            "this is not valid syntax @@@".into(),
            state,
            None,
            None,
            move |v, ws| {
                *last2.borrow_mut() = v;
                warns2.borrow_mut().extend(ws);
            },
        );
        // Compile error: apply still fires with null + diagnostic.
        assert!(last.borrow().is_null());
        assert!(
            !warns.borrow().is_empty(),
            "compile diagnostic must surface"
        );
    }

    // --- DeferredBindingQueue tests ---

    #[test]
    fn deferred_queue_starts_empty() {
        let q = DeferredBindingQueue::new();
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn deferred_queue_push_increments_len() {
        let mut q = DeferredBindingQueue::new();
        q.push("$app.a", None, None, |_, _| {});
        q.push("$app.b", Some("home".into()), Some("n".into()), |_, _| {});
        q.push("$app.c", None, None, |_, _| {});
        assert_eq!(q.len(), 3);
        assert!(!q.is_empty());
    }

    #[test]
    fn deferred_queue_drain_registers_effects_and_empties_self() {
        let (_sched, reg, state, cache) = fixture();
        state.app_set("a", json!(10));
        state.app_set("b", json!(20));

        let last_a = Rc::new(RefCell::new(RuntimeValue::null()));
        let last_b = Rc::new(RefCell::new(RuntimeValue::null()));

        let mut q = DeferredBindingQueue::new();
        let la = last_a.clone();
        q.push("$app.a", None, None, move |v, _| *la.borrow_mut() = v);
        let lb = last_b.clone();
        q.push("$app.b", None, None, move |v, _| *lb.borrow_mut() = v);
        assert_eq!(q.len(), 2);

        let effects = q.drain_into_effects(&reg, Rc::clone(&cache), state.clone());
        assert_eq!(effects.len(), 2, "drain returns one effect per entry");
        assert!(q.is_empty(), "queue must be empty after drain");

        // Initial eval ran during register; the apply callbacks fired.
        assert_eq!(last_a.borrow().as_i64(), Some(10));
        assert_eq!(last_b.borrow().as_i64(), Some(20));
    }

    #[test]
    fn deferred_queue_drain_uses_lazy_compilation() {
        let (_sched, reg, state, cache) = fixture();
        state.app_set("v", json!(1));

        let mut q = DeferredBindingQueue::new();
        for _ in 0..5 {
            q.push("$app.v + 1", None, None, |_, _| {});
        }
        let _effects = q.drain_into_effects(&reg, Rc::clone(&cache), state.clone());
        // Five identical sources share a single cache entry.
        assert_eq!(cache.len(), 1);
        let (hits, misses) = cache.hit_rate();
        assert_eq!(misses, 1);
        assert_eq!(hits, 4);
    }

    #[test]
    fn deferred_queue_compile_error_does_not_break_other_entries() {
        let (_sched, reg, state, cache) = fixture();
        state.app_set("good", json!(99));

        let bad_warns = Rc::new(RefCell::new(Vec::new()));
        let good_value = Rc::new(RefCell::new(RuntimeValue::null()));

        let mut q = DeferredBindingQueue::new();
        let bw = bad_warns.clone();
        q.push("@@@ broken", None, None, move |_, ws| {
            bw.borrow_mut().extend(ws);
        });
        let gv = good_value.clone();
        q.push("$app.good", None, None, move |v, _| {
            *gv.borrow_mut() = v;
        });

        let _effects = q.drain_into_effects(&reg, Rc::clone(&cache), state.clone());

        assert!(
            !bad_warns.borrow().is_empty(),
            "broken source must report diagnostic"
        );
        assert_eq!(good_value.borrow().as_i64(), Some(99));
    }

    #[test]
    fn deferred_queue_drained_effects_are_reactive() {
        let (sched, reg, state, cache) = fixture();
        state.app_set("n", json!(2));

        let last = Rc::new(RefCell::new(RuntimeValue::null()));
        let l = last.clone();

        let mut q = DeferredBindingQueue::new();
        q.push("$app.n * $app.n", None, None, move |v, _| {
            *l.borrow_mut() = v;
        });
        let _effects = q.drain_into_effects(&reg, Rc::clone(&cache), state.clone());
        assert_eq!(last.borrow().as_i64(), Some(4));

        state.app_set("n", json!(7));
        sched.flush();
        assert_eq!(last.borrow().as_i64(), Some(49));
    }
}
