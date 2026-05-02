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

    // Codex C5 round 1 MEDIUM: defend against duplicate node ids in
    // the document. `derive_actions` walks the schema and may emit an
    // action whose `source_node_id` collides with another node's id;
    // `NodeTree::insert_subtree` is the wrong place to enforce
    // uniqueness (overwrites silently). We resolve the rewritten
    // selector here and require *exactly one* hit, refusing
    // ambiguous documents at the prod-op gate rather than dispatching
    // to whichever node the resolver happened to keep.
    let rewritten = Selector {
        id: Some(action.source_node_id.clone()),
        ..Default::default()
    };
    let hits = rewritten.resolve(&doc.tree).map_err(|e| {
        OutcomePayload::error(
            verb,
            &format!(
                "internal: rewritten id selector failed to resolve: {}",
                e
            ),
        )
    })?;
    if hits.is_empty() {
        return Err(OutcomePayload::not_found(
            verb,
            &format!(
                "action `{}` declares source node `{}` but the runtime tree \
                 has no such node (likely a stale document or hot-reload race)",
                id, action.source_node_id
            ),
        ));
    }
    if hits.len() > 1 {
        return Err(OutcomePayload::invalid(
            verb,
            &format!(
                "document has {} nodes with id `{}` — refusing prod op dispatch \
                 because the source node is ambiguous (Plan 18 C5 / codex review). \
                 Fix the document to give every node a unique id.",
                hits.len(),
                action.source_node_id
            ),
        ));
    }

    Ok(rewritten)
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

/// Extract the action id from an op-verb's pre-rewrite selector.
/// Returns `None` for non-op verbs or for an empty / missing id —
/// callers fall back to a no-op sanitization in those cases.
pub fn extract_action_id(verb: &Verb) -> Option<String> {
    let sel = match verb {
        Verb::Tap { selector } => selector,
        Verb::Type { selector, .. } => selector,
        Verb::Scroll { selector, .. } => selector,
        Verb::Swipe { selector, .. } => selector,
        _ => return None,
    };
    sel.id
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned())
}

/// Sanitize an op verb's response so prod-mode wire bodies don't
/// carry `.op` tree structure (Plan 18 spec §10 bullet 2 / codex C6
/// round 1).
///
/// The dev-mode op handlers (`run_tap` / `run_type` / `run_scroll` /
/// `run_swipe`) populate `target` with the schema node id and bake
/// the matched node's layout-rect coordinates into `narrative` —
/// useful diagnostic context for a debugging agent, but a leak in
/// prod where the agent only has business with the action id.
///
/// This function:
/// - Replaces `target` with the action id the agent passed in.
/// - Replaces `narrative` with a generic outcome string keyed off
///   the action id + the response shape (`ok` / `error`).
/// - Preserves `deltas` (state-graph mutations are *business*
///   state, not document structure — the agent needs them).
/// - Preserves `hints` (current ops emit only generic hints; a
///   future structural hint would need a separate stripping
///   mechanism, tracked by codex C6 round 2).
/// - Preserves `detail` and `error`.
pub fn sanitize_prod_op_payload(
    mut p: OutcomePayload,
    action_id: &str,
) -> OutcomePayload {
    // Always set `target` to the agent-visible id, even when the
    // dev handler returned `None` — the agent then has a stable
    // anchor for the response without us guessing whether to
    // populate it.
    p.target = Some(action_id.to_owned());
    p.narrative = if p.ok {
        format!("action `{}` dispatched", action_id)
    } else if let Some(err) = p.error.as_deref() {
        format!("action `{}` rejected: {}", action_id, err)
    } else {
        format!("action `{}` failed", action_id)
    };
    p
}

/// True when `s` has *only* the `id` field set. The strict shape is
/// what spec §9 C5 calls out — a prod agent has to derive its target
/// from `list_actions` ids, never from a structural query, so any
/// extra field is a red flag.
///
/// **Future-proof field coverage** (codex C5 round 1, MEDIUM): the
/// body uses exhaustive struct destructuring without `..`, so adding
/// a 17th field to `Selector` becomes a *compile error* here rather
/// than silent acceptance. If the new field is structural, refuse
/// it; if it's id-shaped (an alternative resolver hint that
/// preserves single-target semantics), opt it in here explicitly.
fn is_action_id_only(s: &Selector) -> bool {
    let Selector {
        id,
        alias,
        role,
        text,
        text_contains,
        visible,
        focused,
        enabled,
        near,
        child_of,
        parent_of,
        all_of,
        any_of,
        not,
        first,
        index,
    } = s;
    id.is_some()
        && alias.is_none()
        && role.is_none()
        && text.is_none()
        && text_contains.is_none()
        && visible.is_none()
        && focused.is_none()
        && enabled.is_none()
        && near.is_none()
        && child_of.is_none()
        && parent_of.is_none()
        && all_of.is_none()
        && any_of.is_none()
        && not.is_none()
        && first.is_none()
        && index.is_none()
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

    // Codex C5 round 1 LOW — additional coverage:

    fn input_doc() -> &'static str {
        // A SetValue action via `bind:value` on a text-input rectangle.
        r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "state":{"email":{"type":"string","default":""}},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
              "children":[
                { "type":"rectangle","id":"email-input","x":50,"y":50,"width":300,"height":40,
                  "bindings":{"bind:value":"$state.email"},
                  "semantics":{"role":"text"}
                }
              ]
            }
          ]
        }"##
    }

    #[test]
    fn rewrite_type_succeeds_on_set_value_action() {
        // Pin codex's "test gap": end-to-end Verb::Type rewrite
        // against a SetValue action — the action_surface emits a
        // `set_*` action whose source_kind is SetValue, the guard
        // accepts it under ProdEvent::Set, and the rewrite returns
        // the source_node_id.
        let rt = rt_with(input_doc());
        let id = first_action_id(&rt);
        let v = Verb::Type {
            selector: Selector {
                id: Some(id),
                ..Default::default()
            },
            text: "user@example.com".into(),
            clear: Some(true),
        };
        let rewritten = rewrite_op_verb_for_prod(&v, &rt).unwrap().unwrap();
        match rewritten {
            Verb::Type {
                selector,
                text,
                clear,
            } => {
                assert_eq!(selector.id.as_deref(), Some("email-input"));
                assert_eq!(text, "user@example.com");
                assert_eq!(clear, Some(true));
            }
            other => panic!("expected Type, got {:?}", other),
        }
    }

    #[test]
    fn aihidden_action_returns_not_found_through_guard() {
        // An `aiHidden: true` ancestor must hide its children's
        // actions from the guard's projection (mirrors C2 logic).
        let doc = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "state":{"count":{"type":"int","default":0}},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
              "semantics":{"aiHidden":true},
              "children":[
                { "type":"rectangle","id":"hidden-btn","x":0,"y":0,"width":100,"height":40,
                  "events":{"onTap":[{"set":{"$app.count":"$app.count + 1"}}]}
                }
              ]
            }
          ]
        }"##;
        let rt = rt_with(doc);
        let doc_ref = rt.document.as_ref().unwrap();
        // Find the action by walking derive_actions output directly
        // (not list_actions, which would already have filtered it).
        let acts = derive_actions(&doc_ref.schema, &BUILD_SALT);
        let hidden_action = acts
            .iter()
            .find(|a| a.source_node_id == "hidden-btn")
            .expect("hidden-btn action exists pre-projection");
        let id = hidden_action.full_name();
        let sel = Selector {
            id: Some(id),
            ..Default::default()
        };
        let err = validate_prod_op_target("tap", ProdEvent::Tap, &sel, &rt).unwrap_err();
        assert_eq!(
            err.error.as_deref(),
            Some("NotFound"),
            "aiHidden ancestor must hide the action: {:?}",
            err
        );
    }

    #[test]
    fn rewrite_scroll_verb_succeeds_on_scroll_action() {
        let doc = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "state":{"page":{"type":"int","default":0}},
          "children":[
            { "type":"frame","id":"root","width":480,"height":600,"x":0,"y":0,
              "children":[
                { "type":"frame","id":"feed","x":0,"y":0,"width":480,"height":600,
                  "events":{"onScroll":[{"set":{"$app.page":"$app.page + 1"}}]}
                }
              ]
            }
          ]
        }"##;
        let rt = rt_with(doc);
        let id = first_action_id(&rt);
        let v = Verb::Scroll {
            selector: Selector {
                id: Some(id),
                ..Default::default()
            },
            direction: crate::protocol::ScrollDir::Down,
            distance: Some(120.0),
        };
        let rewritten = rewrite_op_verb_for_prod(&v, &rt).unwrap().unwrap();
        match rewritten {
            Verb::Scroll {
                selector,
                direction,
                distance,
            } => {
                assert_eq!(selector.id.as_deref(), Some("feed"));
                assert!(matches!(direction, crate::protocol::ScrollDir::Down));
                assert_eq!(distance, Some(120.0));
            }
            other => panic!("expected Scroll, got {:?}", other),
        }
    }

    #[test]
    fn rewrite_swipe_verb_succeeds_on_swipe_action() {
        // Swipe actions derive from `onPanStart` + `onPanEnd`
        // (spec §3.2): both required, action_surface emits four
        // directional swipe_*_<slug> actions sharing the onPanEnd
        // handler. Pick the swipe_left_* one for this test.
        let doc = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "state":{"step":{"type":"int","default":0}},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
              "children":[
                { "type":"rectangle","id":"deck","x":0,"y":0,"width":300,"height":200,
                  "events":{
                    "onPanStart":[{"set":{"$app.step":"$app.step"}}],
                    "onPanEnd":[{"set":{"$app.step":"$app.step + 1"}}]
                  }
                }
              ]
            }
          ]
        }"##;
        let rt = rt_with(doc);
        // Pick a SwipeLeft action specifically (first_action_id
        // could return any of the four directions).
        let doc_ref = rt.document.as_ref().unwrap();
        let acts = derive_actions(&doc_ref.schema, &BUILD_SALT);
        let id = acts
            .iter()
            .find(|a| matches!(a.source_kind, SourceKind::SwipeLeft))
            .map(|a| a.full_name())
            .expect("at least one SwipeLeft action");
        let v = Verb::Swipe {
            selector: Selector {
                id: Some(id),
                ..Default::default()
            },
            direction: crate::protocol::ScrollDir::Left,
            distance: None,
        };
        let rewritten = rewrite_op_verb_for_prod(&v, &rt).unwrap().unwrap();
        match rewritten {
            Verb::Swipe {
                selector,
                direction,
                ..
            } => {
                assert_eq!(selector.id.as_deref(), Some("deck"));
                assert!(matches!(direction, crate::protocol::ScrollDir::Left));
            }
            other => panic!("expected Swipe, got {:?}", other),
        }
    }
}
