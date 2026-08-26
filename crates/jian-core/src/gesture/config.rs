//! Author-facing gesture configuration and handler declarations.
//!
//! The schema types are per-`PenNode`-variant, so configuration is read
//! through the same JSON round-trip the dispatcher already uses. Nothing
//! here interprets `interactionOrder` — it is authoring presentation only
//! and never participates in runtime arbitration.
//!
//! `gestures.disabled` (a state expression) cannot be evaluated without the
//! state graph; the runtime evaluates it in the semantic-delivery path.
//! Static facts (handler presence, `disabledEvents`, thresholds) are read
//! here, where the router needs them.

use crate::document::{NodeKey, RuntimeDocument};
use jian_ops_schema::gestures::AxisLock;

pub const DEFAULT_DRAG_THRESHOLD_PX: f32 = 8.0;
pub const DEFAULT_LONG_PRESS_MS: u64 = 500;
pub const DEFAULT_DOUBLE_TAP_TIMEOUT_MS: u64 = 300;
pub const DEFAULT_DOUBLE_TAP_SLOP_PX: f32 = 16.0;
/// Shared default minimum travel distance for a Swipe to claim
/// (logical px, PROJECTED onto the judged axis — `|dx|` for a
/// horizontal swipe, `|dy|` for a vertical one). Swipe has no Flutter
/// analogue in the existing recognizer set; 48px is the round-number
/// "clear directional stroke" threshold and matches the schema
/// doc-comment default.
pub const DEFAULT_SWIPE_MIN_DISTANCE_PX: f32 = 48.0;
/// Shared default minimum velocity for a Swipe to claim (logical px/s,
/// the component on the judged axis, with the same sign as the judged
/// direction). Short slow drags must not register as directional flicks.
pub const DEFAULT_SWIPE_MIN_VELOCITY_PX_PER_SECOND: f32 = 320.0;

/// Runtime-relevant `gestures` configuration of one node.
#[derive(Debug, Clone, Copy, Default)]
pub struct GestureConfig {
    pub drag_threshold: Option<f32>,
    pub long_press_duration: Option<u64>,
    pub double_tap_timeout: Option<u64>,
    pub double_tap_slop: Option<f32>,
    pub swipe_min_distance: Option<f32>,
    pub swipe_min_velocity: Option<f32>,
    pub axis_lock: Option<AxisLock>,
}

impl GestureConfig {
    /// Effective drag threshold (`dragThreshold` or the shared default).
    pub fn effective_drag_threshold(&self) -> f32 {
        self.drag_threshold.unwrap_or(DEFAULT_DRAG_THRESHOLD_PX)
    }
    /// Effective long-press duration (`longPressDuration` or default).
    pub fn effective_long_press_duration(&self) -> u64 {
        self.long_press_duration.unwrap_or(DEFAULT_LONG_PRESS_MS)
    }
    /// Effective double-tap window (`doubleTapTimeout` or default).
    pub fn effective_double_tap_timeout(&self) -> u64 {
        self.double_tap_timeout
            .unwrap_or(DEFAULT_DOUBLE_TAP_TIMEOUT_MS)
    }
    /// Effective double-tap slop (`doubleTapSlop` or default).
    pub fn effective_double_tap_slop(&self) -> f32 {
        self.double_tap_slop.unwrap_or(DEFAULT_DOUBLE_TAP_SLOP_PX)
    }
    /// Effective swipe minimum distance (`swipeMinDistance` or default).
    pub fn effective_swipe_min_distance(&self) -> f32 {
        self.swipe_min_distance
            .unwrap_or(DEFAULT_SWIPE_MIN_DISTANCE_PX)
    }
    /// Effective swipe minimum velocity (`swipeMinVelocity` or default).
    pub fn effective_swipe_min_velocity(&self) -> f32 {
        self.swipe_min_velocity
            .unwrap_or(DEFAULT_SWIPE_MIN_VELOCITY_PX_PER_SECOND)
    }
    /// Effective axis lock (`axisLock` or the `Auto` default).
    pub fn effective_axis_lock(&self) -> AxisLock {
        self.axis_lock.unwrap_or(AxisLock::Auto)
    }
}

/// Read `node.gestures` (the `gestures` object) or `None` when absent.
pub fn node_gestures(doc: &RuntimeDocument, key: NodeKey) -> Option<serde_json::Value> {
    let data = doc.tree.nodes.get(key)?;
    let v = serde_json::to_value(&data.schema).ok()?;
    v.as_object()?.get("gestures")?.clone().into()
}

/// Read the configured gesture overrides of `key`.
pub fn gesture_config(doc: &RuntimeDocument, key: NodeKey) -> GestureConfig {
    let Some(g) = node_gestures(doc, key) else {
        return GestureConfig::default();
    };
    let Some(g) = g.as_object() else {
        return GestureConfig::default();
    };
    GestureConfig {
        drag_threshold: g
            .get("dragThreshold")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32),
        long_press_duration: g
            .get("longPressDuration")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                g.get("longPressDuration")
                    .and_then(|v| v.as_f64())
                    .map(|v| v as u64)
            }),
        double_tap_timeout: g
            .get("doubleTapTimeout")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                g.get("doubleTapTimeout")
                    .and_then(|v| v.as_f64())
                    .map(|v| v as u64)
            }),
        double_tap_slop: g
            .get("doubleTapSlop")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32),
        swipe_min_distance: g
            .get("swipeMinDistance")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32),
        swipe_min_velocity: g
            .get("swipeMinVelocity")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32),
        axis_lock: g
            .get("axisLock")
            .and_then(|v| serde_json::from_value::<AxisLock>(v.clone()).ok()),
    }
}

/// The `gestures.disabled` expression source, if authored on `key`.
pub fn node_gesture_disabled_source(doc: &RuntimeDocument, key: NodeKey) -> Option<String> {
    let g = node_gestures(doc, key)?;
    g.as_object()?.get("disabled")?.as_str().map(str::to_owned)
}

/// Does `key` declare `handler` (any `events.<handler>` value)?
///
/// Mirrors the dispatcher's rule: an empty ActionList or a `null` value
/// declares nothing.
pub fn node_declares_handler(doc: &RuntimeDocument, key: NodeKey, handler: &str) -> bool {
    let Some(data) = doc.tree.nodes.get(key) else {
        return false;
    };
    let Ok(v) = serde_json::to_value(&data.schema) else {
        return false;
    };
    let Some(events) = v.as_object().and_then(|o| o.get("events")) else {
        return false;
    };
    let Some(events) = events.as_object() else {
        return false;
    };
    let Some(value) = events.get(handler) else {
        return false;
    };
    match value {
        serde_json::Value::Array(items) => !items.is_empty(),
        serde_json::Value::Null => false,
        _ => true,
    }
}

/// Does `key` list `handler` in `gestures.disabledEvents`?
pub fn node_disables_handler(doc: &RuntimeDocument, key: NodeKey, handler: &str) -> bool {
    let Some(g) = node_gestures(doc, key) else {
        return false;
    };
    g.as_object()
        .and_then(|g| g.get("disabledEvents"))
        .and_then(|list| list.as_array())
        .map(|list| list.iter().any(|v| v.as_str() == Some(handler)))
        .unwrap_or(false)
}

/// Walk the ancestor chain of `from` (inclusive) and report whether any
/// node declares `handler` and does not disable it via `disabledEvents`
/// or a truthy `gestures.disabled` expression (`node_disabled`).
/// This is the "handler exists" test used for recognizer installation
/// and Tap/DoubleTap deferral; it matches the bubbling of
/// `resolve_handler` in the delivery path.
pub fn chain_declares_enabled_with(
    doc: &RuntimeDocument,
    from: NodeKey,
    handler: &str,
    node_disabled: &dyn Fn(NodeKey) -> bool,
) -> bool {
    chain_owner_with(doc, from, handler, node_disabled).is_some()
}

/// Static-only variant: `node_disabled` is always `false` (the authoring
/// surface; the runtime pointer path uses the state-aware variant so a
/// dynamically disabled handler participates before arbitration).
pub fn chain_declares_enabled(doc: &RuntimeDocument, from: NodeKey, handler: &str) -> bool {
    chain_declares_enabled_with(doc, from, handler, &|_| false)
}

/// First node on the ancestor chain of `from` (inclusive) that declares an
/// enabled `handler` — the logical owner that bubbling targets. `None` when
/// the chain has no enabled declaration.
pub fn chain_owner_with(
    doc: &RuntimeDocument,
    from: NodeKey,
    handler: &str,
    node_disabled: &dyn Fn(NodeKey) -> bool,
) -> Option<NodeKey> {
    let mut node = Some(from);
    for _ in 0..=doc.tree.nodes.len() {
        let key = node?;
        if node_declares_handler(doc, key, handler)
            && !node_disables_handler(doc, key, handler)
            && !node_disabled(key)
        {
            return Some(key);
        }
        node = doc.tree.nodes.get(key).and_then(|n| n.parent);
    }
    None
}

/// Static-only variant of [`chain_owner_with`].
pub fn chain_owner(doc: &RuntimeDocument, from: NodeKey, handler: &str) -> Option<NodeKey> {
    chain_owner_with(doc, from, handler, &|_| false)
}

/// Handler keys that constitute a Pan gesture owner declaration.
pub const PAN_HANDLER_KEYS: [&str; 3] = ["onPanStart", "onPanUpdate", "onPanEnd"];

/// Nearest node on the ancestor chain of `from` (inclusive) that owns ANY
/// enabled nonempty Pan handler.
///
/// Handlers are scanned per node, not per handler name: a child owning
/// only `onPanUpdate` is nearer the hit point than a parent owning
/// `onPanStart`, and wins as the Pan owner — its authored threshold
/// governs the recognizer and its node is the semantic target (delivery
/// still bubbles each phase to whatever handler the chain declares).
/// A node whose pan handlers are all statically slated
/// (`disabledEvents`) or whose `gestures.disabled` expression evaluates
/// truthy (`node_disabled`) owns nothing and is skipped toward the
/// ancestors.
pub fn chain_pan_owner_with(
    doc: &RuntimeDocument,
    from: NodeKey,
    node_disabled: &dyn Fn(NodeKey) -> bool,
) -> Option<NodeKey> {
    let mut node = Some(from);
    for _ in 0..=doc.tree.nodes.len() {
        let key = node?;
        let owns = PAN_HANDLER_KEYS.iter().any(|handler| {
            node_declares_handler(doc, key, handler)
                && !node_disables_handler(doc, key, handler)
                && !node_disabled(key)
        });
        if owns {
            return Some(key);
        }
        node = doc.tree.nodes.get(key).and_then(|n| n.parent);
    }
    None
}

/// Static-only variant of [`chain_pan_owner_with`].
pub fn chain_pan_owner(doc: &RuntimeDocument, from: NodeKey) -> Option<NodeKey> {
    chain_pan_owner_with(doc, from, &|_| false)
}

/// Handler keys that constitute a Swipe gesture owner declaration.
/// The handler name is singular — a swipe is a discrete one-shot event,
/// not a start/update/end trio like Pan.
pub const SWIPE_HANDLER_KEYS: [&str; 1] = ["onSwipe"];

/// Nearest node on the ancestor chain of `from` (inclusive) that owns an
/// enabled nonempty `onSwipe` handler. Same skip rules as
/// [`chain_pan_owner_with`] (empty/null declarations, `disabledEvents`
/// and `gestures.disabled` do not count) — the nearest enabled owner
/// provides the Swipe recognizer's configuration and semantic node.
pub fn chain_swipe_owner_with(
    doc: &RuntimeDocument,
    from: NodeKey,
    node_disabled: &dyn Fn(NodeKey) -> bool,
) -> Option<NodeKey> {
    let mut node = Some(from);
    for _ in 0..=doc.tree.nodes.len() {
        let key = node?;
        let owns = SWIPE_HANDLER_KEYS.iter().any(|handler| {
            node_declares_handler(doc, key, handler)
                && !node_disables_handler(doc, key, handler)
                && !node_disabled(key)
        });
        if owns {
            return Some(key);
        }
        node = doc.tree.nodes.get(key).and_then(|n| n.parent);
    }
    None
}

/// Static-only variant of [`chain_swipe_owner_with`].
pub fn chain_swipe_owner(doc: &RuntimeDocument, from: NodeKey) -> Option<NodeKey> {
    chain_swipe_owner_with(doc, from, &|_| false)
}
