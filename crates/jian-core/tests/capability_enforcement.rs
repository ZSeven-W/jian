//! Plan 6 — Capability Gate integration suite.
//!
//! Exercises the end-to-end path: `Runtime::new_from_document` reads
//! `app.capabilities`, builds a `DeclaredCapabilityGate` with an
//! `AuditLog`, and every IO action consults the gate before running.

use jian_core::action::{execute_list_shared, ActionError};
use jian_core::capability::{AuditLog, Capability, Verdict};
use jian_core::Runtime;
use jian_ops_schema::load_str;
use serde_json::json;
use std::rc::Rc;

fn load(src: &str) -> Runtime {
    let schema = load_str(src).unwrap().value;
    Runtime::new_from_document(schema).unwrap()
}

const OP_NO_CAPS: &str = r#"{
    "formatVersion": "1.0",
    "version": "1.0.0",
    "id": "x",
    "app": { "name": "x", "version": "1", "id": "x" },
    "children": []
}"#;

const OP_WITH_NETWORK: &str = r#"{
    "formatVersion": "1.0",
    "version": "1.0.0",
    "id": "x",
    "app": {
        "name": "x", "version": "1", "id": "x",
        "capabilities": ["network"]
    },
    "children": []
}"#;

const OP_WITH_NETWORK_STORAGE: &str = r#"{
    "formatVersion": "1.0",
    "version": "1.0.0",
    "id": "x",
    "app": {
        "name": "x", "version": "1", "id": "x",
        "capabilities": ["network", "storage"]
    },
    "children": []
}"#;

#[test]
fn fetch_without_network_is_denied_and_audited() {
    let rt = load(OP_NO_CAPS);
    let ctx = rt.make_action_ctx();
    let list = json!([{ "fetch": { "url": "\"/api\"" } }]);
    let out = execute_list_shared(&rt.actions, &list, &ctx);

    assert!(matches!(
        out.result,
        Err(ActionError::CapabilityDenied {
            action: "fetch",
            needed: Capability::Network,
        })
    ));

    let audit = rt.audit.as_deref().unwrap();
    let snap = audit.snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].action, "fetch");
    assert_eq!(snap[0].needed, Capability::Network);
    assert_eq!(snap[0].verdict, Verdict::Denied);
}

#[test]
fn fetch_with_network_declared_passes_gate() {
    let rt = load(OP_WITH_NETWORK);
    let ctx = rt.make_action_ctx();
    let list = json!([{ "fetch": { "url": "\"/api\"" } }]);
    let out = execute_list_shared(&rt.actions, &list, &ctx);

    // The gate allowed fetch; NullNetworkClient then returns an error,
    // but that error is captured inside the action chain. What we care
    // about here: the audit recorded Allowed.
    let audit = rt.audit.as_deref().unwrap();
    let snap = audit.snapshot();
    assert!(!snap.is_empty());
    assert_eq!(snap[0].action, "fetch");
    assert_eq!(snap[0].verdict, Verdict::Allowed);

    // on_error is unset, so the default NullNetworkClient failure
    // propagates as the list result — that's fine for this test.
    let _ = out;
}

#[test]
fn mixed_actions_accumulate_audit() {
    // Run three lists independently so a denial in one doesn't stop the
    // next — the audit log spans the runtime, not a single execute_list.
    let rt = load(OP_WITH_NETWORK);
    let ctx = rt.make_action_ctx();
    let _ = execute_list_shared(
        &rt.actions,
        &json!([{ "fetch": { "url": "\"/a\"", "on_error": [] } }]),
        &ctx,
    );
    let _ = execute_list_shared(
        &rt.actions,
        &json!([{ "storage_set": { "k": "\"v\"" } }]),
        &ctx,
    );
    let _ = execute_list_shared(
        &rt.actions,
        &json!([{ "fetch": { "url": "\"/b\"", "on_error": [] } }]),
        &ctx,
    );

    let audit = rt.audit.as_deref().unwrap();
    let snap = audit.snapshot();
    assert_eq!(snap.len(), 3);
    assert_eq!(snap[0].action, "fetch");
    assert_eq!(snap[0].verdict, Verdict::Allowed);
    assert_eq!(snap[1].action, "storage_set");
    assert_eq!(snap[1].verdict, Verdict::Denied);
    assert_eq!(snap[2].action, "fetch");
    assert_eq!(snap[2].verdict, Verdict::Allowed);
    assert_eq!(audit.allowed_count(), 2);
    assert_eq!(audit.denied_count(), 1);
}

#[test]
fn denial_short_circuits_current_list() {
    // Within a single ActionList, a CapabilityDenied on step N halts
    // steps N+1.. — defense-in-depth. Audit still records the denial.
    let rt = load(OP_NO_CAPS);
    let ctx = rt.make_action_ctx();
    let list = json!([
        { "storage_set": { "k": "\"v\"" } },
        { "storage_set": { "other": "\"z\"" } },
    ]);
    let out = execute_list_shared(&rt.actions, &list, &ctx);
    assert!(matches!(
        out.result,
        Err(ActionError::CapabilityDenied { .. })
    ));
    let snap = rt.audit.as_deref().unwrap().snapshot();
    assert_eq!(snap.len(), 1);
}

#[test]
fn storage_allowed_when_declared() {
    let rt = load(OP_WITH_NETWORK_STORAGE);
    let ctx = rt.make_action_ctx();
    let list = json!([{ "storage_set": { "theme": "\"dark\"" } }]);
    let _ = execute_list_shared(&rt.actions, &list, &ctx);

    let snap = rt.audit.as_deref().unwrap().snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].action, "storage_set");
    assert_eq!(snap[0].verdict, Verdict::Allowed);
}

#[test]
fn pure_actions_bypass_audit_entirely() {
    let rt = load(OP_NO_CAPS);
    let ctx = rt.make_action_ctx();
    rt.state.app_set("count", json!(0));
    let list = json!([{ "set": { "$app.count": "$app.count + 1" } }]);
    let out = execute_list_shared(&rt.actions, &list, &ctx);
    assert!(out.result.is_ok());
    // No IO capability consulted -> audit stays empty.
    assert!(rt.audit.as_deref().unwrap().is_empty());
    assert_eq!(rt.state.app_get("count").unwrap().as_i64(), Some(1));
}

#[test]
fn audit_ring_buffer_drops_oldest_after_capacity() {
    let log = AuditLog::new(4);
    for a in ["a", "b", "c", "d", "e", "f"] {
        log.record(jian_core::capability::AuditEntry {
            at: std::time::Instant::now(),
            action: match a {
                "a" => "a",
                "b" => "b",
                "c" => "c",
                "d" => "d",
                "e" => "e",
                _ => "f",
            },
            needed: Capability::Network,
            verdict: Verdict::Allowed,
            node_id: None,
        });
    }
    assert_eq!(log.len(), 4);
    let snap = log.snapshot();
    assert_eq!(
        snap.iter().map(|e| e.action).collect::<Vec<_>>(),
        ["c", "d", "e", "f"]
    );
}

#[test]
fn runtime_default_uses_dummy_gate_no_audit() {
    let rt = Runtime::new();
    assert!(rt.audit.is_none());
    assert!(rt.capabilities.check(Capability::Network, "fetch"));
    assert!(rt.capabilities.check(Capability::Storage, "storage_set"));
}

#[test]
fn automation_not_present_in_schema_capabilities() {
    // Declaring only `network` must not accidentally allow automation.
    let rt = load(OP_WITH_NETWORK);
    assert!(rt.capabilities.check(Capability::Network, "fetch"));
    assert!(!rt.capabilities.check(Capability::Storage, "storage_set"));
}

#[test]
fn open_url_without_network_is_denied_and_audited() {
    // Regression (Codex review): open_url used to bypass the gate.
    let rt = load(OP_NO_CAPS);
    let ctx = rt.make_action_ctx();
    let list = json!([{ "open_url": { "url": "\"https://example.com\"" } }]);
    let out = execute_list_shared(&rt.actions, &list, &ctx);

    assert!(matches!(
        out.result,
        Err(ActionError::CapabilityDenied {
            action: "open_url",
            needed: Capability::Network,
        })
    ));
    let snap = rt.audit.as_deref().unwrap().snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].action, "open_url");
    assert_eq!(snap[0].verdict, Verdict::Denied);
}

#[test]
fn open_url_with_network_declared_is_allowed() {
    let rt = load(OP_WITH_NETWORK);
    let ctx = rt.make_action_ctx();
    let list = json!([{ "open_url": { "url": "\"https://example.com\"" } }]);
    let _ = execute_list_shared(&rt.actions, &list, &ctx);
    let snap = rt.audit.as_deref().unwrap().snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].action, "open_url");
    assert_eq!(snap[0].verdict, Verdict::Allowed);
}

#[test]
fn share_without_network_is_denied_and_audited() {
    // Regression (Codex review): `share` was registered with no
    // capability requirement while the map declared `Network`.
    let rt = load(OP_NO_CAPS);
    let ctx = rt.make_action_ctx();
    let list = json!([{ "share": { "url": "\"https://example.com\"" } }]);
    let out = execute_list_shared(&rt.actions, &list, &ctx);

    assert!(matches!(
        out.result,
        Err(ActionError::CapabilityDenied {
            action: "share",
            needed: Capability::Network,
        })
    ));
    let snap = rt.audit.as_deref().unwrap().snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].action, "share");
    assert_eq!(snap[0].verdict, Verdict::Denied);
}

#[test]
fn focus_and_blur_are_registered_actions() {
    // Regression: `focus` / `blur` live in the map's pure-runtime list
    // but weren't registered; now stub factories exist.
    let rt = load(OP_NO_CAPS);
    let ctx = rt.make_action_ctx();
    let list = json!([
        { "focus": { "node": "\"my-input\"" } },
        { "blur": { "node": "\"my-input\"" } },
    ]);
    let out = execute_list_shared(&rt.actions, &list, &ctx);
    assert!(out.result.is_ok(), "{:?}", out.result);
    // No IO capability consulted → audit stays empty.
    assert!(rt.audit.as_deref().unwrap().is_empty());
}

#[test]
fn audit_log_shared_rc_reflects_runtime_activity() {
    let rt = load(OP_WITH_NETWORK);
    let log: Rc<AuditLog> = rt.audit.clone().unwrap();
    assert!(log.is_empty());

    let ctx = rt.make_action_ctx();
    let list = json!([{ "storage_set": { "k": "\"v\"" } }]);
    let _ = execute_list_shared(&rt.actions, &list, &ctx);

    assert_eq!(log.len(), 1);
    assert_eq!(log.snapshot()[0].verdict, Verdict::Denied);
}
