//! `list_available_actions` — spec §5.1, §5.2.

use jian_core::action_surface::{ActionDefinition, AvailabilityStatic, ParamSpec, ParamTy};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse {
    pub actions: Vec<ListedAction>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListedAction {
    pub name: String,
    pub description: String,
    pub params_schema: Value,
    pub returns_schema: Value,
}

#[derive(Debug, Clone, Default)]
pub struct ListOptions {
    /// Author/debug switch from spec §4.1 — when true, ConfirmGated
    /// actions also appear with `status: "confirm_gated"` so the
    /// AI Actions Panel can preview them.
    pub include_confirm_gated: bool,
}

/// Filter the derived action list by static availability + render the
/// MCP-shaped response.
pub fn list_actions(actions: &[ActionDefinition], opts: ListOptions) -> ListResponse {
    let mut out = Vec::with_capacity(actions.len());
    for a in actions {
        match a.status {
            AvailabilityStatic::Available => {}
            AvailabilityStatic::ConfirmGated if opts.include_confirm_gated => {}
            _ => continue,
        }
        out.push(ListedAction {
            name: a.name.full(),
            description: a.description.clone(),
            params_schema: build_params_schema(&a.params),
            returns_schema: returns_ok_schema(),
        });
    }
    let total = out.len();
    ListResponse { actions: out, total }
}

fn build_params_schema(params: &[ParamSpec]) -> Value {
    if params.is_empty() {
        return json!({ "type": "object", "properties": {} });
    }
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();
    for p in params {
        props.insert(p.name.clone(), json_for_ty(p.ty));
        required.push(p.name.clone());
    }
    json!({
        "type": "object",
        "required": required,
        "properties": Value::Object(props),
    })
}

fn json_for_ty(ty: ParamTy) -> Value {
    match ty {
        ParamTy::Int => json!({ "type": "integer" }),
        ParamTy::Float => json!({ "type": "number" }),
        ParamTy::Number => json!({ "type": "number" }),
        ParamTy::String => json!({ "type": "string" }),
        ParamTy::Bool => json!({ "type": "boolean" }),
        ParamTy::Date => json!({ "type": "string", "format": "date-time" }),
        ParamTy::Unknown => json!({}),
    }
}

fn returns_ok_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "ok": { "type": "boolean" } }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_core::action_surface::{ActionName, Scope, SourceKind};

    fn def(slug: &str, status: AvailabilityStatic, params: Vec<ParamSpec>) -> ActionDefinition {
        ActionDefinition {
            name: ActionName {
                scope: Scope::page("home"),
                slug: slug.to_owned(),
            },
            source_node_id: "n".into(),
            source_kind: SourceKind::Tap,
            description: format!("Tap {}", slug),
            status,
            aliases: vec![],
            params,
        }
    }

    #[test]
    fn only_available_listed_by_default() {
        let acts = vec![
            def("a", AvailabilityStatic::Available, vec![]),
            def("b", AvailabilityStatic::StaticHidden, vec![]),
            def("c", AvailabilityStatic::ConfirmGated, vec![]),
        ];
        let r = list_actions(&acts, ListOptions::default());
        assert_eq!(r.total, 1);
        assert_eq!(r.actions[0].name, "home.a");
    }

    #[test]
    fn include_confirm_gated_surfaces_them() {
        let acts = vec![
            def("a", AvailabilityStatic::Available, vec![]),
            def("c", AvailabilityStatic::ConfirmGated, vec![]),
        ];
        let r = list_actions(
            &acts,
            ListOptions {
                include_confirm_gated: true,
            },
        );
        assert_eq!(r.total, 2);
    }

    #[test]
    fn params_schema_reflects_param_types() {
        let acts = vec![def(
            "set_email",
            AvailabilityStatic::Available,
            vec![ParamSpec {
                name: "value".into(),
                ty: ParamTy::String,
            }],
        )];
        let r = list_actions(&acts, ListOptions::default());
        assert_eq!(r.actions[0].params_schema["type"], "object");
        assert_eq!(r.actions[0].params_schema["required"][0], "value");
        assert_eq!(
            r.actions[0].params_schema["properties"]["value"]["type"],
            "string"
        );
    }
}
