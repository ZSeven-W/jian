//! Lifecycle dispatch — the synchronous-harvest contract shared by
//! `Runtime::dispatch_lifecycle` and `Runtime::spawn_lifecycle`.
//!
//! Both doc comments promise the same task path. `spawn_lifecycle`
//! originally spawned without the inline harvest its sibling performs,
//! so a host that spawned `onUnmount` and then read state observed the
//! still-mounted values — the writes were left waiting for the next
//! pump. These tests pin the settled-before-return behaviour.

use jian_core::Runtime;

/// Minimal loadable document: these tests hand the hook list to the
/// runtime directly, so no authored `lifecycle` block is needed.
const EMPTY_DOC: &str = r##"{
    "version": "1.1", "formatVersion": "1.1", "id": "x",
    "app": { "name": "x", "version": "1", "id": "x", "capabilities": [] },
    "children": []
}"##;

fn runtime() -> Runtime {
    let mut rt = Runtime::new();
    rt.load_str(EMPTY_DOC).expect("load doc");
    rt
}

/// A spawned hook's `set` writes are visible the moment
/// `spawn_lifecycle` returns — no intervening `pump`.
#[test]
fn spawn_lifecycle_lands_its_writes_before_returning() {
    let mut rt = runtime();
    let spawned = rt.spawn_lifecycle(
        "onUnmount",
        serde_json::json!([{"set": {"$app.unmounted": "1"}}]),
        None,
        serde_json::json!({ "reason": "unmount" }),
    );
    assert!(spawned, "the pre-resolved hook list parses and spawns");
    assert_eq!(
        rt.state.app_get("unmounted").and_then(|v| v.as_i64()),
        Some(1),
        "the hook's write must land before spawn_lifecycle returns"
    );
}

/// The route-swap ordering a host relies on: four hooks spawned in
/// sequence each observe the previous one's writes, so the last value
/// reflects every hook in order rather than a batch replayed later.
#[test]
fn sequential_spawns_observe_each_other_in_order() {
    let mut rt = runtime();
    for (hook, value) in [
        ("onLeave", "1"),
        ("onUnmount", "$app.step + 1"),
        ("onMount", "$app.step + 1"),
        ("onEnter", "$app.step + 1"),
    ] {
        assert!(
            rt.spawn_lifecycle(
                hook,
                serde_json::json!([{"set": {"$app.step": value}}]),
                None,
                serde_json::json!({ "hook": hook }),
            ),
            "{hook} spawns"
        );
    }
    assert_eq!(
        rt.state.app_get("step").and_then(|v| v.as_i64()),
        Some(4),
        "every hook ran, in order, each seeing the previous write"
    );
}

/// A malformed hook list is reported as `false` rather than panicking
/// or half-applying.
#[test]
fn an_unparseable_hook_list_reports_failure() {
    let mut rt = runtime();
    let spawned = rt.spawn_lifecycle(
        "onMount",
        serde_json::json!([{"no_such_action": {}}]),
        None,
        serde_json::json!({}),
    );
    assert!(!spawned, "an unknown action name fails to parse");
    assert!(
        rt.state.app_get("step").is_none(),
        "nothing was written by the failed spawn"
    );
}
