//! `scroll` verb implementation (Plan 18 Phase 3).
//!
//! Resolves the selector, takes the first match's layout-rect
//! centre, and dispatches a [`WheelEvent`] with a delta computed
//! from the requested direction and distance. The runtime
//! hit-tests at that position and bubbles up to the topmost
//! `events.onScroll` handler — no gesture-arena rivalry, so
//! wheel doesn't compete with Tap/Swipe.
//!
//! Convention (matches `jian_core::gesture::pointer::WheelEvent`'s
//! doc): positive `delta.y`
//! means *content moves up* (i.e. the user scrolled up). To keep
//! the verb's surface intuitive we mirror the OS convention —
//! `direction: Up` produces `delta.y = +distance`, so a list
//! whose handler reads `$event.dy > 0` to mean "scroll up" reads
//! correctly.
//!
//! Returns:
//! - `not_found` when the selector matches zero nodes.
//! - `invalid` when the matched node has no layout rect.
//! - `ok` carrying the matched id, the dispatch position, the
//!   delta, and the count of `SemanticEvent`s the dispatch
//!   produced. `hint` flags zero-emit cases so the agent can
//!   confirm the target had a handler.

use jian_core::geometry::{point, Point};
use jian_core::gesture::pointer::WheelEvent;
use jian_core::Runtime;

use crate::protocol::{OutcomePayload, ScrollDir};
use crate::selector::Selector;
use crate::verb_impls::find_verb::collect_node_summaries;

/// Default scroll distance when `distance: None` — a single mouse
/// wheel notch on every desktop platform's logical-pixel scale.
/// Matches winit's `MouseScrollDelta::LineDelta(_, 1.0)` reduction
/// in raster hosts.
const DEFAULT_SCROLL_DISTANCE: f32 = 120.0;

pub fn run_scroll(
    runtime: &mut Runtime,
    sel: &Selector,
    direction: ScrollDir,
    distance: Option<f32>,
) -> OutcomePayload {
    let Some(doc) = runtime.document.as_ref() else {
        return OutcomePayload::error("scroll", "no document loaded");
    };
    let hits = match sel.resolve(&doc.tree) {
        Ok(h) => h,
        Err(e) => return OutcomePayload::invalid("scroll", &format!("{}", e)),
    };
    let Some(&first_key) = hits.first() else {
        return OutcomePayload::not_found("scroll", "selector matched zero nodes");
    };
    let summary = collect_node_summaries(doc, &[first_key], runtime, 1)
        .into_iter()
        .next();
    let Some(summary) = summary else {
        return OutcomePayload::error("scroll", "could not summarise matched node");
    };
    let rect = summary.rect;
    if rect[2] <= 0.0 || rect[3] <= 0.0 {
        return OutcomePayload::invalid(
            "scroll",
            "matched node has zero layout rect (likely invisible / not laid out)",
        );
    }
    let cx = rect[0] + rect[2] / 2.0;
    let cy = rect[1] + rect[3] / 2.0;
    let mag = distance.unwrap_or(DEFAULT_SCROLL_DISTANCE).abs();
    if mag == 0.0 {
        // Zero-delta wheel still hit-tests + dispatches `onScroll`,
        // which is misleading: the agent asked the verb to do
        // nothing, but a handler fires and any `$event.dy`-reading
        // expression evaluates to 0 and may flip user state. Refuse
        // the no-op explicitly.
        return OutcomePayload::invalid(
            "scroll",
            "distance is zero — a zero-delta wheel would still trigger onScroll handlers",
        )
        .with_hint(
            "pass a non-zero `distance`, or call `inspect` if you only \
             need to confirm the target is on screen"
                .to_owned(),
        );
    }
    let delta = scroll_delta(direction, mag);
    let id = summary.id.clone();
    let _ = doc;

    let event = WheelEvent::simple(point(cx, cy), delta);
    let emitted = runtime.dispatch_wheel(event);
    let count = emitted.len();

    OutcomePayload::ok(
        "scroll",
        Some(id.clone()),
        format!(
            "scrolled {:?} at ({:.1}, {:.1}) by ({:.1}, {:.1}); {} semantic event(s)",
            direction, cx, cy, delta.x, delta.y, count
        ),
    )
    .with_hint(if count == 0 {
        "no handlers fired — confirm the target (or an ancestor) has `events.onScroll`".to_owned()
    } else {
        format!("{} `onScroll` handler(s) fired", count)
    })
}

fn scroll_delta(dir: ScrollDir, magnitude: f32) -> Point {
    match dir {
        ScrollDir::Up => point(0.0, magnitude),
        ScrollDir::Down => point(0.0, -magnitude),
        ScrollDir::Left => point(magnitude, 0.0),
        ScrollDir::Right => point(-magnitude, 0.0),
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

    fn scroll_doc() -> &'static str {
        r##"{
          "formatVersion": "1.0", "version": "1.0.0", "id": "scroll-fx",
          "app": { "name": "scroll-fx", "version": "1", "id": "scroll-fx" },
          "state": { "y": { "type": "int", "default": 0 } },
          "children": [
            { "type": "frame", "id": "list", "width": 480, "height": 320, "x": 0, "y": 0,
              "events": { "onScroll": [ { "set": { "$app.y": "$app.y + 1" } } ] }
            }
          ]
        }"##
    }

    #[test]
    fn scroll_down_fires_handler_on_target() {
        let mut rt = rt_with(scroll_doc());
        let sel = Selector {
            id: Some("list".into()),
            ..Default::default()
        };
        let out = run_scroll(&mut rt, &sel, ScrollDir::Down, None);
        assert!(out.ok, "expected ok, got {:?}", out);
        let v = rt.state.app_get("y").and_then(|v| v.as_i64()).unwrap_or(-1);
        assert_eq!(v, 1, "onScroll handler should have run once");
    }

    #[test]
    fn scroll_with_no_match_returns_not_found() {
        let mut rt = rt_with(scroll_doc());
        let sel = Selector {
            id: Some("nope".into()),
            ..Default::default()
        };
        let out = run_scroll(&mut rt, &sel, ScrollDir::Up, Some(80.0));
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("NotFound"));
    }

    #[test]
    fn scroll_on_handler_less_node_succeeds_with_hint() {
        // Wheel doesn't compete in the gesture arena, so a node
        // without `onScroll` simply emits zero SemanticEvents. The
        // verb still reports `ok` (the wheel *was* dispatched) and
        // hints why nothing fired.
        let doc = r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"x",
          "app":{"name":"x","version":"1","id":"x"},
          "children":[
            { "type":"frame","id":"plain","x":0,"y":0,"width":400,"height":300 }
          ]
        }"##;
        let mut rt = rt_with(doc);
        let sel = Selector {
            id: Some("plain".into()),
            ..Default::default()
        };
        let out = run_scroll(&mut rt, &sel, ScrollDir::Down, None);
        assert!(out.ok);
        assert!(
            out.hints.iter().any(|h| h.contains("no handlers fired")),
            "expected no-handlers hint, got {:?}",
            out.hints
        );
    }

    #[test]
    fn scroll_left_right_axis() {
        let d_up = scroll_delta(ScrollDir::Up, 100.0);
        assert_eq!(d_up.x, 0.0);
        assert_eq!(d_up.y, 100.0);
        let d_down = scroll_delta(ScrollDir::Down, 100.0);
        assert_eq!(d_down.y, -100.0);
        let d_left = scroll_delta(ScrollDir::Left, 100.0);
        assert_eq!(d_left.x, 100.0);
        assert_eq!(d_left.y, 0.0);
        let d_right = scroll_delta(ScrollDir::Right, 100.0);
        assert_eq!(d_right.x, -100.0);
    }

    #[test]
    fn scroll_zero_distance_returns_invalid() {
        let mut rt = rt_with(scroll_doc());
        let sel = Selector {
            id: Some("list".into()),
            ..Default::default()
        };
        let out = run_scroll(&mut rt, &sel, ScrollDir::Up, Some(0.0));
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("Invalid"));
        let v = rt.state.app_get("y").and_then(|v| v.as_i64()).unwrap_or(0);
        assert_eq!(v, 0, "zero-distance scroll must not trigger onScroll");
    }

    #[test]
    fn scroll_negative_distance_is_treated_as_magnitude() {
        // Some agents may pass `distance: -120` meaning "one notch
        // backwards". We always interpret the sign via the
        // `direction` field — `distance` is a magnitude.
        let mut rt = rt_with(scroll_doc());
        let sel = Selector {
            id: Some("list".into()),
            ..Default::default()
        };
        let out = run_scroll(&mut rt, &sel, ScrollDir::Up, Some(-50.0));
        assert!(out.ok);
        assert!(
            out.narrative.contains("0.0, 50.0)"),
            "negative distance should be flipped to positive magnitude"
        );
    }
}
