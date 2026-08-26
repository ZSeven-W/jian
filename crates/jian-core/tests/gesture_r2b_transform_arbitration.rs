//! R2B2 — deterministic Scale/Rotate multi-touch arbitration contracts.
//!
//! Five invariants, all engine-tested here (client-side completion waits
//! for R4's per-pointer-id Preview capture — do not read this file as a
//! client parity claim):
//!
//! 1. FIXED ORDER: when both transforms participate, evaluation AND
//!    emission follow `Scale → Rotate`, never HashMap iteration order;
//!    the authored `interactionOrder` stays presentation-only.
//! 2. CO-WIN: Scale and Rotate belong to one shared multi-team — both may
//!    win together on the same two fingers instead of excluding each
//!    other.
//! 3. PREFLIGHT → ONE-SHOT CAPTURE: fresh claims are checked against
//!    pristine arenas (too-late rejects cancel NOTHING), winners share a
//!    single capture pass over their unioned participants, so no partial
//!    cancel sequences can leak onto the wire.
//! 4. 2→1→2 REPAIR: lifting one finger ends the session symmetrically
//!    (one Start counted one End); regaining the quorum samples a FRESH
//!    baseline and must pass through a new Possible→Claimed edge before
//!    any further Updates — no stale-delta updates without a Start.
//! 5. THIRD-FINGER INDEPENDENCE: pointers beyond the two-finger quorum
//!    never join a transform team; their own press/tap ladder keeps
//!    running and their Up can never fire someone else's End.

use jian_core::geometry::{point, Point};
use jian_core::gesture::{MouseButtons, PointerEvent, PointerId, PointerKind, PointerPhase};
use jian_core::Runtime;

fn runtime_with<S: AsRef<str>>(op: S) -> Runtime {
    let mut rt = Runtime::new();
    rt.load_str(op.as_ref()).unwrap();
    rt.build_layout((800.0, 600.0)).unwrap();
    rt.rebuild_spatial();
    rt
}

fn node_center(rt: &Runtime, id: &str) -> Point {
    let key = rt.document.as_ref().unwrap().tree.get(id).expect(id);
    let rect = rt.layout.node_rect(key).unwrap();
    point(
        rect.min_x() + rect.size.width / 2.0,
        rect.min_y() + rect.size.height / 2.0,
    )
}

fn names(evs: &[jian_core::gesture::SemanticEvent]) -> Vec<&'static str> {
    evs.iter().map(|e| e.handler_key()).collect()
}

fn touch(id: u32, phase: PointerPhase, position: Point, t_ms: u64) -> PointerEvent {
    PointerEvent {
        id: PointerId(id),
        kind: PointerKind::Touch,
        phase,
        position,
        pressure: 1.0,
        buttons: MouseButtons::default(),
        modifiers: Default::default(),
        tilt: None,
        t_ms,
    }
}

/// One canvas-sized node declaring BOTH transform families plus a press
/// witness, so every capture surfaces its PressCancel count explicitly.
const TRANSFORM_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": {
    "pc": { "type": "int", "default": 0 },
    "ss": { "type": "int", "default": 0 },
    "su": { "type": "int", "default": 0 },
    "se": { "type": "int", "default": 0 },
    "rs": { "type": "int", "default": 0 },
    "ru": { "type": "int", "default": 0 },
    "re": { "type": "int", "default": 0 }
  },
  "children": [{
    "type": "rectangle", "id": "stage", "width": 800, "height": 600,
    "events": {
      "onPressCancel": [ { "set": { "$app.pc": "$app.pc + 1" } } ],
      "onScaleStart":   [ { "set": { "$app.ss": "$app.ss + 1" } } ],
      "onScaleUpdate":  [ { "set": { "$app.su": "$app.su + 1" } } ],
      "onScaleEnd":     [ { "set": { "$app.se": "$app.se + 1" } } ],
      "onRotateStart":  [ { "set": { "$app.rs": "$app.rs + 1" } } ],
      "onRotateUpdate": [ { "set": { "$app.ru": "$app.ru + 1" } } ],
      "onRotateEnd":    [ { "set": { "$app.re": "$app.re + 1" } } ]
    }
  }]
}"##;

/// Same stage but with an authrored `interactionOrder` that asks for
/// Rotate first — runtime behavior must ignore it (fixed Scale → Rotate).
const TRANSFORM_ORDER_OP: &str = r##"{
  "formatVersion": "1.0", "version": "1.0.0",
  "state": { "ss": { "type": "int", "default": 0 }, "rs": { "type": "int", "default": 0 } },
  "children": [{
    "type": "rectangle", "id": "stage", "width": 800, "height": 600,
    "gestures": { "interactionOrder": ["onRotate", "onScale"] },
    "events": {
      "onScaleStart":  [ { "set": { "$app.ss": "$app.ss + 1" } } ],
      "onRotateStart": [ { "set": { "$app.rs": "$app.rs + 1" } } ]
    }
  }]
}"##;

/// The pinch-and-twist stream used by several tests below: two fingers
/// land 100 logical px apart, then a simultaneous move pair crosses BOTH
/// activation thresholds in one batch (distance ≈161px ⇒ scale ≈1.61,
/// angle ≈ −29.7° ⇒ well past π/36).
struct TransformFixture {
    rt: Runtime,
}

impl TransformFixture {
    fn new(op: &str) -> Self {
        Self {
            rt: runtime_with(op),
        }
    }

    fn center(&self) -> Point {
        node_center(&self.rt, "stage")
    }

    /// Two Downs straddling `center` and the single crossing move pair.
    /// Returns every event drained across the four dispatches, in order.
    fn run_crossing_batch(&mut self, c: Point) -> Vec<&'static str> {
        let mut out = Vec::new();
        out.extend(names(&self.rt.dispatch_pointer(touch(
            0,
            PointerPhase::Down,
            point(c.x - 50.0, c.y),
            0,
        ))));
        out.extend(names(&self.rt.dispatch_pointer(touch(
            1,
            PointerPhase::Down,
            point(c.x + 50.0, c.y),
            10,
        ))));
        // Both thumbs in toward the vertical axis and apart vertically —
        // one batch crossing scale AND rotation together.
        out.extend(names(&self.rt.dispatch_pointer(touch(
            0,
            PointerPhase::Move,
            point(c.x - 60.0, c.y + 40.0),
            20,
        ))));
        out.extend(names(&self.rt.dispatch_pointer(touch(
            1,
            PointerPhase::Move,
            point(c.x + 60.0, c.y - 40.0),
            30,
        ))));
        // A settling move pair so every activation has definitely fired
        // and updates flow afterwards.
        out.extend(names(&self.rt.dispatch_pointer(touch(
            0,
            PointerPhase::Move,
            point(c.x - 70.0, c.y + 45.0),
            40,
        ))));
        out.extend(names(&self.rt.dispatch_pointer(touch(
            1,
            PointerPhase::Move,
            point(c.x + 70.0, c.y - 45.0),
            50,
        ))));
        out
    }
}

#[test]
fn scale_and_rotate_cowin_in_fixed_scale_rotate_order() {
    let mut fx = TransformFixture::new(TRANSFORM_OP);
    let c = fx.center();
    let evs = fx.run_crossing_batch(c);

    // Exactly two press cancels (one per participating pointer), emitted
    // once, BEFORE either claim event. No other captures interleaved.
    let cancels = evs.iter().filter(|n| **n == "onPressCancel").count();
    assert_eq!(cancels, 2, "one capture pass cancels each press once");
    let first_claim = evs.iter().position(|n| n.starts_with("onScale")).unwrap();
    let rotate_start = evs.iter().position(|n| *n == "onRotateStart").unwrap();
    assert!(
        evs[first_claim] == "onScaleStart",
        "first transform event must be ScaleStart, got {:?}",
        evs[first_claim]
    );
    assert!(
        first_claim < rotate_start,
        "ScaleStart must precede RotateStart (fixed order), got {evs:?}"
    );
    // The claims arrive back-to-back after the cancels — one team won
    // together, no exclusion round between them.
    let last_cancel = evs
        .iter()
        .rposition(|n| *n == "onPressCancel")
        .expect("cancels present");
    assert_eq!(evs[last_cancel + 1], "onScaleStart", "got {evs:?}");
    assert_eq!(evs[last_cancel + 2], "onRotateStart", "got {evs:?}");

    // Updates keep the fixed order too.
    let updates = names(&fx.rt.dispatch_pointer(touch(
        0,
        PointerPhase::Move,
        point(c.x - 74.0, c.y + 46.0),
        60,
    )));
    assert_eq!(
        updates,
        vec!["onScaleUpdate", "onRotateUpdate"],
        "single-pointer geometry refresh streams both teams in fixed order"
    );
}

#[test]
fn scale_and_rotate_end_symmetrically_on_first_lift() {
    let mut fx = TransformFixture::new(TRANSFORM_OP);
    let c = fx.center();
    let _ = fx.run_crossing_batch(c);

    // Lifting EITHER finger drops both teams below quorum: both sessions
    // end exactly once, Scale before Rotate.
    let up = names(&fx.rt.dispatch_pointer(touch(
        1,
        PointerPhase::Up,
        point(c.x + 70.0, c.y - 45.0),
        70,
    )));
    assert_eq!(up, vec!["onScaleEnd", "onRotateEnd"], "got {up:?}");
    // The second finger's Up is quiet — Sessions already closed.
    let up2 = names(&fx.rt.dispatch_pointer(touch(
        0,
        PointerPhase::Up,
        point(c.x - 70.0, c.y + 45.0),
        80,
    )));
    assert!(
        !up2.contains(&"onScaleEnd") && !up2.contains(&"onRotateEnd"),
        "no double Ends, got {up2:?}"
    );
    let rt = &fx.rt;
    assert_eq!(rt.state.app_get("se").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("re").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("ss").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("rs").unwrap().as_i64(), Some(1));
}

#[test]
fn two_one_two_regrab_opens_a_fresh_symmetric_session() {
    let mut fx = TransformFixture::new(TRANSFORM_OP);
    let c = fx.center();
    let _ = fx.run_crossing_batch(c);

    // Lift finger 1: session #1 closes symmetrically.
    let lift = names(&fx.rt.dispatch_pointer(touch(
        1,
        PointerPhase::Up,
        point(c.x + 70.0, c.y - 45.0),
        70,
    )));
    assert_eq!(lift, vec!["onScaleEnd", "onRotateEnd"]);
    // Single-finger wiggling is quiet — no quorum, no stale updates.
    for t in [90_u64, 100] {
        let wiggle = names(&fx.rt.dispatch_pointer(touch(
            0,
            PointerPhase::Move,
            point(c.x - 60.0, c.y + 10.0),
            t,
        )));
        assert!(
            wiggle.is_empty(),
            "one finger must not drive a claimed-but-quorumless transform"
        );
    }

    // Regain quorum at a totally different geometry: a FRESH baseline.
    let regain_down =
        names(
            &fx.rt
                .dispatch_pointer(touch(1, PointerPhase::Down, point(c.x + 100.0, c.y), 110)),
        );
    // The new pointer's own press ladder runs (the chain declares press
    // handlers); what must stay quiet are the TRANSFORMS until their
    // thresholds cross against the fresh baseline.
    assert!(
        regain_down
            .iter()
            .all(|n| !n.starts_with("onScale") && !n.starts_with("onRotate")),
        "transforms quiet until thresholds, got {regain_down:?}"
    );
    // Two moves cross scale first, then rotation, against the FRESH
    // baseline (slot0 far-left of the new finger): both families must
    // re-Start through a fresh Possible→Claimed edge.
    let crossing: Vec<Vec<&'static str>> = [
        (point(c.x + 160.0, c.y), 120_u64),
        (point(c.x - 130.0, c.y), 121),
    ]
    .iter()
    .map(|&(pos, t)| names(&fx.rt.dispatch_pointer(touch(0, PointerPhase::Move, pos, t))))
    .collect();
    let flat: Vec<&'static str> = crossing.concat();
    assert!(
        flat.contains(&"onScaleStart") && flat.contains(&"onRotateStart"),
        "re-grab must fire fresh Starts, got {flat:?}"
    );
    assert_eq!(
        flat.iter().filter(|n| n.ends_with("Start")).count(),
        2,
        "exactly one Start per family after re-grab"
    );

    // Finish symmetrically: 2 Starts / 2 Ends per family overall.
    let _ = fx
        .rt
        .dispatch_pointer(touch(1, PointerPhase::Up, point(c.x + 160.0, c.y), 130));
    let _ = fx
        .rt
        .dispatch_pointer(touch(0, PointerPhase::Up, point(c.x - 130.0, c.y), 131));
    let rt = &fx.rt;
    assert_eq!(rt.state.app_get("ss").unwrap().as_i64(), Some(2));
    assert_eq!(rt.state.app_get("se").unwrap().as_i64(), Some(2));
    assert_eq!(rt.state.app_get("rs").unwrap().as_i64(), Some(2));
    assert_eq!(rt.state.app_get("re").unwrap().as_i64(), Some(2));
}

#[test]
fn third_finger_stays_out_of_the_transform_team() {
    let mut fx = TransformFixture::new(TRANSFORM_OP);
    let c = fx.center();
    let _ = fx.run_crossing_batch(c);

    // A third pointer lands: it runs ITS OWN arena ladder but never
    // joins Scale/Rotate participation, and never perturbs the live
    // transform bookkeeping.
    let third_down =
        names(
            &fx.rt
                .dispatch_pointer(touch(2, PointerPhase::Down, point(c.x, c.y - 200.0), 60)),
        );
    assert!(
        third_down
            .iter()
            .all(|n| !n.starts_with("onScale") && !n.starts_with("onRotate")),
        "third finger must not emit or disturb transform events, got {third_down:?}"
    );
    // Its Up must be equally silent for the transforms.
    let third_up =
        names(
            &fx.rt
                .dispatch_pointer(touch(2, PointerPhase::Up, point(c.x, c.y - 200.0), 65)),
        );
    assert!(
        third_up
            .iter()
            .all(|n| !n.starts_with("onScale") && !n.starts_with("onRotate")),
        "a third finger's Up can never end someone else's gesture, got {third_up:?}"
    );

    // The original pair keeps streaming updates undisturbed, and the
    // final lifts close each family EXACTLY once.
    let after = names(&fx.rt.dispatch_pointer(touch(
        0,
        PointerPhase::Move,
        point(c.x - 76.0, c.y + 48.0),
        68,
    )));
    assert!(
        after.contains(&"onScaleUpdate"),
        "transform alive after third-finger churn"
    );
    let _ = fx.rt.dispatch_pointer(touch(
        1,
        PointerPhase::Up,
        point(c.x + 70.0, c.y - 45.0),
        70,
    ));
    let rt = &fx.rt;
    assert_eq!(rt.state.app_get("se").unwrap().as_i64(), Some(1));
    assert_eq!(rt.state.app_get("re").unwrap().as_i64(), Some(1));
}

#[test]
fn transform_output_is_identical_across_router_rebuilds() {
    // Distinct Runtime instances allocate distinct hash layouts; the
    // emission sequence must not care.
    let reference: Vec<&'static str> = {
        let mut fx = TransformFixture::new(TRANSFORM_OP);
        let c = fx.center();
        fx.run_crossing_batch(c)
    };
    for _ in 0..4 {
        let mut fx = TransformFixture::new(TRANSFORM_OP);
        let c = fx.center();
        let run = fx.run_crossing_batch(c);
        assert_eq!(run, reference, "HashMap layout must not affect output");
    }
}

#[test]
fn interaction_order_never_changes_runtime_scale_rotate_order() {
    let mut fx = TransformFixture::new(TRANSFORM_ORDER_OP);
    let c = fx.center();
    let evs = fx.run_crossing_batch(c);
    let s = evs.iter().position(|n| *n == "onScaleStart");
    let r = evs.iter().position(|n| *n == "onRotateStart");
    let (s, r) = (s.expect("ScaleStart fires"), r.expect("RotateStart fires"));
    assert!(
        s < r,
        "authored interactionOrder is presentation-only; runtime stays Scale → Rotate ({evs:?})"
    );
}
