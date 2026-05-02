//! Prod-mode operation verb guard (Plan 18 ASP prod mode / C5).
//!
//! Spec §9 Task C5 narrows the prod operation surface so an agent
//! that connects to a shipping app can't reach into the document
//! tree by smuggling structural selectors through the operation
//! verbs:
//!
//! - In **prod mode**, `tap` / `type` / `scroll` / `swipe` accept
//!   *only* `Selector { id: Some("<list_actions id>"), .. }`. Every
//!   other selector field (role / text / near / child_of / …) is
//!   refused with `OutcomePayload::invalid`.
//! - The id must resolve to an entry in the `list_actions`
//!   projection (`AvailabilityStatic::Available` AND not in an
//!   `aiHidden` subtree). This is the same filter `run_list_actions`
//!   applies, so the prod-op surface is exactly the listed surface.
//! - The action's `events` (derived from `source_kind`) must be
//!   compatible with the verb. `tap` requires a tap-bearing kind
//!   (Tap / DoubleTap / LongPress / Confirm / Dismiss). `type`
//!   requires `SetValue`. `scroll` requires Scroll / LoadMore.
//!   `swipe` requires SwipeLeft/Right/Up/Down. Mismatch → `invalid`
//!   with the action's actual events.
//!
//! On success the guard returns a fresh `Selector { id:
//! Some(action.source_node_id) }` that the existing dev-mode op
//! handlers (`run_tap` / `run_type` / `run_scroll` / `run_swipe`)
//! resolve against the runtime's node tree without modification —
//! prod mode reuses the same dispatch path, just with the selector
//! narrowed to a single `id` lookup.

use jian_core::action_surface::{
    derive_actions, ActionDefinition, AvailabilityStatic, SourceKind, BUILD_SALT,
};
use jian_core::Runtime;

use crate::protocol::{OutcomePayload, Verb};
use crate::selector::Selector;
use crate::verb_impls::list_actions::collect_ai_hidden_subtree;

/// Which event the prod op verb expects on the targeted action.
/// Mirrors the `source_kind_to_events` mapping in
/// `verb_impls::list_actions` so the agent's UI ("the action lists
/// `events: ["tap"]`") reads the same way the guard checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProdEvent {
    Tap,
    Set,
    Scroll,
    Swipe,
}

impl ProdEvent {
    pub fn name(self) -> &'static str {
        match self {
            ProdEvent::Tap => "tap",
            ProdEvent::Set => "set",
            ProdEvent::Scroll => "scroll",
            ProdEvent::Swipe => "swipe",
        }
    }

    /// Does the action's `source_kind` produce the event this verb
    /// expects?
    pub fn matches_kind(self, kind: SourceKind) -> bool {
        matches!(
            (self, kind),
            (
                ProdEvent::Tap,
                SourceKind::Tap
                    | SourceKind::DoubleTap
                    | SourceKind::LongPress
                    | SourceKind::Confirm
                    | SourceKind::Dismiss,
            ) | (ProdEvent::Set, SourceKind::SetValue)
                | (ProdEvent::Scroll, SourceKind::Scroll | SourceKind::LoadMore)
                | (
                    ProdEvent::Swipe,
                    SourceKind::SwipeLeft
                        | SourceKind::SwipeRight
                        | SourceKind::SwipeUp
                        | SourceKind::SwipeDown,
                )
        )
    }
}

/// `source_kind` → wire event name. Identical to the mapping in
/// `list_actions::source_kind_to_events`; duplicated here so the
/// guard can mention an action's actual event in error messages
/// without crossing a private-fn boundary. (Kept in sync by being
/// near it semantically — the same spec §12 table drives both.)
fn event_name_for_kind(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Tap
        | SourceKind::DoubleTap
        | SourceKind::LongPress
        | SourceKind::Confirm
        | SourceKind::Dismiss => "tap",
        SourceKind::SetValue => "set",
        SourceKind::Scroll | SourceKind::LoadMore => "scroll",
        SourceKind::SwipeLeft
        | SourceKind::SwipeRight
        | SourceKind::SwipeUp
        | SourceKind::SwipeDown => "swipe",
        SourceKind::Submit => "submit",
        SourceKind::OpenRoute => "open",
    }
}

/// Validate one prod-mode op verb selector and rewrite it to a
/// source-node-id selector the dev op handlers can resolve.
///
/// Returns:
/// - `Ok(rewritten)` — caller passes this to `run_tap` / etc.
/// - `Err(payload)` — caller writes this back to the agent and
///   continues the session loop.
///
/// `OutcomePayload` carries the full audit-friendly response shape
/// (~240 B); boxing the Err just to satisfy `result_large_err`
/// would obscure the call sites for no actual savings — the error
/// path runs once per refused request and the payload immediately
/// flows into a `serde_json` line write.
#[allow(clippy::result_large_err)]
pub fn validate_prod_op_target(
    verb: &'static str,
    expected: ProdEvent,
    sel: &Selector,
    runtime: &Runtime,
) -> Result<Selector, OutcomePayload> {
    if !is_action_id_only(sel) {
        return Err(OutcomePayload::invalid(
            verb,
            "prod ASP requires a selector of the form \
             `{\"id\": \"<list_actions id>\"}` — arbitrary structural \
             selectors (role / text / near / child_of / …) are \
             refused (Plan 18 spec §9 C5)",
        ));
    }
    // Safe unwrap because `is_action_id_only` requires `id.is_some()`.
    let id = sel.id.as_deref().unwrap();
    if id.is_empty() {
        return Err(OutcomePayload::invalid(
            verb,
            "prod ASP requires a non-empty `id` from `list_actions`",
        ));
    }
    let Some(doc) = runtime.document.as_ref() else {
        return Err(OutcomePayload::error(verb, "no document loaded"));
    };
    let actions = derive_actions(&doc.schema, &BUILD_SALT);
    let hidden = collect_ai_hidden_subtree(&doc.schema);

    // Find the matching action — must satisfy the same projection
    // filter `run_list_actions` applies (Available + not aiHidden).
    let action: Option<&ActionDefinition> = actions.iter().find(|a| {
        matches!(a.status, AvailabilityStatic::Available)
            && !hidden.contains(&a.source_node_id)
            && a.full_name() == id
    });
    let Some(action) = action else {
        return Err(OutcomePayload::not_found(
            verb,
            &format!(
                "action id `{}` is not in the current `list_actions` projection",
                id
            ),
        ));
    };

    if !expected.matches_kind(action.source_kind) {
        return Err(OutcomePayload::invalid(
            verb,
            &format!(
                "verb `{}` requires event `{}`, but action `{}` declares event `{}` \
                 (source_kind {:?}) — call `list_actions` and pick an action whose \
                 `events` array contains `{}`",
                verb,
                expected.name(),
                id,
                event_name_for_kind(action.source_kind),
                action.source_kind,
                expected.name(),
            ),
        ));
    }

    Ok(Selector {
        id: Some(action.source_node_id.clone()),
        ..Default::default()
    })
}

/// Pre-dispatch hook: if `verb` is an op verb (tap/type/scroll/swipe),
/// validate + rewrite its selector. Other verbs pass through unchanged.
///
/// Returns:
/// - `Ok(Some(rewritten))` — replace the verb in dispatch with this.
/// - `Ok(None)` — `verb` is not an op verb; dispatch as-is.
/// - `Err(payload)` — validation failed; write `payload` to the agent.
#[allow(clippy::result_large_err)]
pub fn rewrite_op_verb_for_prod(
    verb: &Verb,
    runtime: &Runtime,
) -> Result<Option<Verb>, OutcomePayload> {
    match verb {
        Verb::Tap { selector } => {
            let s = validate_prod_op_target("tap", ProdEvent::Tap, selector, runtime)?;
            Ok(Some(Verb::Tap { selector: s }))
        }
        Verb::Type {
            selector,
            text,
            clear,
        } => {
            let s = validate_prod_op_target("type", ProdEvent::Set, selector, runtime)?;
            Ok(Some(Verb::Type {
                selector: s,
                text: text.clone(),
                clear: *clear,
            }))
        }
        Verb::Scroll {
            selector,
            direction,
            distance,
        } => {
            let s = validate_prod_op_target("scroll", ProdEvent::Scroll, selector, runtime)?;
            Ok(Some(Verb::Scroll {
                selector: s,
                direction: *direction,
                distance: *distance,
            }))
        }
        Verb::Swipe {
            selector,
            direction,
            distance,
        } => {
            let s = validate_prod_op_target("swipe", ProdEvent::Swipe, selector, runtime)?;
            Ok(Some(Verb::Swipe {
                selector: s,
                direction: *direction,
                distance: *distance,
            }))
        }
        _ => Ok(None),
    }
}

/// True when `s` has *only* the `id` field set. The strict shape is
/// what spec §9 C5 calls out — a prod agent has to derive its target
/// from `list_actions` ids, never from a structural query, so any
/// extra field is a red flag.
fn is_action_id_only(s: &Selector) -> bool {
    s.id.is_some()
        && s.alias.is_none()
        && s.role.is_none()
        && s.text.is_none()
        && s.text_contains.is_none()
        && s.visible.is_none()
        && s.focused.is_none()
        && s.enabled.is_none()
        && s.near.is_none()
        && s.child_of.is_none()
        && s.parent_of.is_none()
        && s.all_of.is_none()
        && s.any_of.is_none()
        && s.not.is_none()
        && s.first.is_none()
        && s.index.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_ops_schema::document::PenDocument;

    fn rt_with(doc_json: &str) -> Runtime {
        let schema: PenDocument = jian_ops_schema::load_str(doc_json).unwrap().value;
        let mut rt = Runtime::new_from_document(schema).unwrap();
        rt.build_layout((480.0, 320.0)).unwrap();
        rt.rebuild_spatial();
        rt
    }

    fn tap_doc() -> &'static str {
        r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "state":{"count":{"type":"int","default":0}},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
              "children":[
                { "type":"rectangle","id":"save-btn","x":100,"y":100,"width":100,"height":40,
                  "events":{"onTap":[{"set":{"$app.count":"$app.count + 1"}}]},
                  "semantics":{"role":"button","label":"Save"}
                }
              ]
            }
          ]
        }"##
    }

    fn first_action_id(rt: &Runtime) -> String {
        let doc = rt.document.as_ref().unwrap();
        let actions = derive_actions(&doc.schema, &BUILD_SALT);
        actions
            .iter()
            .find(|a| matches!(a.status, AvailabilityStatic::Available))
            .map(|a| a.full_name())
            .expect("at least one action")
    }

    #[test]
    fn rejects_structural_selector_in_prod() {
        let rt = rt_with(tap_doc());
        let sel = Selector {
            role: Some("button".into()),
            text: Some("Save".into()),
            ..Default::default()
        };
        let err = validate_prod_op_target("tap", ProdEvent::Tap, &sel, &rt).unwrap_err();
        assert_eq!(err.error.as_deref(), Some("Invalid"));
        assert!(
            err.narrative.contains("requires a selector of the form"),
            "narrative: {}",
            err.narrative
        );
    }

    #[test]
    fn rejects_id_combined_with_structural_field() {
        let rt = rt_with(tap_doc());
        let sel = Selector {
            id: Some("anything".into()),
            text: Some("Save".into()),
            ..Default::default()
        };
        let err = validate_prod_op_target("tap", ProdEvent::Tap, &sel, &rt).unwrap_err();
        assert_eq!(err.error.as_deref(), Some("Invalid"));
    }

    #[test]
    fn rejects_unknown_action_id() {
        let rt = rt_with(tap_doc());
        let sel = Selector {
            id: Some("does.not_exist".into()),
            ..Default::default()
        };
        let err = validate_prod_op_target("tap", ProdEvent::Tap, &sel, &rt).unwrap_err();
        assert_eq!(err.error.as_deref(), Some("NotFound"));
    }

    #[test]
    fn rejects_event_mismatch() {
        let rt = rt_with(tap_doc());
        let id = first_action_id(&rt); // a tap action
        let sel = Selector {
            id: Some(id.clone()),
            ..Default::default()
        };
        // Use it with `type` (expects "set") → should reject.
        let err = validate_prod_op_target("type", ProdEvent::Set, &sel, &rt).unwrap_err();
        assert_eq!(err.error.as_deref(), Some("Invalid"));
        assert!(
            err.narrative.contains("requires event `set`"),
            "narrative: {}",
            err.narrative
        );
    }

    #[test]
    fn accepts_action_id_and_rewrites_to_source_node() {
        let rt = rt_with(tap_doc());
        let id = first_action_id(&rt);
        let sel = Selector {
            id: Some(id.clone()),
            ..Default::default()
        };
        let rewritten = validate_prod_op_target("tap", ProdEvent::Tap, &sel, &rt).unwrap();
        // Selector now points at the source node id, not the
        // action id. The dev op handlers (run_tap / etc.) treat
        // this as a normal `Selector { id: ... }` lookup.
        assert_eq!(rewritten.id.as_deref(), Some("save-btn"));
        // No structural fields leak in.
        assert!(rewritten.role.is_none());
        assert!(rewritten.text.is_none());
    }

    #[test]
    fn rewrite_op_verb_passes_through_non_op_verbs() {
        let rt = rt_with(tap_doc());
        let v = Verb::Exit;
        assert!(matches!(rewrite_op_verb_for_prod(&v, &rt), Ok(None)));
        let v = Verb::ListActions {
            cursor: None,
            limit: None,
        };
        assert!(matches!(rewrite_op_verb_for_prod(&v, &rt), Ok(None)));
    }

    #[test]
    fn rewrite_op_verb_validates_tap_selector() {
        let rt = rt_with(tap_doc());
        let id = first_action_id(&rt);
        let v = Verb::Tap {
            selector: Selector {
                id: Some(id.clone()),
                ..Default::default()
            },
        };
        let rewritten = rewrite_op_verb_for_prod(&v, &rt).unwrap().unwrap();
        match rewritten {
            Verb::Tap { selector } => {
                assert_eq!(selector.id.as_deref(), Some("save-btn"));
            }
            other => panic!("expected Tap, got {:?}", other),
        }
    }

    #[test]
    fn rewrite_op_verb_validates_type_selector_compatibility() {
        let rt = rt_with(tap_doc());
        let id = first_action_id(&rt); // tap action — incompatible with type
        let v = Verb::Type {
            selector: Selector {
                id: Some(id),
                ..Default::default()
            },
            text: "hello".into(),
            clear: None,
        };
        let err = rewrite_op_verb_for_prod(&v, &rt).unwrap_err();
        assert_eq!(err.error.as_deref(), Some("Invalid"));
    }
}
