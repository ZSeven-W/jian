//! Handler-aware Tap/DoubleTap deferral helpers for `PointerRouter`.
//!
//! A completed Tap on a chain that declares an enabled `onDoubleTap` is
//! buffered as a [`PendingTap`] and flushed **exactly once** at its
//! deadline, either by `PointerRouter::tick` (timer drive) or by the top
//! of the dispatch path (input at/after the deadline). The helpers are
//! self-contained so the router's public dispatch entry points and the
//! runtime's due-order pipeline share one state machine with no
//! double-flush: the runtime flushes + delivers due actions before calling
//! `PointerRouter::dispatch_current`, which never flushes itself.
//!
//! Matching a second Tap requires the same logical owner and STRICTLY
//! before the deadline; at the exact deadline the window is closed (the
//! due flush already surfaced the first Tap), so input at the deadline
//! never pairs into a DoubleTap.

use super::config;
use super::semantic::{SemanticEvent, SemanticEventEnvelope};
use crate::document::{NodeKey, RuntimeDocument};
use crate::geometry::Point;

/// A buffered Tap awaiting a matching second Tap or the double-tap
/// deadline. Flush via `PointerRouter::tick(deadline)`.
pub(super) struct PendingTap {
    pub(super) envelope: SemanticEventEnvelope,
    /// Logical target: the node owning the enabled `onDoubleTap`.
    pub(super) owner: NodeKey,
    pub(super) up_ms: u64,
    pub(super) deadline_ms: u64,
    pub(super) timeout_ms: u64,
    pub(super) slop_px: f32,
}

impl PendingTap {
    /// The buffered Tap has reached its deadline and must be flushed.
    pub(super) fn is_due(&self, now_ms: u64) -> bool {
        now_ms >= self.deadline_ms
    }

    /// Does the arriving Tap pair with this buffered one? Matching
    /// requires the same logical owner and STRICTLY before the deadline
    /// (at the exact deadline the window is closed), with the second Up
    /// within `slop_px` of the first.
    pub(super) fn matches(&self, owner: NodeKey, up_ms: u64, position: Point) -> bool {
        if self.owner != owner {
            return false;
        }
        let dt = up_ms.saturating_sub(self.up_ms);
        let prev_pos = self
            .envelope
            .pointer_facts
            .as_ref()
            .map(|f| f.position)
            .unwrap_or_default();
        let dx = position.x - prev_pos.x;
        let dy = position.y - prev_pos.y;
        dt < self.timeout_ms && (dx * dx + dy * dy).sqrt() <= self.slop_px
    }
}

/// Flush a due pending Tap WITHOUT driving arena timers. Used while input
/// is frozen (parked variant swap) and at the top of the public dispatch
/// path: timer-driven recognizers do not claim inside the freeze, but a
/// deferred Tap whose deadline passed must not be consumed without
/// delivery.
pub(super) fn flush_pending_tap(
    pending: &mut Option<PendingTap>,
    now_ms: u64,
) -> Vec<SemanticEventEnvelope> {
    // Check due-ness BEFORE taking: a not-yet-due pending Tap must stay
    // buffered for a later tick (taking first would drop it).
    let due = pending.as_ref().is_some_and(|p| p.is_due(now_ms));
    if due {
        let pending = pending.take().expect("checked due above");
        vec![pending.envelope]
    } else {
        Vec::new()
    }
}

/// Handler-aware Tap deferral. See the `router` module docs for the state
/// machine; the precise rules are:
///
/// - ANY completed Tap closes an existing window: the old pending
///   flushes NOW, before the new Tap is classified (a competing tap
///   makes the old window impossible — including a Tap-only target,
///   which must also deliver the new Tap immediately and never pair
///   a later Tap with the stale buffer).
/// - chain declares no enabled `onDoubleTap` → new Tap passes through
///   (immediate); a flushed old pending precedes it.
/// - no pending tap and chain declares `onDoubleTap` → buffer the Tap;
///   it is removed from `out` and flushed by `tick` at its deadline.
/// - a matching second Tap (same owner, strictly before the authored
///   deadline, within slop) → only `DoubleTap` is emitted.
/// - a non-matching second Tap → the buffered Tap flushes NOW; the
///   new Tap starts a fresh buffer.
pub(super) fn apply_tap_deferral(
    pending: &mut Option<PendingTap>,
    out: &mut Vec<SemanticEventEnvelope>,
    doc: &RuntimeDocument,
    node_disabled: &dyn Fn(NodeKey) -> bool,
) {
    let Some(idx) = out
        .iter()
        .position(|envelope| matches!(envelope.event, SemanticEvent::Tap { .. }))
    else {
        return;
    };
    let tap = out.remove(idx);
    let node = tap.event.node();

    let prev = pending.take();

    if !config::chain_declares_enabled_with(doc, node, "onDoubleTap", node_disabled) {
        // Tap-only target: flush the old pending (if any) and deliver
        // the new Tap immediately, in chronological order.
        if let Some(prev) = prev {
            out.insert(idx, prev.envelope);
            out.insert(idx + 1, tap);
        } else {
            out.insert(idx, tap);
        }
        return;
    }
    let owner = config::chain_owner_with(doc, node, "onDoubleTap", node_disabled).unwrap_or(node);
    let cfg = config::gesture_config(doc, owner);
    let timeout_ms = cfg.effective_double_tap_timeout();
    let slop_px = cfg.effective_double_tap_slop();
    let up_ms = tap
        .pointer_facts
        .as_ref()
        .map(|f| f.t_ms)
        .unwrap_or_default();

    if let Some(prev) = prev {
        let pos = tap
            .pointer_facts
            .as_ref()
            .map(|f| f.position)
            .unwrap_or_default();
        if prev.matches(owner, up_ms, pos) {
            // Matching second Tap → only DoubleTap; no first/second
            // Tap is ever delivered for this pair.
            let envelope = SemanticEventEnvelope {
                event: SemanticEvent::DoubleTap {
                    node,
                    position: pos,
                },
                pointer_facts: tap.pointer_facts,
                gesture: Default::default(),
            };
            out.insert(idx, envelope);
            return;
        }
        // Non-matching or a different logical target: flush the old
        // pending NOW; the new Tap starts a fresh buffer.
        out.insert(idx, prev.envelope);
    }
    *pending = Some(PendingTap {
        envelope: tap,
        owner,
        up_ms,
        deadline_ms: up_ms.saturating_add(timeout_ms),
        timeout_ms,
        slop_px,
    });
}
