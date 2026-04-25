//! Static-availability classifier (spec §4).
//!
//! Decides whether a derived action is `Available`, `StaticHidden`
//! (author opted out via `semantics.aiHidden`), or `ConfirmGated`
//! (handler contains a destructive signal — `confirm:` /
//! `fetch DELETE|POST` / `storage_clear` / `storage_wipe`).
//! `StateGated` is dynamic and decided at execute time, not here.

use super::types::AvailabilityStatic;
use serde_json::Value;

/// Classify the static availability of an action whose handler is
/// `handler_chain` (an `ActionList` JSON array). The owning node is
/// inspected for `semantics.aiHidden`; explicit `false` un-gates a
/// would-be `ConfirmGated` action so authors can deliberately expose
/// destructive surface.
pub fn classify(node: &Value, handler_chain: Option<&Value>) -> AvailabilityStatic {
    let ai_hidden = node
        .get("semantics")
        .and_then(|s| s.get("aiHidden"))
        .and_then(|v| v.as_bool());

    if ai_hidden == Some(true) {
        return AvailabilityStatic::StaticHidden;
    }

    let destructive = handler_chain
        .map(|h| chain_is_destructive(h))
        .unwrap_or(false);

    match (destructive, ai_hidden) {
        // Handler is destructive AND author hasn't deliberately opened
        // the gate → ConfirmGated.
        (true, None) => AvailabilityStatic::ConfirmGated,
        // Handler is destructive but author set aiHidden:false → they
        // explicitly want the agent to be able to call it. Available.
        (true, Some(false)) => AvailabilityStatic::Available,
        _ => AvailabilityStatic::Available,
    }
}

/// Does this `ActionList` contain any verb the runtime treats as
/// destructive? Walks nested `if` / `for_each` / `parallel` / `race` /
/// `confirm` bodies so a destructive verb buried inside a control
/// flow still flips the gate.
pub fn chain_is_destructive(handler: &Value) -> bool {
    let Some(arr) = handler.as_array() else {
        return false;
    };
    arr.iter().any(action_is_destructive)
}

fn action_is_destructive(action: &Value) -> bool {
    let Some(obj) = action.as_object() else {
        return false;
    };
    for (verb, body) in obj {
        match verb.as_str() {
            // Direct destructive verbs.
            "confirm" | "storage_clear" | "storage_wipe" => return true,
            // fetch is destructive only on POST/PUT/PATCH/DELETE.
            "fetch" => {
                let method = body
                    .get("method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("GET")
                    .to_ascii_uppercase();
                if matches!(
                    method.as_str(),
                    "POST" | "PUT" | "PATCH" | "DELETE"
                ) {
                    return true;
                }
            }
            // Control-flow verbs — recurse into nested ActionLists.
            "if" => {
                if recurse_branch(body.get("then")) || recurse_branch(body.get("else")) {
                    return true;
                }
            }
            "for_each" => {
                if recurse_branch(body.get("do")) {
                    return true;
                }
            }
            "parallel" | "race" => {
                if recurse_branch(body.get("actions")) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn recurse_branch(branch: Option<&Value>) -> bool {
    branch.map(chain_is_destructive).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ai_hidden_true_marks_static_hidden() {
        let node = json!({ "semantics": { "aiHidden": true } });
        let chain = json!([{ "set": { "$state.x": "1" } }]);
        assert_eq!(
            classify(&node, Some(&chain)),
            AvailabilityStatic::StaticHidden
        );
    }

    #[test]
    fn safe_handler_is_available() {
        let node = json!({});
        let chain = json!([{ "set": { "$state.x": "1" } }]);
        assert_eq!(
            classify(&node, Some(&chain)),
            AvailabilityStatic::Available
        );
    }

    #[test]
    fn destructive_handler_is_confirm_gated_by_default() {
        let node = json!({});
        let chain = json!([{ "storage_wipe": null }]);
        assert_eq!(
            classify(&node, Some(&chain)),
            AvailabilityStatic::ConfirmGated
        );
    }

    #[test]
    fn ai_hidden_false_unlocks_destructive_handler() {
        let node = json!({ "semantics": { "aiHidden": false } });
        let chain = json!([{ "fetch": { "url": "/", "method": "DELETE" } }]);
        assert_eq!(
            classify(&node, Some(&chain)),
            AvailabilityStatic::Available
        );
    }

    #[test]
    fn fetch_get_is_safe() {
        let chain = json!([{ "fetch": { "url": "/api", "method": "GET" } }]);
        assert!(!chain_is_destructive(&chain));
    }

    #[test]
    fn fetch_post_is_destructive() {
        let chain = json!([{ "fetch": { "url": "/api", "method": "POST" } }]);
        assert!(chain_is_destructive(&chain));
    }

    #[test]
    fn nested_destructive_verb_still_trips() {
        let chain = json!([
          { "if": {
              "expr": "true",
              "then": [ { "storage_clear": null } ]
          }}
        ]);
        assert!(chain_is_destructive(&chain));
    }

    #[test]
    fn parallel_destructive() {
        let chain = json!([{ "parallel": { "actions": [ { "confirm": {} } ] } }]);
        assert!(chain_is_destructive(&chain));
    }
}
