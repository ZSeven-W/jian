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

    let destructive = handler_chain.map(chain_is_destructive).unwrap_or(false);

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
/// destructive? Walks nested `if` / `for_each` / `parallel` / `race`
/// bodies so a destructive verb buried inside a control flow still
/// flips the gate. Bounded recursion (`MAX_DEPTH`) — pathological
/// authored JSON that exceeds the depth is conservatively flagged
/// destructive (better to gate-confirm than to overflow the stack).
pub fn chain_is_destructive(handler: &Value) -> bool {
    chain_destructive_depth(handler, 0)
}

const MAX_DEPTH: usize = 64;

fn chain_destructive_depth(handler: &Value, depth: usize) -> bool {
    if depth >= MAX_DEPTH {
        // Over-deep authored handler — fail safe to ConfirmGated.
        return true;
    }
    let Some(arr) = handler.as_array() else {
        return false;
    };
    arr.iter().any(|a| action_is_destructive(a, depth + 1))
}

fn action_is_destructive(action: &Value, depth: usize) -> bool {
    if depth >= MAX_DEPTH {
        return true;
    }
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
                if matches!(method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE") {
                    return true;
                }
            }
            // Control-flow verbs — recurse into nested ActionLists,
            // bumping `depth` so a malicious deeply-nested structure
            // can't blow the stack. Match-guards instead of nested
            // `if` blocks keep clippy's `collapsible_if` happy on
            // rust ≥ 1.95.
            "if" if recurse_branch(body.get("then"), depth + 1)
                || recurse_branch(body.get("else"), depth + 1) =>
            {
                return true;
            }
            "for_each" if recurse_branch(body.get("do"), depth + 1) => return true,
            "parallel" | "race" if scan_parallel_body(body, depth + 1) => return true,
            _ => {}
        }
    }
    false
}

/// Walk a `parallel` / `race` body. Each entry is either an action
/// object OR an ActionList array — match `make_parallel_body`'s
/// runtime acceptance.
fn scan_parallel_body(body: &Value, depth: usize) -> bool {
    if depth >= MAX_DEPTH {
        return true;
    }
    let Some(arr) = body.as_array() else {
        return false;
    };
    arr.iter().any(|item| {
        if item.is_array() {
            chain_destructive_depth(item, depth + 1)
        } else {
            action_is_destructive(item, depth + 1)
        }
    })
}

fn recurse_branch(branch: Option<&Value>, depth: usize) -> bool {
    branch
        .map(|b| chain_destructive_depth(b, depth))
        .unwrap_or(false)
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
        assert_eq!(classify(&node, Some(&chain)), AvailabilityStatic::Available);
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
        assert_eq!(classify(&node, Some(&chain)), AvailabilityStatic::Available);
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
    fn parallel_array_body_destructive() {
        // `parallel` body is an array of action objects (or nested
        // ActionList arrays) — the previous `{actions: [...]}` form
        // didn't match the runtime's actual parser.
        let chain = json!([{ "parallel": [ { "confirm": {} } ] }]);
        assert!(chain_is_destructive(&chain));
    }

    #[test]
    fn parallel_array_of_actionlists_destructive() {
        let chain = json!([
          { "parallel": [
              [ { "set": { "$state.x": "1" } } ],
              [ { "storage_wipe": null } ]
          ]}
        ]);
        assert!(chain_is_destructive(&chain));
    }

    #[test]
    fn race_destructive_in_array_body() {
        let chain = json!([{ "race": [ { "fetch": { "url": "/", "method": "POST" } } ] }]);
        assert!(chain_is_destructive(&chain));
    }

    #[test]
    fn pathological_depth_fails_safe_to_destructive() {
        // Hand-build a 200-deep `if/then` chain — well above
        // MAX_DEPTH (64). Should classify as destructive without
        // overflowing the stack.
        let mut current = json!([]);
        for _ in 0..200 {
            current = json!([{ "if": { "expr": "true", "then": current } }]);
        }
        assert!(chain_is_destructive(&current));
    }

    #[test]
    fn moderate_depth_chain_evaluates_correctly() {
        // 32 levels — under the guard, real evaluation should still
        // work and find the destructive verb at the bottom.
        let mut current = json!([{ "storage_wipe": null }]);
        for _ in 0..32 {
            current = json!([{ "if": { "expr": "true", "then": current } }]);
        }
        assert!(chain_is_destructive(&current));
    }
}
