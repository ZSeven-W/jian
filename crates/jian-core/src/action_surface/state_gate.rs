//! Built-in `bindings.visible` / `bindings.disabled` evaluator.
//!
//! Spec §4.2 #4 — the production state-gate walks the source node
//! and every ancestor, evaluating the two binding expressions
//! against the live `StateGraph`. If any ancestor's `visible`
//! resolves to `false` (or `disabled` to `true`), the action is
//! `state_gated`. Default for production hosts that don't ship a
//! custom `StateGate` implementation.
//!
//! Lives in `jian-core` because evaluation needs the document tree +
//! StateGraph + Expression compiler — none of which the
//! `jian-action-surface` crate carries directly. Hosts construct
//! `RuntimeStateGate { runtime: &Runtime }` and pass `&gate` to
//! `ActionSurface::execute_with_gate`.

use crate::document::{NodeKey, RuntimeDocument};
use crate::expression::{Expression, ExpressionCache};
use crate::state::StateGraph;
use jian_ops_schema::node::PenNode;
use serde_json::Value;
use std::rc::Rc;

/// Evaluator that walks `source_node_id` + ancestors and tests each
/// node's `bindings.visible` / `bindings.disabled` expression
/// against a live state graph. Returns `true` when every ancestor
/// is visible AND not disabled — false otherwise.
pub struct RuntimeStateGate<'a> {
    pub document: &'a RuntimeDocument,
    pub state: &'a StateGraph,
    pub expr_cache: Rc<ExpressionCache>,
}

impl<'a> RuntimeStateGate<'a> {
    pub fn new(
        document: &'a RuntimeDocument,
        state: &'a StateGraph,
        expr_cache: Rc<ExpressionCache>,
    ) -> Self {
        Self {
            document,
            state,
            expr_cache,
        }
    }

    /// Returns `true` when the action's source node and every
    /// ancestor passes the visible/disabled checks. False when any
    /// hop is hidden, disabled, or the node is missing entirely.
    pub fn allows(&self, source_node_id: &str) -> bool {
        let Some(start) = self.document.tree.get(source_node_id) else {
            // Missing nodes are state-gated — the action references a
            // tree slot that no longer exists (hot reload that
            // dropped the node, for instance).
            return false;
        };
        let mut current: Option<NodeKey> = Some(start);
        while let Some(key) = current {
            let Some(data) = self.document.tree.nodes.get(key) else {
                return false;
            };
            if !self.node_passes(&data.schema) {
                return false;
            }
            current = data.parent;
        }
        true
    }

    /// Evaluate `bindings.visible` and `bindings.disabled` on a
    /// single node. Result is `false` (state-gated) when either
    /// `visible == false` or `disabled == true`. Missing bindings
    /// default to visible + enabled.
    fn node_passes(&self, node: &PenNode) -> bool {
        let json = match serde_json::to_value(node) {
            Ok(v) => v,
            Err(_) => return true, // Can't introspect → don't block.
        };
        let bindings = json.get("bindings").and_then(|v| v.as_object());
        let Some(bindings) = bindings else {
            return true;
        };
        let node_id = json.get("id").and_then(|v| v.as_str()).unwrap_or("");

        if let Some(expr) = bindings.get("visible").and_then(|v| v.as_str()) {
            if !self.eval_bool(expr, node_id, true) {
                return false;
            }
        }
        if let Some(expr) = bindings.get("disabled").and_then(|v| v.as_str()) {
            if self.eval_bool(expr, node_id, false) {
                return false;
            }
        }
        true
    }

    /// Compile + evaluate a binding expression to bool. Returns
    /// `default` on parse failure or when the expression resolves to
    /// a non-boolean value — spec §4.2 #4 calls for a strict
    /// boolean, not a JS-style truthy/falsey coercion. A binding
    /// like `visible: "$state.count"` should be authored as
    /// `visible: "$state.count > 0"` so the type contract is
    /// explicit; this keeps the gate predictable.
    fn eval_bool(&self, expr_src: &str, node_id: &str, default: bool) -> bool {
        let compiled = match Expression::compile(expr_src) {
            Ok(e) => e,
            Err(_) => return default,
        };
        let (value, _warnings) = compiled.eval(self.state, None, Some(node_id));
        let _ = Value::Null; // keep `serde_json::Value` import alive
        value.as_bool().unwrap_or(default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::loader;
    use crate::signal::scheduler::Scheduler;
    use jian_ops_schema::document::PenDocument;

    fn build(json: &str) -> (RuntimeDocument, Rc<StateGraph>) {
        let schema: PenDocument = serde_json::from_str(json).unwrap();
        let scheduler = Rc::new(Scheduler::new());
        let state = Rc::new(StateGraph::new(scheduler));
        let doc = loader::build(schema, &state).unwrap();
        (doc, state)
    }

    #[test]
    fn missing_node_is_state_gated() {
        let (doc, state) = build(
            r#"{ "version":"0.8.0", "children":[
                { "type":"frame","id":"root" }
            ]}"#,
        );
        let cache = Rc::new(ExpressionCache::new());
        let gate = RuntimeStateGate::new(&doc, &state, cache);
        assert!(!gate.allows("nonexistent"));
    }

    #[test]
    fn no_bindings_means_allowed() {
        let (doc, state) = build(
            r#"{ "version":"0.8.0", "children":[
                { "type":"frame","id":"root","children":[
                    { "type":"frame","id":"child" }
                ]}
            ]}"#,
        );
        let cache = Rc::new(ExpressionCache::new());
        let gate = RuntimeStateGate::new(&doc, &state, cache);
        assert!(gate.allows("child"));
    }

    #[test]
    fn visible_false_blocks() {
        let (doc, state) = build(
            r#"{ "version":"0.8.0", "children":[
                { "type":"frame","id":"root", "bindings":{ "visible":"false" } }
            ]}"#,
        );
        let cache = Rc::new(ExpressionCache::new());
        let gate = RuntimeStateGate::new(&doc, &state, cache);
        assert!(!gate.allows("root"));
    }

    #[test]
    fn disabled_true_blocks() {
        let (doc, state) = build(
            r#"{ "version":"0.8.0", "children":[
                { "type":"frame","id":"root", "bindings":{ "disabled":"true" } }
            ]}"#,
        );
        let cache = Rc::new(ExpressionCache::new());
        let gate = RuntimeStateGate::new(&doc, &state, cache);
        assert!(!gate.allows("root"));
    }

    #[test]
    fn ancestor_hidden_blocks_descendant() {
        let (doc, state) = build(
            r#"{ "version":"0.8.0", "children":[
                { "type":"frame","id":"root", "bindings":{ "visible":"false" },
                  "children":[
                    { "type":"frame","id":"deep" }
                  ]}
            ]}"#,
        );
        let cache = Rc::new(ExpressionCache::new());
        let gate = RuntimeStateGate::new(&doc, &state, cache);
        assert!(!gate.allows("deep"));
    }

    #[test]
    fn non_boolean_expression_falls_back_to_default() {
        // `visible: "1"` must NOT evaluate as truthy — spec §4.2
        // requires an explicit bool. Number/string results land on
        // the default (visible:true → not blocked).
        let (doc, state) = build(
            r#"{ "version":"0.8.0", "children":[
                { "type":"frame","id":"root", "bindings":{ "visible":"1" } }
            ]}"#,
        );
        let cache = Rc::new(ExpressionCache::new());
        let gate = RuntimeStateGate::new(&doc, &state, cache);
        // `1` is not a bool → default visible:true → allowed.
        assert!(gate.allows("root"));
    }

    #[test]
    fn malformed_expression_falls_back_to_default() {
        let (doc, state) = build(
            r#"{ "version":"0.8.0", "children":[
                { "type":"frame","id":"root", "bindings":{ "visible":"((((" } }
            ]}"#,
        );
        let cache = Rc::new(ExpressionCache::new());
        let gate = RuntimeStateGate::new(&doc, &state, cache);
        // Malformed `visible` → default true → not blocked.
        assert!(gate.allows("root"));
    }

    /// Spec §4.2 #4 says the gate must evaluate `bindings.visible` /
    /// `bindings.disabled` against the live StateGraph. The Phase 1
    /// evaluator already runs the full Tier 1 expression suite —
    /// these tests pin that contract so a future evaluator swap or
    /// optimisation can't silently regress operator coverage.

    #[test]
    fn visible_supports_logical_and() {
        let (doc, state) = build(
            r#"{ "version":"0.8.0",
                 "state":{ "show":{ "type":"bool","default":true },
                           "perm":{ "type":"bool","default":true } },
                 "children":[
                   { "type":"frame","id":"root",
                     "bindings":{ "visible":"$state.show && $state.perm" } }
                 ]}"#,
        );
        let cache = Rc::new(ExpressionCache::new());
        let gate = RuntimeStateGate::new(&doc, &state, cache);
        assert!(gate.allows("root"), "both true → allowed");

        state.app_set("perm", serde_json::json!(false));
        assert!(!gate.allows("root"), "perm false → blocked");
    }

    #[test]
    fn disabled_supports_logical_or() {
        let (doc, state) = build(
            r#"{ "version":"0.8.0",
                 "state":{ "busy":{ "type":"bool","default":false },
                           "locked":{ "type":"bool","default":false } },
                 "children":[
                   { "type":"frame","id":"root",
                     "bindings":{ "disabled":"$state.busy || $state.locked" } }
                 ]}"#,
        );
        let cache = Rc::new(ExpressionCache::new());
        let gate = RuntimeStateGate::new(&doc, &state, cache);
        assert!(gate.allows("root"), "both false → allowed");

        state.app_set("locked", serde_json::json!(true));
        assert!(!gate.allows("root"), "locked true → blocked");
    }

    #[test]
    fn visible_supports_comparison_and_arithmetic() {
        let (doc, state) = build(
            r#"{ "version":"0.8.0",
                 "state":{ "count":{ "type":"int","default":3 } },
                 "children":[
                   { "type":"frame","id":"root",
                     "bindings":{ "visible":"$state.count > 0 && $state.count < 10" } }
                 ]}"#,
        );
        let cache = Rc::new(ExpressionCache::new());
        let gate = RuntimeStateGate::new(&doc, &state, cache);
        assert!(gate.allows("root"));

        state.app_set("count", serde_json::json!(0));
        assert!(!gate.allows("root"));

        state.app_set("count", serde_json::json!(99));
        assert!(!gate.allows("root"));
    }

    #[test]
    fn visible_supports_ternary() {
        let (doc, state) = build(
            r#"{ "version":"0.8.0",
                 "state":{ "mode":{ "type":"string","default":"public" } },
                 "children":[
                   { "type":"frame","id":"root",
                     "bindings":{ "visible":"$state.mode == \"public\" ? true : false" } }
                 ]}"#,
        );
        let cache = Rc::new(ExpressionCache::new());
        let gate = RuntimeStateGate::new(&doc, &state, cache);
        assert!(gate.allows("root"));

        state.app_set("mode", serde_json::json!("private"));
        assert!(!gate.allows("root"));
    }

    #[test]
    fn disabled_supports_negation() {
        let (doc, state) = build(
            r#"{ "version":"0.8.0",
                 "state":{ "ready":{ "type":"bool","default":false } },
                 "children":[
                   { "type":"frame","id":"submit",
                     "bindings":{ "disabled":"!$state.ready" } }
                 ]}"#,
        );
        let cache = Rc::new(ExpressionCache::new());
        let gate = RuntimeStateGate::new(&doc, &state, cache);
        assert!(
            !gate.allows("submit"),
            "ready=false → !ready=true → disabled → blocked"
        );

        state.app_set("ready", serde_json::json!(true));
        assert!(
            gate.allows("submit"),
            "ready=true → !ready=false → enabled → allowed"
        );
    }

    #[test]
    fn disabled_template_literal_falls_back_to_default() {
        // Backtick templates produce *strings*, never bools, so the
        // strict-bool §4.2 rule rejects them — gate falls back to
        // the default. We pin `disabled` (default `false` → allowed)
        // because it makes the fallback observable: a real bool
        // evaluator would flip with the value of `x`, but the
        // string-typed projection always reads the default.
        let (doc, state) = build(
            r#"{ "version":"0.8.0",
                 "state":{ "x":{ "type":"int","default":10 } },
                 "children":[
                   { "type":"frame","id":"root",
                     "bindings":{ "disabled":"`${$state.x > 5}`" } }
                 ]}"#,
        );
        let cache = Rc::new(ExpressionCache::new());
        let gate = RuntimeStateGate::new(&doc, &state, cache);
        // x = 10 > 5 → template renders "true" → not a bool → default
        // disabled:false → allowed. (A real bool eval would block.)
        assert!(gate.allows("root"));

        // Flip x so a bool-typed projection would *unblock* — fallback
        // semantics keep us at default-allowed regardless.
        state.app_set("x", serde_json::json!(0));
        assert!(
            gate.allows("root"),
            "fallback ignores template content; allowed in both states"
        );

        // Authors who want bool-valued shortcuts compose the comparison
        // directly, no backticks:
        //   "disabled": "$state.x > 5"
    }
}
