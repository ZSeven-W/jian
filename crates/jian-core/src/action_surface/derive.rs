//! `derive_actions(doc, build_salt)` — pure derivation function.
//!
//! Given a `PenDocument`, walk every page + child node and apply spec
//! §3.2's rules to produce an ordered `Vec<ActionDefinition>`. Result
//! is **bitwise stable** for the same `(doc, build_salt)` pair —
//! covered by `derive_is_deterministic` in the test suite.
//!
//! Phase 1 implements the user-intent rules:
//! - `events.onTap` → `<slug>`
//! - `events.onDoubleTap` → `double_tap_<slug>`
//! - `events.onLongPress` → `long_press_<slug>`
//! - `events.onSubmit` → `submit_<slug>`
//! - `bindings["bind:value"]` → `set_<slug>(value)` (input-style nodes)
//! - `route.push` → `open_<slug>(p)` (route params from `RouteSpec.params`)
//!
//! Swipe / scroll / key actions are deferred until the gesture arena
//! exposes pan-direction + key + wheel events through the schema.

use super::naming::{compute_slug, has_ai_name, short_hash};
use super::types::{ActionDefinition, ActionName, AvailabilityStatic, Scope, SourceKind};
use jian_ops_schema::document::PenDocument;
use serde_json::Value;

/// Walk `doc` and emit the deterministic action list. `build_salt` is
/// the compile-time disambiguator (typically derived from the package
/// version + git rev) — same input ⇒ same output, byte-for-byte.
pub fn derive_actions(doc: &PenDocument, build_salt: &[u8; 16]) -> Vec<ActionDefinition> {
    let mut out = Vec::new();
    let doc_json = match serde_json::to_value(doc) {
        Ok(v) => v,
        Err(_) => return out,
    };

    if let Some(pages) = doc_json.get("pages").and_then(|v| v.as_array()) {
        for page in pages {
            let page_id = page
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("page")
                .to_owned();
            let scope_resolver = ScopeResolver::page(&page_id);
            if let Some(children) = page.get("children").and_then(|v| v.as_array()) {
                for child in children {
                    walk(child, &doc_json, &scope_resolver, build_salt, &mut out);
                }
            }
        }
    }

    if let Some(children) = doc_json.get("children").and_then(|v| v.as_array()) {
        // Document-level children fall back to global scope.
        let scope_resolver = ScopeResolver::global();
        for child in children {
            walk(child, &doc_json, &scope_resolver, build_salt, &mut out);
        }
    }

    out
}

fn walk(
    node: &Value,
    doc_json: &Value,
    parent_scope: &ScopeResolver,
    build_salt: &[u8; 16],
    out: &mut Vec<ActionDefinition>,
) {
    let scope = parent_scope.refine(node);
    emit_for_node(node, doc_json, &scope, build_salt, out);
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        let next = ScopeResolver::from_scope(scope);
        for child in children {
            walk(child, doc_json, &next, build_salt, out);
        }
    }
}

fn emit_for_node(
    node: &Value,
    doc_json: &Value,
    scope: &Scope,
    build_salt: &[u8; 16],
    out: &mut Vec<ActionDefinition>,
) {
    let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let slug = compute_slug(node);
    let suffixed = if has_ai_name(node) {
        slug.clone()
    } else {
        format!("{}_{}", slug, short_hash(id, build_salt))
    };
    let description = node
        .get("semantics")
        .and_then(|s| s.get("aiDescription"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let aliases = node
        .get("semantics")
        .and_then(|s| s.get("aiAliases"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let events = node.get("events").and_then(|v| v.as_object());

    // --- onTap → <slug>
    if let Some(handler) = events.and_then(|e| e.get("onTap")) {
        out.push(make_action(
            scope,
            &suffixed,
            id,
            SourceKind::Tap,
            description.clone(),
            &aliases,
            node,
            Some(handler),
        ));
    }
    if let Some(handler) = events.and_then(|e| e.get("onDoubleTap")) {
        let slug_v = format!("double_tap_{}", suffixed);
        out.push(make_action(
            scope,
            &slug_v,
            id,
            SourceKind::DoubleTap,
            description.clone(),
            &aliases,
            node,
            Some(handler),
        ));
    }
    if let Some(handler) = events.and_then(|e| e.get("onLongPress")) {
        let slug_v = format!("long_press_{}", suffixed);
        out.push(make_action(
            scope,
            &slug_v,
            id,
            SourceKind::LongPress,
            description.clone(),
            &aliases,
            node,
            Some(handler),
        ));
    }
    if let Some(handler) = events.and_then(|e| e.get("onSubmit")) {
        let slug_v = format!("submit_{}", suffixed);
        out.push(make_action(
            scope,
            &slug_v,
            id,
            SourceKind::Submit,
            description.clone(),
            &aliases,
            node,
            Some(handler),
        ));
    }

    // --- bindings["bind:value"] → set_<slug>
    let bind_value = node
        .get("bindings")
        .and_then(|b| b.get("bind:value"))
        .and_then(|v| v.as_str());
    if bind_value.is_some() {
        let slug_v = format!("set_{}", suffixed);
        out.push(make_action(
            scope,
            &slug_v,
            id,
            SourceKind::SetValue,
            description.clone(),
            &aliases,
            node,
            None,
        ));
    }

    // --- route.push → open_<slug>
    let route_push = node
        .get("route")
        .and_then(|r| r.get("push"))
        .and_then(|v| v.as_str());
    if route_push.is_some() {
        let _ = doc_json; // RouteSpec.params consultation lands in Phase 2.
        let slug_v = format!("open_{}", suffixed);
        out.push(make_action(
            scope,
            &slug_v,
            id,
            SourceKind::OpenRoute,
            description.clone(),
            &aliases,
            node,
            None,
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn make_action(
    scope: &Scope,
    slug: &str,
    source_node_id: &str,
    source_kind: SourceKind,
    description: Option<String>,
    aliases: &[String],
    node: &Value,
    handler: Option<&Value>,
) -> ActionDefinition {
    let name = ActionName {
        scope: scope.clone(),
        slug: slug.to_owned(),
    };
    let alias_names = aliases
        .iter()
        .map(|a| ActionName {
            scope: scope.clone(),
            slug: a.clone(),
        })
        .collect();
    let status = super::availability::classify(node, handler);
    let auto_desc = description
        .or_else(|| auto_describe(source_kind, slug))
        .unwrap_or_default();
    ActionDefinition {
        name,
        source_node_id: source_node_id.to_owned(),
        source_kind,
        description: auto_desc,
        status,
        aliases: alias_names,
    }
}

fn auto_describe(kind: SourceKind, slug: &str) -> Option<String> {
    Some(match kind {
        SourceKind::Tap => format!("Tap {}", slug),
        SourceKind::DoubleTap => format!("Double-tap {}", slug),
        SourceKind::LongPress => format!("Long-press {}", slug),
        SourceKind::Submit => format!("Submit {}", slug),
        SourceKind::SetValue => format!("Set the value of {}", slug),
        SourceKind::OpenRoute => format!("Open {}", slug),
        SourceKind::SwipeLeft => format!("Swipe left on {}", slug),
        SourceKind::SwipeRight => format!("Swipe right on {}", slug),
        SourceKind::SwipeUp => format!("Swipe up on {}", slug),
        SourceKind::SwipeDown => format!("Swipe down on {}", slug),
        SourceKind::Scroll => format!("Scroll {}", slug),
        SourceKind::Confirm => format!("Confirm {}", slug),
        SourceKind::Dismiss => format!("Dismiss {}", slug),
    })
}

/// Tracks the current scope and refines as we descend. A child sitting
/// inside a `dialog` ancestor switches scope to `modal.<dialog_id>`;
/// otherwise the parent scope carries through.
struct ScopeResolver {
    current: Scope,
}

impl ScopeResolver {
    fn page(page_id: &str) -> Self {
        Self {
            current: Scope::page(page_id),
        }
    }
    fn global() -> Self {
        Self {
            current: Scope::global(),
        }
    }
    fn from_scope(scope: Scope) -> Self {
        Self { current: scope }
    }

    fn refine(&self, node: &Value) -> Scope {
        let role = node
            .get("semantics")
            .and_then(|s| s.get("role"))
            .and_then(|v| v.as_str());
        if role == Some("dialog") {
            let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("dialog");
            return Scope::modal(id);
        }
        self.current.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_ops_schema::document::PenDocument;

    fn doc_from(json: &str) -> PenDocument {
        serde_json::from_str(json).expect("schema must parse")
    }

    #[test]
    fn empty_document_yields_no_actions() {
        let doc = doc_from(r#"{ "version":"0.8.0", "children":[] }"#);
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert!(acts.is_empty());
    }

    #[test]
    fn on_tap_emits_basic_action() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"sign-in","content":"Sign In",
                  "events":{ "onTap": [ { "set": { "$state.user.signed_in": "true" } } ] }
                }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0x42u8; 16]);
        assert_eq!(acts.len(), 1);
        let a = &acts[0];
        assert_eq!(a.source_kind, SourceKind::Tap);
        assert_eq!(a.name.scope.as_str(), "home");
        assert!(a.name.slug.starts_with("sign_in_"));
        assert_eq!(a.name.slug.len(), "sign_in_".len() + 4);
    }

    #[test]
    fn ai_name_drops_hash_suffix() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"x",
                  "semantics":{ "aiName":"sign_in" },
                  "events":{ "onTap": [ { "set": { "$state.x": "1" } } ] }
                }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0x42u8; 16]);
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].name.slug, "sign_in");
        assert_eq!(acts[0].name.full(), "home.sign_in");
    }

    #[test]
    fn ai_hidden_marks_static_hidden() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"x",
                  "semantics":{ "aiName":"hidden_btn", "aiHidden":true },
                  "events":{ "onTap": [ { "set": { "$state.x": "1" } } ] }
                }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts[0].status, AvailabilityStatic::StaticHidden);
    }

    #[test]
    fn destructive_handler_is_confirm_gated() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"delete-btn",
                  "semantics":{ "label":"Delete account" },
                  "events":{ "onTap": [ { "storage_wipe": null } ] }
                }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts[0].status, AvailabilityStatic::ConfirmGated);
    }

    #[test]
    fn dialog_ancestor_picks_modal_scope() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"dlg","semantics":{ "role":"dialog" },
                  "children":[
                    { "type":"frame","id":"close",
                      "semantics":{ "aiName":"close" },
                      "events":{ "onTap": [ { "pop": null } ] }
                    }
                  ]
                }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts[0].name.scope.as_str(), "modal.dlg");
        assert_eq!(acts[0].name.full(), "modal.dlg.close");
    }

    #[test]
    fn route_emits_open_action() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"list","name":"List","children":[
                { "type":"frame","id":"card",
                  "semantics":{ "aiName":"open_detail" },
                  "route":{ "push": "/detail/:id" }
                }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts[0].source_kind, SourceKind::OpenRoute);
        assert_eq!(acts[0].name.slug, "open_open_detail");
    }

    #[test]
    fn bind_value_emits_set_action() {
        // Schema MVP doesn't define a `text_input` variant yet —
        // a `frame` carrying `bindings: bind:value` is the closest
        // valid form that exercises this rule.
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"signup","name":"Sign up","children":[
                { "type":"frame","id":"email-input",
                  "semantics":{ "aiName":"email" },
                  "bindings": { "bind:value": "$state.email" }
                }
              ]}],
              "children":[]
            }"#,
        );
        let acts = derive_actions(&doc, &[0u8; 16]);
        assert_eq!(acts[0].source_kind, SourceKind::SetValue);
        assert_eq!(acts[0].name.slug, "set_email");
    }

    #[test]
    fn derive_is_deterministic() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"a", "events":{ "onTap": [ { "pop": null } ] } },
                { "type":"frame","id":"b", "events":{ "onTap": [ { "pop": null } ] } }
              ]}],
              "children":[]
            }"#,
        );
        let salt = [0xab; 16];
        let a = derive_actions(&doc, &salt);
        let b = derive_actions(&doc, &salt);
        assert_eq!(a, b);
    }

    #[test]
    fn salt_changes_hash_but_preserves_ai_name() {
        let doc = doc_from(
            r#"{
              "version":"0.8.0",
              "pages":[{ "id":"home","name":"Home","children":[
                { "type":"frame","id":"auto",
                  "events":{ "onTap": [ { "pop": null } ] } },
                { "type":"frame","id":"named",
                  "semantics":{ "aiName":"keep_me" },
                  "events":{ "onTap": [ { "pop": null } ] } }
              ]}],
              "children":[]
            }"#,
        );
        let s1 = [1u8; 16];
        let s2 = [2u8; 16];
        let a = derive_actions(&doc, &s1);
        let b = derive_actions(&doc, &s2);
        assert_ne!(a[0].name.slug, b[0].name.slug);
        assert_eq!(a[1].name.slug, b[1].name.slug);
        assert_eq!(a[1].name.slug, "keep_me");
    }
}
