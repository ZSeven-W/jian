//! `wait_for` and `assert` runtime-coupled verbs (Plan 18 Phase 3).
//!
//! Both verbs evaluate a Jian expression against the runtime's
//! `StateGraph` and react to truthiness. `assert` is one-shot:
//! evaluate, succeed/fail. `wait_for` polls every ~16 ms until
//! either the expression goes truthy OR `timeout_ms` elapses;
//! between polls it `runtime.tick(now)` to drive the scheduler so
//! deferred bindings / signal updates that the expression depends
//! on actually run.

use std::time::{Duration, Instant};

use jian_core::expression::Expression;
use jian_core::value::RuntimeValue;
use jian_core::Runtime;

use crate::protocol::OutcomePayload;

/// Default `wait_for` timeout when the agent omits `timeout_ms`.
/// Picked so a slow-but-eventual binding (~5 frames) resolves
/// without the agent blocking forever.
const DEFAULT_WAIT_FOR_TIMEOUT_MS: u64 = 5_000;

/// Polling interval for `wait_for`. 16 ms ≈ one 60 Hz frame so
/// the runtime gets at least one tick per visible refresh.
const WAIT_FOR_POLL_MS: u64 = 16;

/// `assert` verb. Compiles `expr`, evaluates against the current
/// state graph, and surfaces `ok`/`fail` based on truthiness.
/// Compile-time errors come back as `Invalid`; eval-time
/// diagnostics ride along as `hints`.
pub fn run_assert(runtime: &Runtime, expr: &str) -> OutcomePayload {
    let compiled = match Expression::compile(expr) {
        Ok(c) => c,
        Err(d) => {
            return OutcomePayload::invalid(
                "assert",
                &format!("could not compile `{}`: {}", expr, d.message),
            )
        }
    };
    let (value, warnings) = compiled.eval(&runtime.state, None, None);
    let truthy = is_truthy(&value);
    let mut payload = if truthy {
        OutcomePayload::ok(
            "assert",
            Some(expr.to_owned()),
            format!("`{}` evaluated to truthy", expr),
        )
    } else {
        OutcomePayload::error(
            "assert",
            &format!("`{}` evaluated to falsy ({:?})", expr, value),
        )
    };
    for w in warnings {
        payload = payload.with_hint(w.message);
    }
    payload
}

/// `wait_for` verb. Polls the same expression-eval path on a
/// short cadence (`WAIT_FOR_POLL_MS`) until truthy or the
/// `timeout_ms` budget runs out. Each poll includes a
/// `runtime.tick(now)` so deferred work the expression depends
/// on (signal subscriptions, recogniser timeouts, etc) actually
/// gets a chance to run between checks.
pub fn run_wait_for(runtime: &mut Runtime, expr: &str, timeout_ms: Option<u64>) -> OutcomePayload {
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(DEFAULT_WAIT_FOR_TIMEOUT_MS));
    let compiled = match Expression::compile(expr) {
        Ok(c) => c,
        Err(d) => {
            return OutcomePayload::invalid(
                "wait_for",
                &format!("could not compile `{}`: {}", expr, d.message),
            )
        }
    };
    let started = Instant::now();
    loop {
        let (value, _warnings) = compiled.eval(&runtime.state, None, None);
        if is_truthy(&value) {
            let elapsed = started.elapsed();
            return OutcomePayload::ok(
                "wait_for",
                Some(expr.to_owned()),
                format!(
                    "`{}` became truthy after {} ms",
                    expr,
                    elapsed.as_millis()
                ),
            );
        }
        if started.elapsed() >= timeout {
            return OutcomePayload::timeout("wait_for", expr, timeout.as_millis() as u64);
        }
        // Drive the scheduler so deferred work + recogniser
        // timeouts can run between polls. `runtime.tick(now)`
        // is the same hook the host's frame loop uses.
        runtime.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(WAIT_FOR_POLL_MS));
    }
}

/// Truthiness rules: `false` / `null` / `0` / `""` are falsy;
/// everything else (including `[]`, `{}`) is truthy. Matches
/// the runtime's binding-evaluation truthiness for `visible` /
/// `disabled` so an agent's `assert visible` / `wait_for
/// disabled` agree with what the renderer sees.
fn is_truthy(v: &RuntimeValue) -> bool {
    use serde_json::Value;
    match &v.0 {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        // arrays / objects are truthy regardless of contents
        Value::Array(_) | Value::Object(_) => true,
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

    fn fixture() -> &'static str {
        r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "state":{
            "ready":{"type":"bool","default":false},
            "count":{"type":"int","default":0}
          },
          "children":[
            { "type":"frame","id":"root","width":480,"height":320,"x":0,"y":0,"children":[] }
          ]
        }"##
    }

    #[test]
    fn assert_truthy_returns_ok() {
        let mut rt = rt_with(fixture());
        rt.state.app_set("ready", serde_json::json!(true));
        let out = run_assert(&rt, "$app.ready == true");
        assert!(out.ok, "got {:?}", out);
    }

    #[test]
    fn assert_falsy_returns_error() {
        let mut rt = rt_with(fixture());
        rt.state.app_set("count", serde_json::json!(0));
        let out = run_assert(&rt, "$app.count > 5");
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("RuntimeError"));
    }

    #[test]
    fn assert_compile_error_returns_invalid() {
        let rt = rt_with(fixture());
        let out = run_assert(&rt, "$ + ! ?");
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("Invalid"));
    }

    #[test]
    fn wait_for_succeeds_when_already_truthy() {
        let mut rt = rt_with(fixture());
        rt.state.app_set("ready", serde_json::json!(true));
        let out = run_wait_for(&mut rt, "$app.ready == true", Some(100));
        assert!(out.ok);
        assert!(out.narrative.contains("became truthy"));
    }

    #[test]
    fn wait_for_times_out_when_never_true() {
        // No state mutation; the expression stays falsy forever.
        // Use a tight timeout so the test stays quick.
        let mut rt = rt_with(fixture());
        let out = run_wait_for(&mut rt, "$app.ready == true", Some(50));
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("Timeout"));
    }

    #[test]
    fn wait_for_compile_error_short_circuits() {
        // A compile error is reported before the poll loop, so
        // the test runs in microseconds even with a long timeout.
        let mut rt = rt_with(fixture());
        let out = run_wait_for(&mut rt, "$ + ! ?", Some(60_000));
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("Invalid"));
    }

    #[test]
    fn is_truthy_handles_jian_value_kinds() {
        let t = RuntimeValue(serde_json::json!(true));
        let f = RuntimeValue(serde_json::json!(false));
        let zero = RuntimeValue(serde_json::json!(0));
        let one = RuntimeValue(serde_json::json!(1));
        let empty = RuntimeValue(serde_json::json!(""));
        let s = RuntimeValue(serde_json::json!("x"));
        let null = RuntimeValue(serde_json::json!(null));
        let arr = RuntimeValue(serde_json::json!([]));
        let obj = RuntimeValue(serde_json::json!({}));
        assert!(is_truthy(&t));
        assert!(!is_truthy(&f));
        assert!(!is_truthy(&zero));
        assert!(is_truthy(&one));
        assert!(!is_truthy(&empty));
        assert!(is_truthy(&s));
        assert!(!is_truthy(&null));
        assert!(is_truthy(&arr));
        assert!(is_truthy(&obj));
    }
}
