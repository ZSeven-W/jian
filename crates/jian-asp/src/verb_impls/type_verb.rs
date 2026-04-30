//! `type` verb implementation (Plan 18 Phase 3).
//!
//! Resolves the selector, reads the matched node's
//! `bindings.bind:value` (the canonical write surface for text
//! inputs — same path SetValue dispatches through), and writes
//! the new text into the bound `$app` state key. With
//! `clear: true` the existing value is replaced; otherwise the
//! new text is appended to the current value.
//!
//! The verb does not synthesise per-character key events. Real
//! text-input flow runs through `bind:value` projection (see
//! `scene::apply_bindings_to_node` for the read side); key
//! events are reserved for `events.onKey` handlers (typically
//! Enter / Escape on modals + form submits) and `tap`/`type`
//! shouldn't conflate the two surfaces.
//!
//! Returns:
//! - `not_found` when the selector matches zero nodes.
//! - `invalid` when the matched node has no `bindings.bind:value`
//!   (we have no writable target to fill).
//! - `invalid` when the binding doesn't follow the
//!   `$state.<flat-key>` shape derive::bind_target validates.
//! - `ok` carrying the matched id, the new value, and a
//!   `before/after` delta so the agent can confirm the write.

use jian_core::Runtime;

use crate::protocol::OutcomePayload;
use crate::selector::Selector;

/// Run the `type` verb. `text` is the literal string to write; when
/// `clear` is `Some(true)` the existing value is replaced, otherwise
/// the new text is appended to whatever is already in the bound
/// state key.
pub fn run_type(
    runtime: &mut Runtime,
    sel: &Selector,
    text: &str,
    clear: Option<bool>,
) -> OutcomePayload {
    let Some(doc) = runtime.document.as_ref() else {
        return OutcomePayload::error("type", "no document loaded");
    };
    let hits = match sel.resolve(&doc.tree) {
        Ok(h) => h,
        Err(e) => return OutcomePayload::invalid("type", &format!("{}", e)),
    };
    let Some(&first_key) = hits.first() else {
        return OutcomePayload::not_found("type", "selector matched zero nodes");
    };
    let schema = &doc.tree.nodes[first_key].schema;
    let id = jian_core::document::tree::node_schema_id(schema).to_owned();
    let path = match extract_state_path(schema) {
        Ok(p) => p,
        Err(reason) => {
            return OutcomePayload::invalid("type", reason).with_hint(
                "type targets a node with `bindings.bind:value: \"$state.<key>\"` \
                 — declare the binding on the input or use `set_state` directly"
                    .to_owned(),
            );
        }
    };

    let before = runtime
        .state
        .app_get(&path)
        .map(|v| v.0)
        .unwrap_or(serde_json::Value::Null);
    let prior_text = if clear.unwrap_or(false) {
        String::new()
    } else {
        json_to_text(&before)
    };
    let new_text = format!("{}{}", prior_text, text);
    let after = serde_json::Value::String(new_text.clone());
    runtime.state.app_set(&path, after.clone());

    let scoped_path = format!("$app.{}", path);
    OutcomePayload::ok(
        "type",
        Some(id.clone()),
        if clear.unwrap_or(false) {
            format!(
                "set `{}` to {} char(s) (cleared first)",
                id,
                text.chars().count()
            )
        } else {
            format!(
                "appended {} char(s) to `{}` (now {} char(s))",
                text.chars().count(),
                id,
                new_text.chars().count()
            )
        },
    )
    .with_delta(scoped_path, before, after, Some("asp.type".into()))
}

/// Extract the writable `$app` key from a node's `bindings.bind:value`.
/// Mirrors `action_surface::derive::bind_target` so the Type verb's
/// accept rules match the canonical SetValue dispatch — any binding
/// that derives a writable action (flat *or* dotted/indexed) is also
/// writable through ASP. Whitespace inside the path and an empty
/// remainder are rejected; everything else (including
/// `items[0].title` style paths) is forwarded verbatim to
/// `state.app_set`. The caller's app_set lookup is the authoritative
/// "does this path exist" check; we don't second-guess it here.
fn extract_state_path(node: &jian_ops_schema::node::PenNode) -> Result<String, &'static str> {
    let json = serde_json::to_value(node).map_err(|_| "node failed to serialise")?;
    let bindings = json.get("bindings").ok_or("node has no `bindings` block")?;
    let raw = bindings
        .get("bind:value")
        .ok_or("node has no `bindings.bind:value` to write into")?
        .as_str()
        .ok_or("`bindings.bind:value` is not a string")?
        .trim();
    let key = raw
        .strip_prefix("$state.")
        .ok_or("`bindings.bind:value` must start with `$state.`")?;
    if key.is_empty() {
        return Err("`bindings.bind:value` has empty key after `$state.`");
    }
    if key.chars().any(char::is_whitespace) {
        return Err("`bindings.bind:value` key must not contain whitespace");
    }
    Ok(key.to_owned())
}

/// Stringify the prior `$app` value for append-mode. Strings come
/// through verbatim; numbers and bools take their natural display
/// form; null / object / array fall back to `""` so a misuse doesn't
/// jam the runtime's display of a JSON literal into the input field.
fn json_to_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
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

    fn input_doc() -> &'static str {
        r##"{
          "formatVersion": "1.0", "version": "1.0.0", "id": "type-fx",
          "app": { "name": "type-fx", "version": "1", "id": "type-fx" },
          "state": { "draft": { "type": "string", "default": "" } },
          "children": [
            { "type": "frame", "id": "root", "width": 480, "height": 320, "x": 0, "y": 0,
              "children": [
                { "type": "text_input", "id": "field",
                  "x": 10, "y": 10, "width": 200, "height": 40,
                  "bindings": { "bind:value": "$state.draft" }
                }
              ]
            }
          ]
        }"##
    }

    #[test]
    fn type_appends_to_bound_state() {
        let mut rt = rt_with(input_doc());
        let sel = Selector {
            id: Some("field".into()),
            ..Default::default()
        };
        let out = run_type(&mut rt, &sel, "hi", None);
        assert!(out.ok, "expected ok, got {:?}", out);
        let v = rt.state.app_get("draft").map(|v| v.0).unwrap();
        assert_eq!(v.as_str(), Some("hi"));
        assert_eq!(out.deltas.len(), 1);
        assert_eq!(out.deltas[0].path, "$app.draft");

        // Second call appends.
        let out2 = run_type(&mut rt, &sel, " there", None);
        assert!(out2.ok);
        let v = rt.state.app_get("draft").map(|v| v.0).unwrap();
        assert_eq!(v.as_str(), Some("hi there"));
    }

    #[test]
    fn type_clear_replaces_existing_value() {
        let mut rt = rt_with(input_doc());
        rt.state.app_set("draft", serde_json::json!("old"));
        let sel = Selector {
            id: Some("field".into()),
            ..Default::default()
        };
        let out = run_type(&mut rt, &sel, "new", Some(true));
        assert!(out.ok);
        let v = rt.state.app_get("draft").map(|v| v.0).unwrap();
        assert_eq!(v.as_str(), Some("new"));
    }

    #[test]
    fn type_with_no_match_returns_not_found() {
        let mut rt = rt_with(input_doc());
        let sel = Selector {
            id: Some("nope".into()),
            ..Default::default()
        };
        let out = run_type(&mut rt, &sel, "x", None);
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("NotFound"));
    }

    #[test]
    fn type_on_node_without_bind_value_is_invalid() {
        let doc = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
              "children":[
                { "type":"rectangle","id":"plain","x":0,"y":0,"width":100,"height":40 }
              ]
            }
          ]
        }"##;
        let mut rt = rt_with(doc);
        let sel = Selector {
            id: Some("plain".into()),
            ..Default::default()
        };
        let out = run_type(&mut rt, &sel, "hello", None);
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("Invalid"));
        assert!(!out.hints.is_empty(), "should hint about bind:value");
    }

    #[test]
    fn type_rejects_non_state_binding() {
        let doc = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "state":{"draft":{"type":"string","default":""}},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
              "children":[
                { "type":"text_input","id":"f","x":0,"y":0,"width":100,"height":40,
                  "bindings": { "bind:value": "$app.draft" }
                }
              ]
            }
          ]
        }"##;
        let mut rt = rt_with(doc);
        let sel = Selector {
            id: Some("f".into()),
            ..Default::default()
        };
        let out = run_type(&mut rt, &sel, "x", None);
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("Invalid"));
    }

    #[test]
    fn type_accepts_dotted_state_path() {
        // bind:value can declare nested / indexed paths (matches
        // action_surface::derive::bind_target). app_set treats the
        // whole string as one map key, so the dotted path round-
        // trips through state.app_get with the same string.
        let doc = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
              "children":[
                { "type":"text_input","id":"f","x":0,"y":0,"width":100,"height":40,
                  "bindings": { "bind:value": "$state.items[0].title" }
                }
              ]
            }
          ]
        }"##;
        let mut rt = rt_with(doc);
        let sel = Selector {
            id: Some("f".into()),
            ..Default::default()
        };
        let out = run_type(&mut rt, &sel, "hello", Some(true));
        assert!(out.ok, "{:?}", out);
        let v = rt.state.app_get("items[0].title").map(|v| v.0).unwrap();
        assert_eq!(v.as_str(), Some("hello"));
    }

    #[test]
    fn type_rejects_whitespace_in_path() {
        let doc = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "state":{"draft":{"type":"string","default":""}},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
              "children":[
                { "type":"text_input","id":"f","x":0,"y":0,"width":100,"height":40,
                  "bindings": { "bind:value": "$state.draft x" }
                }
              ]
            }
          ]
        }"##;
        let mut rt = rt_with(doc);
        let sel = Selector {
            id: Some("f".into()),
            ..Default::default()
        };
        let out = run_type(&mut rt, &sel, "x", None);
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("Invalid"));
    }

    #[test]
    fn type_appends_numeric_prior_value_as_string() {
        let doc = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "state":{"draft":{"type":"int","default":42}},
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,
              "children":[
                { "type":"text_input","id":"f","x":0,"y":0,"width":100,"height":40,
                  "bindings": { "bind:value": "$state.draft" }
                }
              ]
            }
          ]
        }"##;
        let mut rt = rt_with(doc);
        let sel = Selector {
            id: Some("f".into()),
            ..Default::default()
        };
        let out = run_type(&mut rt, &sel, "+1", None);
        assert!(out.ok);
        let v = rt.state.app_get("draft").map(|v| v.0).unwrap();
        assert_eq!(v.as_str(), Some("42+1"));
    }
}
