//! `list_available_actions` — spec §5.1, §5.2.

use jian_core::action_surface::{ActionDefinition, AvailabilityStatic, ParamSpec, ParamTy};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ListResponse {
    pub actions: Vec<ListedAction>,
    pub total: usize,
    /// Current page id (matches the §5.2 wire field). `None` when the
    /// runtime didn't pass a page hint and `page_scope == All`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ListedAction {
    pub name: String,
    pub description: String,
    pub params_schema: Value,
    pub returns_schema: Value,
    /// Static availability bucket. Default available actions omit it
    /// (cleaner wire form); ConfirmGated actions surfaced via
    /// `include_confirm_gated` carry `"confirm_gated"` per §4.1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Whether `list_available_actions` should filter to the active page
/// or return everything in the document. Spec §5.1 default is
/// `Current`; `All` is opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageScope {
    #[default]
    Current,
    All,
}

#[derive(Debug, Clone, Default)]
pub struct ListOptions {
    /// Spec §5.1 `page_scope`. Defaults to current page; the request
    /// passes `Current` even when `current_page` is `None` (in which
    /// case the filter is a no-op and the result matches `All`).
    pub page_scope: PageScope,
    /// The active page id. Hosts should set this when they know it
    /// so `Current` filtering actually trims the list. Used as the
    /// returned `page` field.
    pub current_page: Option<String>,
    /// Author/debug switch from spec §4.1 — when true, ConfirmGated
    /// actions also appear with `status: "confirm_gated"` so the
    /// AI Actions Panel can preview them.
    pub include_confirm_gated: bool,
}

/// Filter the derived action list by static availability + scope and
/// render the MCP-shaped response.
pub fn list_actions(actions: &[ActionDefinition], opts: ListOptions) -> ListResponse {
    let mut out = Vec::with_capacity(actions.len());
    for a in actions {
        // §4.1 default — only Available unless include_confirm_gated.
        let status: Option<String> = match a.status {
            AvailabilityStatic::Available => None,
            AvailabilityStatic::ConfirmGated if opts.include_confirm_gated => {
                Some("confirm_gated".to_owned())
            }
            _ => continue,
        };
        if opts.page_scope == PageScope::Current {
            if let Some(ref p) = opts.current_page {
                if !is_in_scope(a.name.scope.as_str(), p) {
                    continue;
                }
            }
        }
        out.push(ListedAction {
            name: a.name.full(),
            description: a.description.clone(),
            params_schema: build_params_schema(&a.params),
            returns_schema: returns_ok_schema(),
            status,
        });
    }
    let total = out.len();
    ListResponse {
        actions: out,
        total,
        page: opts.current_page,
    }
}

/// `Current` page filter — keep an action whose scope is either
/// `global`, the current page literal, or a `modal.*` whose dialog
/// belongs to that page. We can't perfectly resolve "modal belongs to
/// page X" without the document tree, so for Phase 1 modals are
/// always kept (they're contextual to the user's current view).
fn is_in_scope(scope: &str, current_page: &str) -> bool {
    if scope == current_page || scope == "global" || scope.starts_with("modal.") {
        return true;
    }
    false
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
            has_explicit_name: false,
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
    fn include_confirm_gated_surfaces_them_with_status() {
        let acts = vec![
            def("a", AvailabilityStatic::Available, vec![]),
            def("c", AvailabilityStatic::ConfirmGated, vec![]),
        ];
        let r = list_actions(
            &acts,
            ListOptions {
                include_confirm_gated: true,
                ..ListOptions::default()
            },
        );
        assert_eq!(r.total, 2);
        // Available action has no status field on the wire.
        let avail = r.actions.iter().find(|a| a.name == "home.a").unwrap();
        assert!(avail.status.is_none());
        // ConfirmGated action carries the literal "confirm_gated".
        let gated = r.actions.iter().find(|a| a.name == "home.c").unwrap();
        assert_eq!(gated.status.as_deref(), Some("confirm_gated"));
    }

    #[test]
    fn current_page_scope_filters_other_pages() {
        let mut acts = vec![def("a", AvailabilityStatic::Available, vec![])];
        // An action on a different page.
        let mut other = def("b", AvailabilityStatic::Available, vec![]);
        other.name = jian_core::action_surface::ActionName {
            scope: jian_core::action_surface::Scope::page("settings"),
            slug: "b".into(),
        };
        acts.push(other);
        let r = list_actions(
            &acts,
            ListOptions {
                page_scope: PageScope::Current,
                current_page: Some("home".into()),
                ..ListOptions::default()
            },
        );
        let names: Vec<_> = r.actions.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"home.a"));
        assert!(!names.contains(&"settings.b"));
        assert_eq!(r.page.as_deref(), Some("home"));
    }

    #[test]
    fn all_scope_keeps_everything() {
        let mut acts = vec![def("a", AvailabilityStatic::Available, vec![])];
        let mut other = def("b", AvailabilityStatic::Available, vec![]);
        other.name = jian_core::action_surface::ActionName {
            scope: jian_core::action_surface::Scope::page("settings"),
            slug: "b".into(),
        };
        acts.push(other);
        let r = list_actions(
            &acts,
            ListOptions {
                page_scope: PageScope::All,
                ..ListOptions::default()
            },
        );
        assert_eq!(r.total, 2);
    }

    #[test]
    fn modal_scope_kept_under_current_page_filter() {
        let mut acts = vec![def("a", AvailabilityStatic::Available, vec![])];
        let mut modal = def("close", AvailabilityStatic::Available, vec![]);
        modal.name = jian_core::action_surface::ActionName {
            scope: jian_core::action_surface::Scope::modal("dlg-1"),
            slug: "close".into(),
        };
        acts.push(modal);
        let r = list_actions(
            &acts,
            ListOptions {
                page_scope: PageScope::Current,
                current_page: Some("home".into()),
                ..ListOptions::default()
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
