//! Build a [`RuntimeDocument`] from a parsed schema [`PenDocument`].
//!
//! Also seeds the [`StateGraph`] from the document's `state` schema, and
//! loads design variables into the `$vars` scope.

use super::tree::NodeTree;
use super::RuntimeDocument;
use crate::error::CoreResult;
use crate::state::StateGraph;
use jian_ops_schema::document::PenDocument;
use jian_ops_schema::state::StateEntry;
use jian_ops_schema::variable::{VariableDefinition, VariableScalar, VariableValue};
use serde_json::{json, Value};

pub fn build(schema: PenDocument, state: &StateGraph) -> CoreResult<RuntimeDocument> {
    // Seed app-scope state from the document's state schema.
    if let Some(ref sch) = schema.state {
        for (name, entry) in sch {
            let default = resolve_default(entry);
            state.app_set(name, default);
        }
    }

    // Seed $vars from design variables (first themed value if themed).
    if let Some(ref vars) = schema.variables {
        for (name, def) in vars {
            state.vars_set(name, var_default(def));
        }
    }

    // Build the node tree. If pages exist, use the first page's children as roots;
    // otherwise use the document's `children`.
    let mut tree = NodeTree::new();
    let active_page = schema
        .pages
        .as_ref()
        .and_then(|ps| ps.first())
        .map(|p| p.id.clone());
    let root_nodes = match (&schema.pages, &schema.children) {
        (Some(pages), _) if !pages.is_empty() => pages[0].children.clone(),
        _ => schema.children.clone(),
    };
    for n in root_nodes {
        tree.insert_subtree(n, None);
    }

    Ok(RuntimeDocument {
        schema,
        tree,
        active_page,
    })
}

fn resolve_default(entry: &StateEntry) -> Value {
    entry.default.clone().unwrap_or(Value::Null)
}

fn var_default(def: &VariableDefinition) -> Value {
    match &def.value {
        VariableValue::Scalar(VariableScalar::Bool(b)) => json!(b),
        VariableValue::Scalar(VariableScalar::Num(n)) => json!(n),
        VariableValue::Scalar(VariableScalar::Str(s)) => json!(s),
        VariableValue::Themed(list) => match list.first().map(|t| &t.value) {
            Some(VariableScalar::Bool(b)) => json!(b),
            Some(VariableScalar::Num(n)) => json!(n),
            Some(VariableScalar::Str(s)) => json!(s),
            None => Value::Null,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::scheduler::Scheduler;
    use jian_ops_schema::load_str;
    use std::rc::Rc;

    fn load(src: &str) -> PenDocument {
        load_str(src).unwrap().value
    }

    #[test]
    fn build_minimal() {
        let s = load(r#"{"version":"0.8.0","children":[]}"#);
        let sched = Rc::new(Scheduler::new());
        let state = StateGraph::new(sched);
        let doc = build(s, &state).unwrap();
        assert_eq!(doc.node_count(), 0);
    }

    #[test]
    fn build_with_children() {
        let s = load(
            r#"{
              "version":"0.8.0",
              "children":[
                {"type":"frame","id":"root","children":[
                  {"type":"rectangle","id":"a"},
                  {"type":"rectangle","id":"b"}
                ]}
              ]
            }"#,
        );
        let sched = Rc::new(Scheduler::new());
        let state = StateGraph::new(sched);
        let doc = build(s, &state).unwrap();
        assert_eq!(doc.node_count(), 3);
        assert_eq!(doc.tree.roots.len(), 1);
    }

    #[test]
    fn build_with_pages_uses_first_page() {
        let s = load(
            r#"{
              "version":"0.8.0",
              "pages":[
                {"id":"home","name":"Home","children":[{"type":"rectangle","id":"h1"}]},
                {"id":"about","name":"About","children":[{"type":"rectangle","id":"a1"}]}
              ],
              "children":[]
            }"#,
        );
        let sched = Rc::new(Scheduler::new());
        let state = StateGraph::new(sched);
        let doc = build(s, &state).unwrap();
        assert_eq!(doc.node_count(), 1);
        assert!(doc.tree.get("h1").is_some());
        assert!(doc.tree.get("a1").is_none());
        assert_eq!(doc.active_page.as_deref(), Some("home"));
    }

    #[test]
    fn seeds_app_state_from_schema() {
        let s = load(
            r#"{
              "formatVersion":"1.0","version":"1.0.0",
              "state":{"count":{"type":"int","default":7}},
              "children":[]
            }"#,
        );
        let sched = Rc::new(Scheduler::new());
        let state = StateGraph::new(sched);
        let _doc = build(s, &state).unwrap();
        let v = state.app_get("count").unwrap();
        assert_eq!(v.as_i64(), Some(7));
    }
}
