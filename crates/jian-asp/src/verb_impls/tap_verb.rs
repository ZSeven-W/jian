//! `tap` verb implementation (Plan 18 Task 3 first verb).
//!
//! Resolves the selector, takes the first match's layout rect,
//! and dispatches Down → Up `PointerEvent`s through
//! `Runtime::dispatch_pointer` at the rect's centre. The runtime
//! routes through its own gesture arena and fires the same
//! `onTap` handler a real user-finger tap would, so any
//! state changes (counter increments, route pushes, etc) flow
//! back through the normal pipeline.
//!
//! Returns:
//! - `not_found` when the selector matches zero nodes.
//! - `invalid` when the matched node has no layout rect (e.g.
//!   `visible: false` so taffy didn't compute one).
//! - `ok` carrying the matched id, the rect's centre, and any
//!   `SemanticEvent`s the dispatch produced. Audit ring records
//!   one entry per outcome at the server-loop level.

use jian_core::gesture::pointer::{PointerEvent, PointerPhase};
use jian_core::Runtime;

use crate::protocol::{NodeSummary, OutcomePayload};
use crate::selector::Selector;
use crate::verb_impls::find_verb::collect_node_summaries;

/// Reserved pointer id for ASP-synthesised events. Chosen at the
/// top of u32 so a real mouse / touch id (typically 1..=N for a
/// few simultaneous fingers) can't collide with the agent's
/// taps. Hosts that bridge synthesised events into the gesture
/// arena route them through the same `dispatch_pointer` call,
/// but the unique id keeps the recogniser's per-arena state
/// separate so a finger held down by the user during an agent
/// tap doesn't get its arena state clobbered.
const ASP_POINTER_ID: u32 = u32::MAX;

/// Dispatch a synthesised tap on the first selector match. The
/// pointer id is fixed at `1` — the runtime's gesture arena
/// keys recognisers by id, and tap is single-pointer by
/// definition.
pub fn run_tap(runtime: &mut Runtime, sel: &Selector) -> OutcomePayload {
    let Some(doc) = runtime.document.as_ref() else {
        return OutcomePayload::error("tap", "no document loaded");
    };
    let hits = match sel.resolve(&doc.tree) {
        Ok(h) => h,
        Err(e) => return OutcomePayload::invalid("tap", &format!("{}", e)),
    };
    let Some(&first_key) = hits.first() else {
        return OutcomePayload::not_found("tap", "selector matched zero nodes");
    };
    // Project the matched node into a summary first so we can
    // borrow `runtime` immutably for layout. After this call the
    // immutable borrows on `doc` / layout drop and we can take a
    // `&mut` to dispatch.
    let summary: Option<NodeSummary> =
        collect_node_summaries(doc, &[first_key], runtime, 1)
            .into_iter()
            .next();
    let Some(summary) = summary else {
        return OutcomePayload::error("tap", "could not summarise matched node");
    };
    let rect = summary.rect;
    if rect[2] <= 0.0 || rect[3] <= 0.0 {
        // Width or height of zero usually means the node has no
        // computed layout — `visible: false`, a flex child whose
        // parent collapsed it, etc. Surface as `invalid` so the
        // agent retries after a `wait_for` rather than dispatching
        // into the void.
        return OutcomePayload::invalid(
            "tap",
            "matched node has zero layout rect (likely invisible / not laid out)",
        );
    }
    let cx = rect[0] + rect[2] / 2.0;
    let cy = rect[1] + rect[3] / 2.0;
    // Drop the immutable borrows before mutating the runtime.
    // (`let _ = doc;` instead of `drop(doc)` — `doc` is a
    // reference, and clippy warns on dropping references; the
    // assignment-to-`_` form has the same end-of-scope effect.)
    let id = summary.id.clone();
    let _ = doc;
    let down = PointerEvent::simple(
        ASP_POINTER_ID,
        PointerPhase::Down,
        jian_core::geometry::point(cx, cy),
    );
    let up = PointerEvent::simple(
        ASP_POINTER_ID,
        PointerPhase::Up,
        jian_core::geometry::point(cx, cy),
    );
    let down_events = runtime.dispatch_pointer(down);
    let up_events = runtime.dispatch_pointer(up);
    let total_semantic = down_events.len() + up_events.len();
    OutcomePayload::ok(
        "tap",
        Some(id.clone()),
        format!(
            "tapped node `{}` at ({:.1}, {:.1}); {} semantic event(s) fired",
            id, cx, cy, total_semantic
        ),
    )
    .with_hint(if total_semantic == 0 {
        "no handlers fired — confirm the node has `events.onTap` or a parent does (event bubbling)"
            .to_owned()
    } else {
        format!("{} semantic event(s) propagated through the gesture arena", total_semantic)
    })
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

    fn counter_doc() -> &'static str {
        r##"{
          "formatVersion": "1.0", "version": "1.0.0", "id": "tap-fx",
          "app": { "name": "tap-fx", "version": "1", "id": "tap-fx" },
          "state": { "count": { "type": "int", "default": 0 } },
          "children": [
            {
              "type": "frame", "id": "root", "width": 480, "height": 320, "x": 0, "y": 0,
              "children": [
                { "type": "rectangle", "id": "btn",
                  "x": 100, "y": 100, "width": 100, "height": 60,
                  "events": { "onTap": [ { "set": { "$app.count": "$app.count + 1" } } ] }
                }
              ]
            }
          ]
        }"##
    }

    #[test]
    fn tap_increments_counter_on_handler_node() {
        let mut rt = rt_with(counter_doc());
        let sel = Selector {
            id: Some("btn".into()),
            ..Default::default()
        };
        let out = run_tap(&mut rt, &sel);
        assert!(out.ok, "expected ok, got {:?}", out);
        // Pre-tap: 0. Post-tap: 1.
        let count = rt
            .state
            .app_get("count")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        assert_eq!(count, 1);
    }

    #[test]
    fn tap_with_no_match_returns_not_found() {
        let mut rt = rt_with(counter_doc());
        let sel = Selector {
            id: Some("does-not-exist".into()),
            ..Default::default()
        };
        let out = run_tap(&mut rt, &sel);
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("NotFound"));
    }

    #[test]
    fn tap_succeeds_on_handler_less_node_without_state_change() {
        // The matched node has no `events.onTap`, so the runtime's
        // gesture arena still emits the SemanticEvent (Tap) but no
        // user-installed handler runs — state stays untouched and
        // the verb still reports `ok` (the tap *was* dispatched).
        let doc = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "state":{"flag":{"type":"int","default":7}},
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
        let out = run_tap(&mut rt, &sel);
        assert!(out.ok);
        // State is untouched because no handler was wired.
        let flag = rt
            .state
            .app_get("flag")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        assert_eq!(flag, 7);
    }
}
