//! EvalContext bridging the expression VM to [`crate::state::StateGraph`].

use super::cache::ExpressionCache;
use super::diag::Diagnostic;
use super::vm::EvalContext;
use crate::state::{Scope, StateGraph};
use crate::value::RuntimeValue;
use std::cell::RefCell;
use std::collections::BTreeMap;

pub type BuiltinFn = Box<dyn Fn(&dyn EvalContext, &[RuntimeValue]) -> Result<RuntimeValue, Diagnostic>>;

pub struct StateGraphContext<'a> {
    pub state: &'a StateGraph,
    pub page_id: Option<&'a str>,
    pub node_id: Option<&'a str>,
    pub locals: &'a BTreeMap<String, RuntimeValue>,
    pub builtins: &'a BTreeMap<String, BuiltinFn>,
    pub cache: Option<&'a ExpressionCache>,
    pub warnings: RefCell<Vec<Diagnostic>>,
}

impl<'a> StateGraphContext<'a> {
    pub fn new(
        state: &'a StateGraph,
        page_id: Option<&'a str>,
        node_id: Option<&'a str>,
        locals: &'a BTreeMap<String, RuntimeValue>,
        builtins: &'a BTreeMap<String, BuiltinFn>,
    ) -> Self {
        Self {
            state,
            page_id,
            node_id,
            locals,
            builtins,
            cache: None,
            warnings: RefCell::new(Vec::new()),
        }
    }

    pub fn with_cache(mut self, cache: &'a ExpressionCache) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn take_warnings(&self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.warnings.borrow_mut())
    }
}

impl<'a> EvalContext for StateGraphContext<'a> {
    fn lookup_scope(&self, path: &str) -> Option<RuntimeValue> {
        // Dotted path: fine-grained per-Signal subscription.
        if let Some(dot) = path.find('.') {
            let root = &path[..dot];
            let rest = &path[dot + 1..];
            let mut rest_parts = rest.split('.');
            let key = rest_parts.next()?;
            let tail: Vec<&str> = rest_parts.collect();

            let signal = match root {
                "$app" => self.state.app.borrow().get(key).cloned(),
                "$page" => self.page_id.and_then(|pid| {
                    self.state
                        .page
                        .borrow()
                        .get(pid)
                        .and_then(|m| m.get(key).cloned())
                }),
                "$self" => self.node_id.and_then(|nid| {
                    self.state
                        .self_
                        .borrow()
                        .get(nid)
                        .and_then(|m| m.get(key).cloned())
                }),
                "$route" => self.state.route.borrow().get(key).cloned(),
                "$storage" => self.state.storage.borrow().get(key).cloned(),
                "$vars" => self.state.vars.borrow().get(key).cloned(),
                "$state" => self
                    .node_id
                    .and_then(|nid| {
                        self.state
                            .self_
                            .borrow()
                            .get(nid)
                            .and_then(|m| m.get(key).cloned())
                    })
                    .or_else(|| {
                        self.page_id.and_then(|pid| {
                            self.state
                                .page
                                .borrow()
                                .get(pid)
                                .and_then(|m| m.get(key).cloned())
                        })
                    })
                    .or_else(|| self.state.app.borrow().get(key).cloned()),
                other => {
                    // Locals (e.g. `$item.title`): use `key`+`tail` against the
                    // local value directly.
                    let name = other.trim_start_matches('$');
                    let local = self
                        .locals
                        .get(name)
                        .cloned()
                        .or_else(|| self.locals.get(other).cloned())?;
                    let mut val = local.0;
                    val = walk_member(&val, key);
                    for seg in tail {
                        val = walk_member(&val, seg);
                    }
                    return Some(RuntimeValue(val));
                }
            };

            let signal = signal?;
            let mut val = signal.get().0;
            for seg in tail {
                val = walk_member(&val, seg);
            }
            return Some(RuntimeValue(val));
        }

        // Bare scope root — return a snapshot object.
        match path {
            "$app" => Some(RuntimeValue(scope_to_object(self.state, Scope::App, None))),
            "$page" => self
                .page_id
                .map(|pid| RuntimeValue(scope_to_object(self.state, Scope::Page, Some(pid)))),
            "$self" => self
                .node_id
                .map(|nid| RuntimeValue(scope_to_object(self.state, Scope::SelfNode, Some(nid)))),
            "$route" => Some(RuntimeValue(scope_to_object(self.state, Scope::Route, None))),
            "$storage" => Some(RuntimeValue(scope_to_object(
                self.state,
                Scope::Storage,
                None,
            ))),
            "$vars" => Some(RuntimeValue(scope_to_object(self.state, Scope::Vars, None))),
            "$state" => {
                if let Some(nid) = self.node_id {
                    return Some(RuntimeValue(scope_to_object(
                        self.state,
                        Scope::SelfNode,
                        Some(nid),
                    )));
                }
                if let Some(pid) = self.page_id {
                    return Some(RuntimeValue(scope_to_object(
                        self.state,
                        Scope::Page,
                        Some(pid),
                    )));
                }
                Some(RuntimeValue(scope_to_object(self.state, Scope::App, None)))
            }
            other => {
                let name = other.trim_start_matches('$');
                self.locals
                    .get(name)
                    .cloned()
                    .or_else(|| self.locals.get(other).cloned())
            }
        }
    }

    fn call_builtin(
        &self,
        name: &str,
        args: &[RuntimeValue],
    ) -> Option<Result<RuntimeValue, Diagnostic>> {
        self.builtins.get(name).map(|f| f(self, args))
    }

    fn warn(&self, d: Diagnostic) {
        self.warnings.borrow_mut().push(d);
    }

    fn cache(&self) -> Option<&ExpressionCache> {
        self.cache
    }
}

fn walk_member(v: &serde_json::Value, name: &str) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => m.get(name).cloned().unwrap_or(serde_json::Value::Null),
        _ => serde_json::Value::Null,
    }
}

/// Materialise an entire scope as a JSON object (used only when code pushes a
/// bare `$scope` without a dotted path, e.g. passing it to a builtin).
fn scope_to_object(state: &StateGraph, scope: Scope, id: Option<&str>) -> serde_json::Value {
    use serde_json::{Map, Value};
    let mut m = Map::new();
    match scope {
        Scope::App => {
            for (k, s) in state.app.borrow().iter() {
                m.insert(k.clone(), s.get().0);
            }
        }
        Scope::Page => {
            if let Some(pid) = id {
                if let Some(page_map) = state.page.borrow().get(pid) {
                    for (k, s) in page_map {
                        m.insert(k.clone(), s.get().0);
                    }
                }
            }
        }
        Scope::SelfNode => {
            if let Some(nid) = id {
                if let Some(node_map) = state.self_.borrow().get(nid) {
                    for (k, s) in node_map {
                        m.insert(k.clone(), s.get().0);
                    }
                }
            }
        }
        Scope::Route => {
            for (k, s) in state.route.borrow().iter() {
                m.insert(k.clone(), s.get().0);
            }
        }
        Scope::Storage => {
            for (k, s) in state.storage.borrow().iter() {
                m.insert(k.clone(), s.get().0);
            }
        }
        Scope::Vars => {
            for (k, s) in state.vars.borrow().iter() {
                m.insert(k.clone(), s.get().0);
            }
        }
    }
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::super::{compiler::compile, parser::parse, vm::run};
    use super::*;
    use crate::signal::{scheduler::Scheduler, Signal};
    use serde_json::json;
    use std::rc::Rc;

    fn setup() -> (
        Rc<Scheduler>,
        StateGraph,
        BTreeMap<String, RuntimeValue>,
        BTreeMap<String, BuiltinFn>,
    ) {
        let sched = Rc::new(Scheduler::new());
        let state = StateGraph::new(sched.clone());
        let locals = BTreeMap::new();
        let builtins: BTreeMap<String, BuiltinFn> = BTreeMap::new();
        (sched, state, locals, builtins)
    }

    #[test]
    fn read_app_scope() {
        let (_s, state, locals, builtins) = setup();
        state.app_set("count", json!(5));
        let ctx = StateGraphContext::new(&state, None, None, &locals, &builtins);
        let chunk = compile(&parse("$app.count").unwrap()).unwrap();
        let v = run(&chunk, &ctx).unwrap();
        assert_eq!(v.as_i64(), Some(5));
    }

    #[test]
    fn read_app_then_arithmetic() {
        let (_s, state, locals, builtins) = setup();
        state.app_set("base", json!(10));
        let ctx = StateGraphContext::new(&state, None, None, &locals, &builtins);
        let chunk = compile(&parse("$app.base + 5").unwrap()).unwrap();
        let v = run(&chunk, &ctx).unwrap();
        assert_eq!(v.as_i64(), Some(15));
    }

    #[test]
    fn contextual_state_uses_self() {
        let (sched, state, locals, builtins) = setup();
        state
            .self_
            .borrow_mut()
            .entry("n1".into())
            .or_default()
            .insert(
                "count".into(),
                Signal::new(RuntimeValue::from_i64(7), sched.clone()),
            );
        let ctx = StateGraphContext::new(&state, None, Some("n1"), &locals, &builtins);
        let chunk = compile(&parse("$state.count").unwrap()).unwrap();
        assert_eq!(run(&chunk, &ctx).unwrap().as_i64(), Some(7));
    }

    #[test]
    fn locals_for_item_and_index() {
        let (_s, state, mut locals, builtins) = setup();
        locals.insert("item".into(), RuntimeValue(json!({"title": "Hi"})));
        locals.insert("index".into(), RuntimeValue::from_i64(3));
        let ctx = StateGraphContext::new(&state, None, None, &locals, &builtins);
        let chunk = compile(&parse("$item.title").unwrap()).unwrap();
        assert_eq!(run(&chunk, &ctx).unwrap().as_str(), Some("Hi"));
    }

    #[test]
    fn unknown_scope_warns_and_returns_null() {
        let (_s, state, locals, builtins) = setup();
        let ctx = StateGraphContext::new(&state, None, None, &locals, &builtins);
        let chunk = compile(&parse("$mystery.foo").unwrap()).unwrap();
        let v = run(&chunk, &ctx).unwrap();
        assert!(v.is_null());
        assert!(!ctx.take_warnings().is_empty());
    }
}
