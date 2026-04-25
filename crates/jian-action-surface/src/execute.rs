//! `execute_action` — spec §4.2 dynamic gating + parameter validation.
//!
//! Runs the gating sequence top-to-bottom:
//! 1. Look up the action by name (or aiAlias).
//! 2. StaticHidden / ConfirmGated short-circuits.
//! 3. (StateGated would consult `bindings.visible` / `disabled` against
//!    the live StateGraph — Phase 1 leaves that hook for the runtime
//!    handler to surface; the surface itself does the static checks.)
//! 4. Parameter validation against the derived `ParamSpec` list.
//! 5. Caller hands the result back to the runtime which dispatches
//!    the synthesised event.
//!
//! Phase 1 returns a structured `ExecuteOutcome` indicating which
//! action to dispatch + how. Wiring into `Runtime` lives in `lib.rs`.

use crate::error::{ExecuteError, ValidationReason};
use jian_core::action_surface::{ActionDefinition, AvailabilityStatic, ParamSpec, ParamTy};
use serde_json::{Map, Value};

/// What the surface decided to do with the request — either dispatch
/// the named action or short-circuit with an error.
#[derive(Debug)]
pub(crate) enum Decision<'a> {
    Dispatch {
        action: &'a ActionDefinition,
        params: Map<String, Value>,
    },
    Reject(ExecuteError),
}

/// Resolve `name` against the derived list (matching aliases too) and
/// run the static gate + parameter validation steps. Side-effect-free.
pub(crate) fn decide<'a>(
    actions: &'a [ActionDefinition],
    name: &str,
    raw_params: Option<&Value>,
) -> Decision<'a> {
    let Some(action) = lookup(actions, name) else {
        return Decision::Reject(ExecuteError::unknown_action());
    };
    match action.status {
        AvailabilityStatic::StaticHidden => {
            return Decision::Reject(ExecuteError::static_hidden());
        }
        AvailabilityStatic::ConfirmGated => {
            // Spec §4.2 #3: ConfirmGated rejects regardless of whether
            // the action was listed (include_confirm_gated only opens
            // visibility, not callability).
            return Decision::Reject(ExecuteError::confirm_gated());
        }
        AvailabilityStatic::Available => {}
    }
    let params = match validate_params(&action.params, raw_params) {
        Ok(map) => map,
        Err(e) => return Decision::Reject(e),
    };
    Decision::Dispatch { action, params }
}

fn lookup<'a>(actions: &'a [ActionDefinition], name: &str) -> Option<&'a ActionDefinition> {
    if let Some(a) = actions.iter().find(|a| a.name.full() == name) {
        return Some(a);
    }
    actions
        .iter()
        .find(|a| a.aliases.iter().any(|al| al.full() == name))
}

/// Walk the declared `params` list against the caller-supplied object.
/// Any missing required key, type mismatch, or unexpected extra → a
/// fixed-reason `ExecuteError::ValidationFailed`.
fn validate_params(
    declared: &[ParamSpec],
    raw: Option<&Value>,
) -> Result<Map<String, Value>, ExecuteError> {
    let supplied = match raw {
        None => Map::new(),
        Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err(ExecuteError::schema_violation()),
    };

    if declared.is_empty() {
        if !supplied.is_empty() {
            return Err(ExecuteError::schema_violation());
        }
        return Ok(supplied);
    }

    let mut out = Map::new();
    for spec in declared {
        let Some(v) = supplied.get(&spec.name) else {
            return Err(ExecuteError::missing_required());
        };
        if !value_matches(v, spec.ty) {
            return Err(ExecuteError::ValidationFailed {
                reason: ValidationReason::TypeMismatch,
            });
        }
        out.insert(spec.name.clone(), v.clone());
    }
    // Reject unexpected extras — spec doesn't carry an "additional
    // properties" channel back to the agent.
    for k in supplied.keys() {
        if !declared.iter().any(|p| &p.name == k) {
            return Err(ExecuteError::schema_violation());
        }
    }
    Ok(out)
}

fn value_matches(v: &Value, ty: ParamTy) -> bool {
    match ty {
        ParamTy::Int => v.is_i64() || v.is_u64(),
        ParamTy::Float | ParamTy::Number => v.is_number(),
        ParamTy::String | ParamTy::Date => v.is_string(),
        ParamTy::Bool => v.is_boolean(),
        ParamTy::Unknown => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_core::action_surface::{ActionName, Scope, SourceKind};
    use serde_json::json;

    fn make(slug: &str, status: AvailabilityStatic, params: Vec<ParamSpec>) -> ActionDefinition {
        ActionDefinition {
            name: ActionName {
                scope: Scope::page("home"),
                slug: slug.to_owned(),
            },
            source_node_id: "n".into(),
            source_kind: SourceKind::Tap,
            description: "".into(),
            status,
            aliases: vec![],
            params,
            has_explicit_name: false,
        }
    }

    #[test]
    fn unknown_action_rejected() {
        let acts = vec![make("a", AvailabilityStatic::Available, vec![])];
        let d = decide(&acts, "home.missing", None);
        assert!(matches!(
            d,
            Decision::Reject(ExecuteError::NotAvailable {
                reason: super::super::error::NotAvailableReason::UnknownAction
            })
        ));
    }

    #[test]
    fn alias_resolves_to_action() {
        let mut a = make("rename_target", AvailabilityStatic::Available, vec![]);
        a.aliases.push(ActionName {
            scope: Scope::page("home"),
            slug: "old_name".into(),
        });
        let acts = vec![a];
        let d = decide(&acts, "home.old_name", None);
        match d {
            Decision::Dispatch { action, .. } => {
                assert_eq!(action.name.slug, "rename_target")
            }
            _ => panic!("expected dispatch on alias"),
        }
    }

    #[test]
    fn static_hidden_rejected() {
        let acts = vec![make("h", AvailabilityStatic::StaticHidden, vec![])];
        let d = decide(&acts, "home.h", None);
        assert!(matches!(
            d,
            Decision::Reject(ExecuteError::NotAvailable { .. })
        ));
    }

    #[test]
    fn confirm_gated_rejected_even_when_listed() {
        let acts = vec![make("c", AvailabilityStatic::ConfirmGated, vec![])];
        let d = decide(&acts, "home.c", None);
        assert!(matches!(
            d,
            Decision::Reject(ExecuteError::NotAvailable {
                reason: super::super::error::NotAvailableReason::ConfirmGated
            })
        ));
    }

    #[test]
    fn missing_required_param() {
        let acts = vec![make(
            "set_v",
            AvailabilityStatic::Available,
            vec![ParamSpec {
                name: "value".into(),
                ty: ParamTy::String,
            }],
        )];
        let d = decide(&acts, "home.set_v", Some(&json!({})));
        assert!(matches!(
            d,
            Decision::Reject(ExecuteError::ValidationFailed {
                reason: ValidationReason::MissingRequired
            })
        ));
    }

    #[test]
    fn type_mismatch_param() {
        let acts = vec![make(
            "set_v",
            AvailabilityStatic::Available,
            vec![ParamSpec {
                name: "value".into(),
                ty: ParamTy::Int,
            }],
        )];
        let d = decide(&acts, "home.set_v", Some(&json!({ "value": "not-int" })));
        assert!(matches!(
            d,
            Decision::Reject(ExecuteError::ValidationFailed {
                reason: ValidationReason::TypeMismatch
            })
        ));
    }

    #[test]
    fn unexpected_extra_param() {
        let acts = vec![make("a", AvailabilityStatic::Available, vec![])];
        let d = decide(&acts, "home.a", Some(&json!({ "junk": 1 })));
        assert!(matches!(
            d,
            Decision::Reject(ExecuteError::ValidationFailed {
                reason: ValidationReason::SchemaViolation
            })
        ));
    }

    #[test]
    fn happy_path_dispatch() {
        let acts = vec![make(
            "set_v",
            AvailabilityStatic::Available,
            vec![ParamSpec {
                name: "value".into(),
                ty: ParamTy::Int,
            }],
        )];
        let d = decide(&acts, "home.set_v", Some(&json!({ "value": 7 })));
        match d {
            Decision::Dispatch { action, params } => {
                assert_eq!(action.name.slug, "set_v");
                assert_eq!(params["value"], 7);
            }
            _ => panic!("expected dispatch"),
        }
    }
}
