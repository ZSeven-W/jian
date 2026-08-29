//! R3 Preview Action Policy — the fixed allowlist, structured rejection,
//! sibling continuation, and ordered host effects through the EffectSink.

use jian_core::action::policy::{ActionPolicy, PreviewActionPolicy};
use jian_core::action::services::effect_sink::{EffectOutcome, EffectRequest, EffectSink};
use jian_core::action::services::NullFeedback;
use jian_core::action::{execute_list_async, ExecOutcome};
use jian_core::Runtime;
use std::cell::RefCell;
use std::rc::Rc;

/// Minimal loadable document: the policy tests drive actions directly,
/// no tree needed.
const EMPTY_DOC: &str = r##"{
    "version": "1.1", "formatVersion": "1.1", "id": "x",
    "app": { "name": "x", "version": "1", "id": "x",
             "capabilities": ["clipboard", "network", "notifications", "haptic"] },
    "children": []
}"##;

/// Records every effect request in dispatch order, with the activation
/// id the chain carried, and accepts everything.
#[derive(Default)]
struct RecordingSink {
    requests: RefCell<Vec<(String, Option<u64>)>>,
}

impl RecordingSink {
    fn names(&self) -> Vec<String> {
        self.requests
            .borrow()
            .iter()
            .map(|(n, _)| n.clone())
            .collect()
    }
}

impl EffectSink for RecordingSink {
    fn request(
        &self,
        ctx: &jian_core::action::context::EffectRequestContext,
        request: &EffectRequest,
    ) -> EffectOutcome {
        let name = match request {
            EffectRequest::OpenUrl { .. } => "open_url",
            EffectRequest::Copy { .. } => "copy",
            EffectRequest::Share { .. } => "share",
            EffectRequest::Haptic { .. } => "haptic",
            EffectRequest::FocusNode { .. } => "focus",
            EffectRequest::BlurFocus => "blur",
            EffectRequest::DismissKeyboard => "dismiss_keyboard",
            EffectRequest::Toast { .. } => "toast",
            EffectRequest::Alert { .. } => "alert",
            EffectRequest::Confirm { .. } => "confirm",
        };
        self.requests
            .borrow_mut()
            .push((name.to_owned(), ctx.activation));
        EffectOutcome::Accepted
    }
}

fn runtime_with_policy(sink: &Rc<RecordingSink>) -> Runtime {
    let sink_dyn: Rc<dyn EffectSink> = sink.clone();
    let mut rt = Runtime::new();
    rt.load_str(EMPTY_DOC).expect("load doc");
    rt.set_effect_sink(sink_dyn);
    let policy: Rc<dyn ActionPolicy> = Rc::new(PreviewActionPolicy::policy());
    rt.set_policy(Some(policy));
    rt
}

fn run(rt: &mut Runtime, list: serde_json::Value) -> ExecOutcome {
    let registry = rt.actions.clone();
    let ctx = rt.make_action_ctx();
    let outcome = {
        let registry = registry.borrow();
        futures::executor::block_on(execute_list_async(&registry, &list, &ctx))
    };
    rt.scheduler.flush();
    outcome
}

/// The approved catalog is the EXACT allowlist: every Preview-authorable
/// action name passes, and the known-unsafe vocabulary is absent.
#[test]
fn preview_allowlist_is_the_exact_safe_catalog() {
    let policy = PreviewActionPolicy::policy();
    for name in PreviewActionPolicy::ALLOWED {
        assert!(policy.check(name).is_ok(), "`{name}` is authorable");
    }
    for denied in [
        "fetch",
        "ws_connect",
        "ws_send",
        "ws_close",
        "storage_wipe",
        "notify",
        "paste",
        "race",
        "call",
        "vibrate",
    ] {
        assert!(
            policy.check(denied).is_err(),
            "`{denied}` must not be Preview-authorable"
        );
    }
}

/// Every unsafe action is rejected by POLICY (not capability, not
/// execution) with the structured diagnostic — even though their
/// capability gates are declared.
#[test]
fn unsafe_actions_are_rejected_with_policy_rejected() {
    let sink = Rc::new(RecordingSink::default());
    let mut rt = runtime_with_policy(&sink);
    for (action, body) in [
        (
            "fetch",
            serde_json::json!({"fetch": {"url": "'https://x'"}}),
        ),
        (
            "ws_connect",
            serde_json::json!({"ws_connect": {"id": "'c'", "url": "'wss://x'"}}),
        ),
        ("storage_wipe", serde_json::json!({"storage_wipe": {}})),
        ("notify", serde_json::json!({"notify": {"title": "'t'"}})),
        ("paste", serde_json::json!({"paste": {"into": "$app.x"}})),
        ("race", serde_json::json!({"race": []})),
        (
            "call",
            serde_json::json!({"call": {"module": "'m'", "function": "'f'"}}),
        ),
    ] {
        let outcome = run(&mut rt, serde_json::json!([body]));
        // The rejection is non-fatal BY DESIGN: the chain reports Ok and
        // the structured diagnostic lands in the warnings.
        assert!(
            outcome.result.is_ok(),
            "`{action}` rejection must not fail the chain"
        );
        assert!(
            outcome.warnings.iter().any(|w| w
                .message
                .contains(&format!("policy rejected action `{action}`"))),
            "`{action}` must carry a PolicyRejected diagnostic, got {:?}",
            outcome.warnings
        );
    }
    assert!(
        sink.names().is_empty(),
        "denied actions never reach the effect sink"
    );
}

/// A rejected action must not swallow its later safe siblings: the
/// sibling `set` still executes.
#[test]
fn rejected_action_lets_later_safe_siblings_run() {
    let sink = Rc::new(RecordingSink::default());
    let mut rt = runtime_with_policy(&sink);
    let outcome = run(
        &mut rt,
        serde_json::json!([
            {"notify": {"title": "'blocked'"}},
            {"set": {"$app.after": "$app.after + 1"}}
        ]),
    );
    assert!(outcome.result.is_ok(), "the chain itself continues");
    assert_eq!(
        rt.state.app_get("after").and_then(|v| v.as_i64()),
        Some(1),
        "the safe sibling executed after the policy rejection"
    );
}

/// The already-registered effect actions create ordered effects at the
/// sink, with the chain's activation id attached.
#[test]
fn approved_actions_create_ordered_effects_with_activation() {
    let sink = Rc::new(RecordingSink::default());
    let mut rt = runtime_with_policy(&sink);
    rt.set_activation(Some(42));
    let outcome = run(
        &mut rt,
        serde_json::json!([
            {"open_url": {"url": "'https://openpencil.dev'"}},
            {"copy": {"text": "'hello'"}},
            {"haptic": {"style": "light"}},
            {"focus": {"nodeId": "email"}},
            {"blur": {}},
            {"toast": {"message": "'saved'"}},
            {"alert": {"title": "'T'", "message": "'M'"}},
            {"share": {"text": "'payload'"}},
            {"confirm": {"title": "'C'", "message": "'Go?'"}}
        ]),
    );
    assert!(outcome.result.is_ok(), "all approved actions run");
    assert_eq!(
        sink.names(),
        vec!["open_url", "copy", "haptic", "focus", "blur", "toast", "alert", "share", "confirm",],
        "effects arrive at the sink in chain order"
    );
    assert!(
        sink.requests.borrow().iter().all(|(_, a)| *a == Some(42)),
        "every effect carries the chain's activation id"
    );
}

/// Activation certifies exactly ONE synchronous chain: a second context
/// (no fresh set_activation) sees `None`.
#[test]
fn activation_is_expired_after_one_chain() {
    let sink = Rc::new(RecordingSink::default());
    let mut rt = runtime_with_policy(&sink);
    rt.set_activation(Some(7));
    run(&mut rt, serde_json::json!([{"copy": {"text": "'a'"}}]));
    run(&mut rt, serde_json::json!([{"copy": {"text": "'b'"}}]));
    let activations: Vec<Option<u64>> = sink.requests.borrow().iter().map(|(_, a)| *a).collect();
    assert_eq!(activations, vec![Some(7), None]);
}

/// Without a policy, everything still executes (legacy behavior
/// preserved) and the sink still receives effect requests.
#[test]
fn no_policy_keeps_every_action_executable() {
    let sink = Rc::new(RecordingSink::default());
    let mut rt = Runtime::new();
    rt.load_str(EMPTY_DOC).expect("load doc");
    let sink_dyn: Rc<dyn EffectSink> = sink.clone();
    rt.set_effect_sink(sink_dyn);
    let outcome = run(
        &mut rt,
        serde_json::json!([
            {"notify": {"title": "'t'"}},
            {"set": {"$app.after": "1"}}
        ]),
    );
    assert!(outcome.result.is_ok(), "no policy: stubs still succeed");
    assert!(
        sink.names().is_empty(),
        "no sink request from non-effect actions"
    );
    let _ = NullFeedback;
}
