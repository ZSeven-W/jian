//! SemanticEvent — the normalized output of the gesture pipeline.
//!
//! Consumed by `EventDispatcher` which maps each variant to the corresponding
//! schema `events.*` ActionList and executes it through Plan 4.
//!
//! # Envelope
//!
//! Recognizers never reconstruct facts after the fact: `SemanticEventEnvelope`
//! carries the `SemanticEvent` plus the factual `PointerFacts` captured from
//! the `PointerEvent` that produced it. One payload path ([`Self::payload`])
//! turns the envelope into the `$event` object fed to handlers through
//! `runtime/async_runtime.rs`. Missing facts are absent, never guessed.

use super::pointer::{Modifiers, MouseButtons, PointerEvent, PointerId, PointerKind, PointerPhase};
use crate::document::NodeKey;
use crate::geometry::Point;

#[derive(Debug, Clone, PartialEq)]
pub enum SemanticEvent {
    Tap {
        node: NodeKey,
        position: Point,
    },
    DoubleTap {
        node: NodeKey,
        position: Point,
    },
    LongPress {
        node: NodeKey,
        position: Point,
        duration_ms: u32,
    },
    PanStart {
        node: NodeKey,
        position: Point,
    },
    PanUpdate {
        node: NodeKey,
        delta: Point,
        velocity: Point,
    },
    PanEnd {
        node: NodeKey,
        velocity: Point,
    },
    ScaleStart {
        node: NodeKey,
        focal: Point,
    },
    ScaleUpdate {
        node: NodeKey,
        scale: f32,
        focal: Point,
    },
    ScaleEnd {
        node: NodeKey,
    },
    RotateStart {
        node: NodeKey,
    },
    RotateUpdate {
        node: NodeKey,
        radians: f32,
    },
    RotateEnd {
        node: NodeKey,
    },
    PressStart {
        node: NodeKey,
        position: Point,
    },
    PressEnd {
        node: NodeKey,
        position: Point,
    },
    PressCancel {
        node: NodeKey,
        position: Point,
    },
    ContextMenu {
        node: NodeKey,
        position: Point,
    },
    Scroll {
        node: NodeKey,
        delta: Point,
    },
    HoverEnter {
        node: NodeKey,
        position: Point,
    },
    HoverLeave {
        node: NodeKey,
        position: Point,
    },
    KeyDown {
        node: NodeKey,
        key: String,
        modifiers: Modifiers,
    },
    /// Raw escape-hatch — delivered when an ancestor sets `gestures.rawPointer`.
    RawPointer {
        node: NodeKey,
        phase: super::pointer::PointerPhase,
        position: Point,
    },
    /// Tab-tree focus moved onto `node`. Fires after any `FocusLost`
    /// for the previously-focused node so authored handlers can rely
    /// on the documented blur-then-focus ordering.
    FocusGained {
        node: NodeKey,
    },
    /// Tab-tree focus moved off `node`.
    FocusLost {
        node: NodeKey,
    },
}

impl SemanticEvent {
    /// Return the target node this event routes to.
    pub fn node(&self) -> NodeKey {
        match self {
            Self::Tap { node, .. }
            | Self::DoubleTap { node, .. }
            | Self::LongPress { node, .. }
            | Self::PanStart { node, .. }
            | Self::PanUpdate { node, .. }
            | Self::PanEnd { node, .. }
            | Self::ScaleStart { node, .. }
            | Self::ScaleUpdate { node, .. }
            | Self::ScaleEnd { node }
            | Self::RotateStart { node }
            | Self::RotateUpdate { node, .. }
            | Self::RotateEnd { node }
            | Self::PressStart { node, .. }
            | Self::PressEnd { node, .. }
            | Self::PressCancel { node, .. }
            | Self::ContextMenu { node, .. }
            | Self::Scroll { node, .. }
            | Self::HoverEnter { node, .. }
            | Self::HoverLeave { node, .. }
            | Self::KeyDown { node, .. }
            | Self::RawPointer { node, .. }
            | Self::FocusGained { node }
            | Self::FocusLost { node } => *node,
        }
    }

    /// Return the matching `events.*` handler name used by the schema
    /// (Plan 1 `EventHandlers` field naming, snake_case after serde).
    pub fn handler_key(&self) -> &'static str {
        match self {
            Self::Tap { .. } => "onTap",
            Self::DoubleTap { .. } => "onDoubleTap",
            Self::LongPress { .. } => "onLongPress",
            Self::PanStart { .. } => "onPanStart",
            Self::PanUpdate { .. } => "onPanUpdate",
            Self::PanEnd { .. } => "onPanEnd",
            Self::ScaleStart { .. } => "onScaleStart",
            Self::ScaleUpdate { .. } => "onScaleUpdate",
            Self::ScaleEnd { .. } => "onScaleEnd",
            Self::RotateStart { .. } => "onRotateStart",
            Self::RotateUpdate { .. } => "onRotateUpdate",
            Self::RotateEnd { .. } => "onRotateEnd",
            Self::PressStart { .. } => "onPressStart",
            Self::PressEnd { .. } => "onPressEnd",
            Self::PressCancel { .. } => "onPressCancel",
            Self::ContextMenu { .. } => "onContextMenu",
            Self::Scroll { .. } => "onScroll",
            Self::HoverEnter { .. } => "onHoverEnter",
            Self::HoverLeave { .. } => "onHoverLeave",
            Self::KeyDown { .. } => "onKey",
            Self::RawPointer { .. } => "onRawPointer",
            Self::FocusGained { .. } => "onFocus",
            Self::FocusLost { .. } => "onBlur",
        }
    }
}

/// Normalized, factual pointer metadata attached to a semantic event.
///
/// Built once in `PointerFacts::from_event` and carried unchanged through
/// the recognizer into the envelope — downstream code never reconstructs
/// pointer facts from something else.
///
/// `PointerFacts` holds **host-reported** facts only. The node-local
/// coordinate (`$event.local`) is NOT a fact: it is derived per handler
/// during payload construction ([`SemanticEventEnvelope::payload`]),
/// relative to the resolved handler owner's layout rect, and therefore
/// lives exclusively on the `$event` payload path.
#[derive(Debug, Clone, PartialEq)]
pub struct PointerFacts {
    pub id: PointerId,
    pub kind: PointerKind,
    pub phase: PointerPhase,
    /// Global (viewport) coordinates as reported by the host.
    pub position: Point,
    /// Pressure only for contact kinds (touch/pen/stylus). Mouse and
    /// trackpad hosts cannot prove pressure, so it is absent.
    pub pressure: Option<f32>,
    /// Bitmask of buttons held at this event, as reported. Absent when the
    /// host reported none (`buttons = 0`).
    pub buttons: Option<MouseButtons>,
    /// The single button whose transition caused the Down, when provable:
    /// a `Down` whose bitmask has exactly one bit set. Any other case
    /// (multi-button Down, Up, Move, Cancel) is absent — never guessed.
    ///
    /// Recognizers of a continuous gesture retain the initiating Down's
    /// provable button across the sequence: a Tap emitted on `Up` still
    /// reports `button` (from the Down) while `phase`/`position`/
    /// `timestamp`/`buttons` stay those of the triggering `Up`. See
    /// [`PointerFacts::with_initiating_button`] in
    /// `gesture::recognizer`.
    pub button: Option<MouseButtons>,
    /// Modifier keys held at this event. Hosts always report modifier
    /// state; empty means explicitly none held.
    pub modifiers: Modifiers,
    /// Pen tilt in degrees, when the host reported it.
    pub tilt: Option<(f32, f32)>,
    /// Monotonic host timestamp in milliseconds.
    pub t_ms: u64,
}

impl PointerFacts {
    /// Capture the factual pointer metadata of `event`.
    pub fn from_event(event: &PointerEvent) -> Self {
        // Provable changed-button rule: only a Down with exactly one
        // button bit proves which button was pressed. Multi-button Downs,
        // Moves (no transition), and Ups (the released button is absent
        // from `buttons`) do not.
        let button = (event.phase == PointerPhase::Down)
            .then(|| single_button(event.buttons))
            .flatten();
        let pressure = (!matches!(event.kind, PointerKind::Mouse | PointerKind::Trackpad))
            .then_some(event.pressure);
        let buttons = (!event.buttons.is_empty()).then_some(event.buttons);
        Self {
            id: event.id,
            kind: event.kind,
            phase: event.phase,
            position: event.position,
            pressure,
            buttons,
            button,
            modifiers: event.modifiers,
            tilt: event.tilt,
            t_ms: event.t_ms,
        }
    }
}

/// Return the sole set bit as a button, or `None` when the bitmask has
/// zero or several bits (the changed button is not provable).
fn single_button(buttons: MouseButtons) -> Option<MouseButtons> {
    let mut set = Vec::new();
    for flag in [
        MouseButtons::LEFT,
        MouseButtons::RIGHT,
        MouseButtons::MIDDLE,
        MouseButtons::BACK,
        MouseButtons::FORWARD,
    ] {
        if buttons.contains(flag) {
            set.push(flag);
        }
    }
    (set.len() == 1).then(|| set[0])
}

/// Gesture-specific numeric/point facts mirrored onto the envelope.
///
/// The `SemanticEvent` variants keep their existing fields (source
/// compatible), while the envelope carries the complete factual set the
/// design §5.3 demands — e.g. Pan gets start/current/delta/translation/
/// velocity instead of only `delta`/`velocity`, and Scale/Rotate get
/// absolute + per-frame delta values alongside their existing fields.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GestureFacts {
    /// LongPress: authored/effective duration.
    pub duration_ms: Option<u32>,
    /// Pan: position of the pointer Down that started the gesture.
    pub pan_start: Option<Point>,
    /// Pan: position of the pointer at this event.
    pub pan_current: Option<Point>,
    /// Pan: movement since the previous event.
    pub pan_delta: Option<Point>,
    /// Pan: cumulative movement from `pan_start`.
    pub pan_translation: Option<Point>,
    /// Pan: velocity in logical px/s.
    pub pan_velocity: Option<Point>,
    /// Scale: absolute scale ratio vs. the gesture's initial baseline.
    pub scale: Option<f32>,
    /// Scale: per-frame absolute difference from the previous event.
    pub delta_scale: Option<f32>,
    /// Rotate: absolute rotation (radians) vs. the initial angle.
    pub rotation: Option<f32>,
    /// Rotate: per-frame absolute difference from the previous event.
    pub delta_rotation: Option<f32>,
    /// Scale/Rotate: focal point (midpoint of the two active pointers).
    pub focal: Option<Point>,
}

/// A `SemanticEvent` plus the factual pointer/gesture metadata captured
/// at recognition time. The runtime's semantic-delivery path turns it
/// into `$event` via [`Self::payload`].
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticEventEnvelope {
    pub event: SemanticEvent,
    /// Pointer facts for pointer-originated events. `None` for
    /// non-pointer events (key/scroll/focus) — their payloads carry no
    /// pointer fields (later tasks expand them).
    pub pointer_facts: Option<PointerFacts>,
    /// Gesture facts (pan/scale/rotate/long-press) — all `None` for
    /// events that are not gesture updates.
    pub gesture: GestureFacts,
}

impl SemanticEventEnvelope {
    /// Wrap a non-pointer semantic event (key/scroll/focus) with no
    /// pointer facts.
    pub fn plain(event: SemanticEvent) -> Self {
        Self {
            event,
            pointer_facts: None,
            gesture: GestureFacts::default(),
        }
    }

    /// Build the `$event` payload for the ActionContext.
    ///
    /// `local_origin` is the handler node's layout-rect origin, resolved
    /// by the runtime from its `LayoutEngine`; the node-local coordinate
    /// is `global - origin` (possibly outside the rect — that is factual
    /// geometry, not an error). `None` omits `local` (absent, not
    /// fabricated). This is the ONE place `local` is computed: the facts
    /// themselves never carry it.
    ///
    /// Returns `None` for events whose payload expansion belongs to later
    /// tasks (Scroll/Focus keep their previous absent payloads — their
    /// pointer facts are carried on the envelope, not guessed).
    pub fn payload(&self, local_origin: Option<Point>) -> Option<serde_json::Value> {
        match &self.event {
            SemanticEvent::KeyDown { key, modifiers, .. } => {
                return Some(serde_json::json!({
                    "key": key,
                    "modifiers": modifier_names(*modifiers),
                }));
            }
            SemanticEvent::Scroll { .. }
            | SemanticEvent::FocusGained { .. }
            | SemanticEvent::FocusLost { .. } => return None,
            _ => {}
        }
        let mut obj = serde_json::Map::new();
        if let Some(pointer) = &self.pointer_facts {
            obj.insert("pointerId".into(), serde_json::json!(pointer.id.0));
            obj.insert(
                "pointerType".into(),
                serde_json::json!(pointer_type(pointer.kind)),
            );
            obj.insert("phase".into(), serde_json::json!(phase_name(pointer.phase)));
            obj.insert("position".into(), point_json(pointer.position));
            if let Some(local) = local_origin {
                obj.insert(
                    "local".into(),
                    point_json(point(
                        pointer.position.x - local.x,
                        pointer.position.y - local.y,
                    )),
                );
            }
            if let Some(pressure) = pointer.pressure {
                obj.insert("pressure".into(), serde_json::json!(pressure));
            }
            if let Some(button) = pointer.button {
                obj.insert("button".into(), serde_json::json!(button_name(button)));
            }
            if let Some(buttons) = pointer.buttons {
                obj.insert("buttons".into(), serde_json::json!(button_names(buttons)));
            }
            obj.insert(
                "modifiers".into(),
                serde_json::json!(modifier_names(pointer.modifiers)),
            );
            if let Some((x, y)) = pointer.tilt {
                obj.insert(
                    "tilt".into(),
                    serde_json::json!({
                        "xDegrees": x,
                        "yDegrees": y,
                    }),
                );
            }
            obj.insert("timestamp".into(), serde_json::json!(pointer.t_ms));
        }
        let gesture = &self.gesture;
        match &self.event {
            SemanticEvent::Tap { .. }
            | SemanticEvent::DoubleTap { .. }
            | SemanticEvent::PressStart { .. }
            | SemanticEvent::PressEnd { .. }
            | SemanticEvent::PressCancel { .. }
            | SemanticEvent::ContextMenu { .. }
            | SemanticEvent::RawPointer { .. } => {}
            SemanticEvent::LongPress { .. } => {
                if let Some(duration_ms) = gesture.duration_ms {
                    obj.insert("durationMs".into(), serde_json::json!(duration_ms));
                }
            }
            SemanticEvent::PanStart { .. }
            | SemanticEvent::PanUpdate { .. }
            | SemanticEvent::PanEnd { .. } => {
                for (key, value) in [
                    ("start", gesture.pan_start),
                    ("current", gesture.pan_current),
                    ("delta", gesture.pan_delta),
                    ("translation", gesture.pan_translation),
                    ("velocity", gesture.pan_velocity),
                ] {
                    if let Some(value) = value {
                        obj.insert(key.into(), point_json(value));
                    }
                }
            }
            SemanticEvent::ScaleStart { .. }
            | SemanticEvent::ScaleUpdate { .. }
            | SemanticEvent::ScaleEnd { .. } => {
                if let Some(scale) = gesture.scale {
                    obj.insert("scale".into(), serde_json::json!(scale));
                }
                if let Some(delta_scale) = gesture.delta_scale {
                    obj.insert("deltaScale".into(), serde_json::json!(delta_scale));
                }
                if let Some(focal) = gesture.focal {
                    obj.insert("focal".into(), point_json(focal));
                }
            }
            SemanticEvent::RotateStart { .. }
            | SemanticEvent::RotateUpdate { .. }
            | SemanticEvent::RotateEnd { .. } => {
                // `radians` is the pre-existing key (source-compatible
                // payload; existing docs use `$event.radians`).
                if let SemanticEvent::RotateUpdate { radians, .. } = &self.event {
                    obj.insert("radians".into(), serde_json::json!(radians));
                }
                if let Some(rotation) = gesture.rotation {
                    obj.insert("rotation".into(), serde_json::json!(rotation));
                }
                if let Some(delta_rotation) = gesture.delta_rotation {
                    obj.insert("deltaRotation".into(), serde_json::json!(delta_rotation));
                }
                if let Some(focal) = gesture.focal {
                    obj.insert("focal".into(), point_json(focal));
                }
            }
            // Key/control/Scroll/lifecycle payload expansion belongs to
            // later tasks. Scroll/Focus keep their previous payload shape
            // (none) and are not broadened in this slice. `KeyDown`
            // returned above and never reaches this arm. Hover carries the
            // standard pointer facts from the block above but no gesture
            // facts.
            SemanticEvent::KeyDown { .. }
            | SemanticEvent::Scroll { .. }
            | SemanticEvent::HoverEnter { .. }
            | SemanticEvent::HoverLeave { .. }
            | SemanticEvent::FocusGained { .. }
            | SemanticEvent::FocusLost { .. } => {}
        }
        Some(serde_json::Value::Object(obj))
    }
}

fn point_json(p: Point) -> serde_json::Value {
    serde_json::json!({ "x": p.x, "y": p.y })
}

fn point(x: f32, y: f32) -> Point {
    crate::geometry::point(x, y)
}

fn pointer_type(kind: PointerKind) -> &'static str {
    match kind {
        PointerKind::Touch => "touch",
        PointerKind::Mouse => "mouse",
        PointerKind::Pen => "pen",
        PointerKind::Stylus => "stylus",
        PointerKind::Trackpad => "trackpad",
    }
}

fn phase_name(phase: PointerPhase) -> &'static str {
    match phase {
        PointerPhase::Down => "down",
        PointerPhase::Move => "move",
        PointerPhase::Up => "up",
        PointerPhase::Cancel => "cancel",
        PointerPhase::Hover => "hover",
    }
}

fn button_name(button: MouseButtons) -> &'static str {
    match button {
        MouseButtons::LEFT => "left",
        MouseButtons::RIGHT => "right",
        MouseButtons::MIDDLE => "middle",
        MouseButtons::BACK => "back",
        MouseButtons::FORWARD => "forward",
        _ => "unknown",
    }
}

fn button_names(buttons: MouseButtons) -> Vec<&'static str> {
    [
        (MouseButtons::LEFT, "left"),
        (MouseButtons::RIGHT, "right"),
        (MouseButtons::MIDDLE, "middle"),
        (MouseButtons::BACK, "back"),
        (MouseButtons::FORWARD, "forward"),
    ]
    .into_iter()
    .filter_map(|(flag, name)| buttons.contains(flag).then_some(name))
    .collect()
}

fn modifier_names(modifiers: Modifiers) -> Vec<&'static str> {
    [
        (Modifiers::SHIFT, "shift"),
        (Modifiers::CTRL, "ctrl"),
        (Modifiers::ALT, "alt"),
        (Modifiers::CMD, "cmd"),
    ]
    .into_iter()
    .filter_map(|(flag, name)| modifiers.contains(flag).then_some(name))
    .collect()
}
