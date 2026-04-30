//! `swipe` verb implementation (Plan 18 Phase 3).
//!
//! Synthesises a pointer drag that crosses the pan recogniser's
//! threshold and ends past the requested distance. The gesture
//! arena claims the pan, emits `PanStart` / `PanUpdate` / `PanEnd`,
//! and the action-surface layer (when present) maps a fast pan
//! into the corresponding `swipe_<dir>_<slug>` action. Authors
//! who wired `events.onPanStart/Update/End` directly receive the
//! same semantic events a real touch-style flick produces.
//!
//! Convention: the pointer **moves** in the direction it is
//! swiping — swipe **up** moves the pointer's `y` *down* (toward
//! the top of the screen), matching the physical-finger model
//! everyone uses (and the `swipe_up_*` action-surface name an
//! agent calls). This is opposite the wheel convention used by
//! `scroll` (where `delta.y > 0` = scroll up), because pan and
//! wheel sit on different axes of the gesture pipeline.
//!
//! Returns:
//! - `not_found` when the selector matches zero nodes.
//! - `invalid` when the matched node has no layout rect.
//! - `ok` carrying the matched id, the start/end positions, and
//!   the count of `SemanticEvent`s the dispatch produced. `hint`
//!   flags zero-emit cases — the gesture arena claims even
//!   without a handler attached, so a zero count usually means
//!   the recogniser was suppressed (e.g. another arena claimed
//!   first).

use jian_core::geometry::{point, Point};
use jian_core::gesture::pointer::{PointerEvent, PointerPhase};
use jian_core::Runtime;

use crate::protocol::{OutcomePayload, ScrollDir};
use crate::selector::Selector;
use crate::verb_impls::find_verb::collect_node_summaries;

/// Reserved pointer id for ASP-synthesised events. Same value
/// `tap_verb` uses — different verb on the same session never
/// interleaves Down sequences, so a single reserved id is safe.
const ASP_POINTER_ID: u32 = u32::MAX;

/// Default swipe distance when `distance: None`. Crosses the pan
/// recogniser's 8px threshold by an order of magnitude so the
/// recogniser claims and the swipe registers as a flick.
const DEFAULT_SWIPE_DISTANCE: f32 = 120.0;

/// Minimum swipe magnitude. Below the pan recogniser's 8 px
/// threshold the gesture arena lets `Tap` claim instead — the
/// agent that called `swipe` would silently see an `onTap`
/// handler fire on the source node, masking the bug. Reject
/// such requests as `Invalid` rather than dispatching the
/// stray Down/Up sequence.
const MIN_SWIPE_DISTANCE: f32 = 8.0;

/// Number of intermediate Move events between Down and Up. Three
/// is the minimum to produce a `PanStart` (after the threshold is
/// crossed), followed by `PanUpdate`, plus a still-moving signal
/// that lets the action-surface layer compute a velocity.
const SWIPE_MOVE_STEPS: usize = 4;

pub fn run_swipe(
    runtime: &mut Runtime,
    sel: &Selector,
    direction: ScrollDir,
    distance: Option<f32>,
) -> OutcomePayload {
    let Some(doc) = runtime.document.as_ref() else {
        return OutcomePayload::error("swipe", "no document loaded");
    };
    let hits = match sel.resolve(&doc.tree) {
        Ok(h) => h,
        Err(e) => return OutcomePayload::invalid("swipe", &format!("{}", e)),
    };
    let Some(&first_key) = hits.first() else {
        return OutcomePayload::not_found("swipe", "selector matched zero nodes");
    };
    let summary = collect_node_summaries(doc, &[first_key], runtime, 1)
        .into_iter()
        .next();
    let Some(summary) = summary else {
        return OutcomePayload::error("swipe", "could not summarise matched node");
    };
    let rect = summary.rect;
    if rect[2] <= 0.0 || rect[3] <= 0.0 {
        return OutcomePayload::invalid(
            "swipe",
            "matched node has zero layout rect (likely invisible / not laid out)",
        );
    }
    let cx = rect[0] + rect[2] / 2.0;
    let cy = rect[1] + rect[3] / 2.0;
    let mag = distance.unwrap_or(DEFAULT_SWIPE_DISTANCE).abs();
    if mag < MIN_SWIPE_DISTANCE {
        return OutcomePayload::invalid(
            "swipe",
            &format!(
                "distance {:.1} is below the pan threshold ({} px) — \
                 a Down/Up at the same point would let the gesture \
                 arena claim Tap instead of Pan",
                mag, MIN_SWIPE_DISTANCE
            ),
        )
        .with_hint(
            "use a larger `distance` (>= 8 px) or call `tap` if a tap \
             is what you actually want"
                .to_owned(),
        );
    }
    let delta = swipe_delta(direction, mag);
    let start = point(cx, cy);
    let end = point(cx + delta.x, cy + delta.y);
    let id = summary.id.clone();
    let _ = doc;

    let mut total_emitted = 0usize;
    let down = PointerEvent::simple(ASP_POINTER_ID, PointerPhase::Down, start);
    total_emitted += runtime.dispatch_pointer(down).len();
    for step in 1..=SWIPE_MOVE_STEPS {
        let t = step as f32 / SWIPE_MOVE_STEPS as f32;
        let move_pos = point(cx + delta.x * t, cy + delta.y * t);
        let mv = PointerEvent::simple(ASP_POINTER_ID, PointerPhase::Move, move_pos);
        total_emitted += runtime.dispatch_pointer(mv).len();
    }
    let up = PointerEvent::simple(ASP_POINTER_ID, PointerPhase::Up, end);
    total_emitted += runtime.dispatch_pointer(up).len();

    OutcomePayload::ok(
        "swipe",
        Some(id.clone()),
        format!(
            "swiped {:?} on `{}`: ({:.1}, {:.1}) → ({:.1}, {:.1}); {} semantic event(s)",
            direction, id, start.x, start.y, end.x, end.y, total_emitted
        ),
    )
    .with_hint(if total_emitted == 0 {
        "no semantic events emitted — confirm the target (or an ancestor) has \
         `events.onPanStart/Update/End`, or expose a `swipe_*` action via the \
         action-surface layer"
            .to_owned()
    } else {
        format!(
            "{} pan-family event(s) emitted; check `events.onPan*` or action-surface `swipe_{}_<slug>`",
            total_emitted,
            scroll_dir_word(direction)
        )
    })
}

fn swipe_delta(dir: ScrollDir, magnitude: f32) -> Point {
    // Pointer moves in the swipe direction. Up = pointer y decreases.
    match dir {
        ScrollDir::Up => point(0.0, -magnitude),
        ScrollDir::Down => point(0.0, magnitude),
        ScrollDir::Left => point(-magnitude, 0.0),
        ScrollDir::Right => point(magnitude, 0.0),
    }
}

fn scroll_dir_word(dir: ScrollDir) -> &'static str {
    match dir {
        ScrollDir::Up => "up",
        ScrollDir::Down => "down",
        ScrollDir::Left => "left",
        ScrollDir::Right => "right",
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

    fn pannable_doc() -> &'static str {
        r##"{
          "formatVersion":"1.0","version":"1.0.0","id":"swipe-fx",
          "app":{"name":"swipe-fx","version":"1","id":"swipe-fx"},
          "state":{"started":{"type":"int","default":0},"ended":{"type":"int","default":0}},
          "children":[
            { "type":"frame","id":"card","x":40,"y":40,"width":200,"height":200,
              "events":{
                "onPanStart": [ { "set": { "$app.started": "$app.started + 1" } } ],
                "onPanEnd":   [ { "set": { "$app.ended":   "$app.ended + 1"   } } ]
              }
            }
          ]
        }"##
    }

    #[test]
    fn swipe_left_emits_pan_start_and_end() {
        let mut rt = rt_with(pannable_doc());
        let sel = Selector { id: Some("card".into()), ..Default::default() };
        let out = run_swipe(&mut rt, &sel, ScrollDir::Left, None);
        assert!(out.ok, "expected ok, got {:?}", out);
        let started = rt.state.app_get("started").and_then(|v| v.as_i64()).unwrap_or(0);
        let ended = rt.state.app_get("ended").and_then(|v| v.as_i64()).unwrap_or(0);
        assert!(started >= 1, "PanStart handler should run at least once");
        assert!(ended >= 1, "PanEnd handler should run at least once");
    }

    #[test]
    fn swipe_with_no_match_returns_not_found() {
        let mut rt = rt_with(pannable_doc());
        let sel = Selector { id: Some("nope".into()), ..Default::default() };
        let out = run_swipe(&mut rt, &sel, ScrollDir::Up, Some(80.0));
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("NotFound"));
    }

    #[test]
    fn swipe_direction_sign_convention() {
        // Up = pointer y decreases; Down = pointer y increases.
        let up = swipe_delta(ScrollDir::Up, 100.0);
        assert_eq!(up.y, -100.0);
        let down = swipe_delta(ScrollDir::Down, 100.0);
        assert_eq!(down.y, 100.0);
        let left = swipe_delta(ScrollDir::Left, 100.0);
        assert_eq!(left.x, -100.0);
        let right = swipe_delta(ScrollDir::Right, 100.0);
        assert_eq!(right.x, 100.0);
    }

    #[test]
    fn swipe_subthreshold_distance_returns_invalid() {
        let mut rt = rt_with(pannable_doc());
        let sel = Selector { id: Some("card".into()), ..Default::default() };
        // 4 < pan threshold 8 — must reject so a tap doesn't slip through.
        let out = run_swipe(&mut rt, &sel, ScrollDir::Right, Some(4.0));
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("Invalid"));
        let started = rt.state.app_get("started").and_then(|v| v.as_i64()).unwrap_or(0);
        let ended = rt.state.app_get("ended").and_then(|v| v.as_i64()).unwrap_or(0);
        assert_eq!(started, 0, "subthreshold swipe must not fire any handler");
        assert_eq!(ended, 0);
    }

    #[test]
    fn swipe_negative_distance_is_treated_as_magnitude() {
        let mut rt = rt_with(pannable_doc());
        let sel = Selector { id: Some("card".into()), ..Default::default() };
        // Distance -50; direction picks the sign. Right swipe → x
        // increases by |-50| = 50 from the matched card's centre.
        let out = run_swipe(&mut rt, &sel, ScrollDir::Right, Some(-50.0));
        assert!(out.ok);
        // Pull the actual centre out of the narrative's start tuple
        // ("...: (cx, cy) → (...)") — taffy's flex layout rules
        // determine the resolved rect, so we don't hardcode the
        // schema's `x`/`y` here.
        let narrative = out.narrative.clone();
        let start_open = narrative.find(": (").expect("start position present") + 3;
        let arrow = narrative[start_open..].find(") →").expect("start delimiter") + start_open;
        let start = &narrative[start_open..arrow];
        let mut parts = start.split(", ");
        let sx: f32 = parts.next().unwrap().parse().unwrap();
        let sy: f32 = parts.next().unwrap().parse().unwrap();
        let expected_end = format!("({:.1}, {:.1})", sx + 50.0, sy);
        assert!(
            narrative.contains(&format!("→ {}", expected_end)),
            "expected end {} after right swipe of magnitude 50 from start ({:.1}, {:.1}); got: {}",
            expected_end,
            sx,
            sy,
            narrative
        );
    }
}
