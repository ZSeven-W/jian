//! SwipeRecognizer — a discrete directional flick.
//!
//! Claims on the first `Move` whose **travel along the judged primary
//! axis** reaches `swipeMinDistance` while the **factual segment
//! velocity component on that SAME axis** (this Move vs. the previous
//! sample, over that segment's own time) reaches `swipeMinVelocity`
//! with the SAME SIGN as the judged direction. The direction, the
//! distance gate and the velocity gate all share ONE axis, so a fast
//! perpendicular final segment can never satisfy a horizontal swipe:
//! direction, distance and velocity are judged as a single directional
//! stroke, not three independent facts.
//!
//! Both gates must pass on the same Move; a segment with no measurable
//! time (`dt == 0`, e.g. a `t_ms = 0` sequence) has no velocity fact
//! and never claims — timestamps and velocities are never invented.
//!
//! # Primary axis & direction
//!
//! Judged from the total displacement vector from the initiating Down.
//! With `Auto`, the dominant axis is the one whose |component| is
//! larger (a tie resolves horizontal); an `axisLock` fixes the axis
//! instead. `Left`/`Right` for horizontal, `Up`/`Down` for vertical
//! (the y axis points down, so `dy < 0` is `Up`).
//!
//! # Distance gate (projected travel)
//!
//! The gate is the PROJECTED component of the total displacement on the
//! judged axis (`|dx|` for horizontal, `|dy|` for vertical) — NOT the
//! Euclidean vector length. That is also the value reported as
//! `$event.distance` (see `semantic.rs`), so the payload number is the
//! gated quantity and is consistent with `$event.direction`.
//!
//! # Velocity gate (same-axis, same-sign component)
//!
//! The triggering segment's velocity component on the judged axis must
//! have the SAME SIGN as the chosen direction and an absolute value
//! `>= swipeMinVelocity` (inclusive). The full factual velocity vector
//! (both components) is still reported as `$event.velocity`; only the
//! GATE looks at the component. A pure-perpendicular fast segment has a
//! zero component on the judged axis and can never claim.
//!
//! # axisLock
//!
//! - `Auto` — no constraint; the dominant axis decides.
//! - `Horizontal` — only horizontal-primary movement may claim; the
//!   first Move whose total displacement reaches `swipeMinDistance`
//!   (Euclidean) AND is vertical-dominant rejects the recognizer, so a
//!   wrong-axis sequence never emits Swipe (and cannot claim later).
//!   Sub-threshold jitter in either direction never rejects.
//! - `Vertical` — the mirror image.
//!
//! # Lifecycle
//!
//! The claim-time `Swipe` is emitted from `Self::accept` so the arena
//! rejects losers first — a Press cancellation (if any) precedes the
//! Swipe. `Up`/`Cancel` before the claim reject; after a claim no
//! second Swipe is ever emitted for the sequence.

use crate::document::NodeKey;
use crate::geometry::{point, Point};
use crate::gesture::config::{
    DEFAULT_SWIPE_MIN_DISTANCE_PX, DEFAULT_SWIPE_MIN_VELOCITY_PX_PER_SECOND,
};
use crate::gesture::pointer::{MouseButtons, PointerEvent, PointerPhase};
use crate::gesture::recognizer::{ArenaHandle, Recognizer, RecognizerId, RecognizerState};
use crate::gesture::semantic::{
    GestureFacts, PointerFacts, SemanticEvent, SemanticEventEnvelope, SwipeDirection,
};
use jian_ops_schema::gestures::AxisLock;

pub struct SwipeRecognizer {
    id: RecognizerId,
    node: NodeKey,
    state: RecognizerState,
    /// Down position/time — the swipe's origin and the total-displacement
    /// reference.
    start: Option<(Point, u64)>,
    /// Last observed sample (position/time) — the per-segment velocity
    /// reference.
    last: Option<(Point, u64)>,
    /// The initiating Down's provable single button — retained on the
    /// Swipe envelope while phase/position/timestamp/buttons stay from
    /// the triggering Move. `None` when the Down was button-less or
    /// ambiguous (keeps the key absent).
    down_button: Option<MouseButtons>,
    min_distance: f32,
    min_velocity: f32,
    axis_lock: AxisLock,
    claimed: bool,
    /// Claim-time Swipe, emitted from `accept` AFTER losers were rejected
    /// (so a Press cancellation precedes the Swipe).
    pending_claim: Option<SemanticEventEnvelope>,
}

impl SwipeRecognizer {
    pub fn new(id: RecognizerId, node: NodeKey) -> Self {
        Self {
            id,
            node,
            state: RecognizerState::Possible,
            start: None,
            last: None,
            down_button: None,
            min_distance: DEFAULT_SWIPE_MIN_DISTANCE_PX,
            min_velocity: DEFAULT_SWIPE_MIN_VELOCITY_PX_PER_SECOND,
            axis_lock: AxisLock::Auto,
            claimed: false,
            pending_claim: None,
        }
    }

    pub fn with_min_distance(mut self, px: f32) -> Self {
        self.min_distance = px;
        self
    }

    pub fn with_min_velocity(mut self, px_per_s: f32) -> Self {
        self.min_velocity = px_per_s;
        self
    }

    pub fn with_axis_lock(mut self, lock: AxisLock) -> Self {
        self.axis_lock = lock;
        self
    }
}

impl Recognizer for SwipeRecognizer {
    fn id(&self) -> RecognizerId {
        self.id
    }
    fn kind(&self) -> &'static str {
        "Swipe"
    }
    fn node(&self) -> NodeKey {
        self.node
    }
    fn state(&self) -> RecognizerState {
        self.state
    }

    fn handle_pointer(
        &mut self,
        event: &PointerEvent,
        _arena: &mut ArenaHandle<'_>,
    ) -> RecognizerState {
        match event.phase {
            PointerPhase::Down => {
                self.start = Some((event.position, event.t_ms));
                self.last = self.start;
                self.down_button = PointerFacts::from_event(event).button;
                self.state = RecognizerState::Possible;
                self.claimed = false;
                self.pending_claim = None;
            }
            PointerPhase::Move => {
                // A rejected (wrong-axis) recognizer never revives; a
                // claimed one stays silent (one Swipe per sequence).
                if matches!(self.state, RecognizerState::Rejected) || self.claimed {
                    return self.state;
                }
                let (start_pos, _) = match self.start {
                    Some(s) => s,
                    None => return self.state,
                };
                let dx = event.position.x - start_pos.x;
                let dy = event.position.y - start_pos.y;
                let total = (dx * dx + dy * dy).sqrt();
                // The PRIMARY axis is judged from the TOTAL displacement:
                // the lock's axis when one is set, the dominant axis
                // otherwise (|dx| >= |dy|; a 45° tie resolves horizontal,
                // deterministically). Direction, distance and velocity
                // all share this axis.
                let horizontal = match self.axis_lock {
                    AxisLock::Horizontal => true,
                    AxisLock::Vertical => false,
                    AxisLock::Auto => dx.abs() >= dy.abs(),
                };
                // Wrong-axis rejection: only once the total displacement
                // is meaningful (Euclidean >= min_distance) — a 2px
                // vertical jitter on the way to a horizontal stroke must
                // not kill the gesture, but a decisive cross-axis
                // displacement permanently rejects the sequence.
                if total >= self.min_distance {
                    let dominant_horizontal = dx.abs() >= dy.abs();
                    let wrong = match self.axis_lock {
                        AxisLock::Horizontal => !dominant_horizontal,
                        AxisLock::Vertical => dominant_horizontal,
                        AxisLock::Auto => false,
                    };
                    if wrong {
                        self.state = RecognizerState::Rejected;
                        return self.state;
                    }
                }
                // Distance gate: the PROJECTED travel on the primary axis
                // (not the Euclidean length). A horizontal-locked stroke
                // with a large vertical jog still has to move 48px along
                // x; a vertical-locked one 48px along y.
                let travel = if horizontal { dx.abs() } else { dy.abs() };
                if travel >= self.min_distance {
                    let direction = direction_for(horizontal, dx, dy);
                    // Factual segment velocity: this Move vs. the
                    // previous sample over its own time. `dt == 0` →
                    // no fact → no claim (never a fabricated value).
                    let (last_pos, last_t) = match self.last {
                        Some(l) => l,
                        None => return self.state,
                    };
                    let delta = point(event.position.x - last_pos.x, event.position.y - last_pos.y);
                    let dt = event.t_ms.saturating_sub(last_t) as f32 / 1000.0;
                    if let Some(velocity) = (dt > 0.0).then(|| point(delta.x / dt, delta.y / dt)) {
                        // Velocity gate: the component on the JUDGED axis
                        // must have the SAME SIGN as the direction and
                        // |component| >= min_velocity (inclusive). The
                        // full vector still rides the payload.
                        let comp = if horizontal { velocity.x } else { velocity.y };
                        let speed_on_axis = comp.abs();
                        let same_sign = match direction {
                            SwipeDirection::Left | SwipeDirection::Up => comp < 0.0,
                            SwipeDirection::Right | SwipeDirection::Down => comp > 0.0,
                        };
                        if same_sign && speed_on_axis >= self.min_velocity {
                            let facts = PointerFacts::from_event(event)
                                .with_initiating_button(self.down_button);
                            let gesture = GestureFacts {
                                swipe_direction: Some(direction.as_str().to_owned()),
                                swipe_distance: Some(travel),
                                swipe_velocity: Some(velocity),
                                ..Default::default()
                            };
                            self.pending_claim = Some(SemanticEventEnvelope {
                                event: SemanticEvent::Swipe {
                                    node: self.node,
                                    direction,
                                    distance: travel,
                                    velocity,
                                },
                                pointer_facts: Some(facts),
                                gesture,
                            });
                            self.state = RecognizerState::Claimed;
                            self.claimed = true;
                            return self.state;
                        }
                    }
                }
                // Whether or not this Move claimed, it becomes the next
                // segment's reference sample.
                self.last = Some((event.position, event.t_ms));
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                // Release/cancel before the claim: the sequence is not a
                // swipe. A claim-time Up sees `claimed == true` and is a
                // no-op — no duplicate Swipe, no end event.
                if !self.claimed {
                    self.state = RecognizerState::Rejected;
                    self.pending_claim = None;
                }
            }
            PointerPhase::Hover => {}
        }
        self.state
    }

    fn accept(&mut self, arena: &mut ArenaHandle<'_>) {
        self.state = RecognizerState::Claimed;
        if let Some(claim) = self.pending_claim.take() {
            *arena.pending_semantic = Some(claim);
        }
    }

    fn reject(&mut self) {
        self.state = RecognizerState::Rejected;
        self.pending_claim = None;
    }

    fn refresh_node_disabled(&mut self, node_disabled: &dyn Fn(NodeKey) -> bool) {
        // The session CAPTURED its owner node + thresholds at Down. If a
        // dynamic `gestures.disabled` flip invalidates that owner before
        // the claim, cancel the session outright: delivery must never
        // skip the (now-disabled) child and run the parent's handler with
        // the child's lower thresholds. A fresh Down re-resolves the
        // nearest ENABLED owner normally. Once claimed (or rejected) the
        // session is closed — one-shot Swipe, nothing to cancel.
        if matches!(self.state, RecognizerState::Possible) && node_disabled(self.node) {
            self.reject();
        }
    }
}

/// Direction from the total displacement vector along the pre-judged
/// primary axis. The y axis points down, so `dy < 0` is `Up`.
fn direction_for(horizontal: bool, dx: f32, dy: f32) -> SwipeDirection {
    if horizontal {
        if dx < 0.0 {
            SwipeDirection::Left
        } else {
            // `dx == 0` can only co-occur with `dy == 0` (the horizontal
            // dominance test requires |dx| >= |dy|), and a zero-length
            // displacement never reaches `min_distance`.
            SwipeDirection::Right
        }
    } else if dy < 0.0 {
        SwipeDirection::Up
    } else {
        SwipeDirection::Down
    }
}
